// SPDX-License-Identifier: AGPL-3.0-only
//! The workbook: HyperFormula's engine state for `@oc/sheet`'s configuration — sheets of raw cell contents,
//! parsed formulas, array formulas spilling into neighbouring cells, a static dependency graph evaluated in
//! topological order with strongly connected components (cycles) reported as `#CYCLE!`, structural edits
//! that rewrite every formula, and HyperFormula's exported values (smart rounding, `precisionRounding: 10`).
//!
//! Every mutation leaves the workbook dirty; call [`Workbook::recompute`] before reading values (the
//! `SheetEngine` does this after each batch, like `hf.batch`).

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};

use super::array_size::{self, ArraySize};
use super::ast::{Ast, SheetRef};
use super::coerce;
use super::datetime::date_string_to_number;
use super::interp::{Env, Interp, State};
use super::parser::{self, MAX_COLUMNS, MAX_ROWS};
use super::transform::{self, Change};
use super::unparse;
use super::value::{AbsRange, ErrorType, NumType, RangeVal, Scalar, Val};
use crate::format::{CellValue, ValueKind};

pub type SheetId = u32;
/// (sheet, row, col)
pub type Addr = (SheetId, u32, u32);

/// Clock, timezone and randomness for TODAY/NOW/RAND (injected: the library never reads system time).
pub trait Host {
    /// Milliseconds since the Unix epoch (UTC).
    fn now_ms(&self) -> f64;
    /// `Date#getTimezoneOffset()` of the host (minutes, UTC − local).
    fn tz_offset_minutes(&self) -> f64 {
        0.0
    }
    /// Uniform in [0, 1).
    fn random(&self) -> f64;
}

/// A deterministic host: a fixed clock and a seeded mulberry32 generator.
#[derive(Debug)]
pub struct FixedHost {
    pub now_ms: f64,
    pub tz_offset_minutes: f64,
    state: Cell<u32>,
}

impl FixedHost {
    pub fn new(now_ms: f64, seed: u32) -> FixedHost {
        FixedHost { now_ms, tz_offset_minutes: 0.0, state: Cell::new(seed) }
    }
}

impl Default for FixedHost {
    fn default() -> Self {
        FixedHost::new(0.0, 1)
    }
}

impl Host for FixedHost {
    fn now_ms(&self) -> f64 {
        self.now_ms
    }
    fn tz_offset_minutes(&self) -> f64 {
        self.tz_offset_minutes
    }
    fn random(&self) -> f64 {
        let s = self.state.get().wrapping_add(0x6D2B_79F5);
        self.state.set(s);
        let mut t = (s ^ (s >> 15)).wrapping_mul(1 | s);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// The system clock and OS randomness (native builds only).
#[cfg(feature = "native")]
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemHost;

#[cfg(feature = "native")]
impl Host for SystemHost {
    fn now_ms(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
    }
    fn random(&self) -> f64 {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("OS random source");
        (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A parsed formula.
#[derive(Debug, Clone)]
pub struct Formula {
    pub ast: Ast,
    /// Predicted result size; not scalar = array formula.
    pub size: ArraySize,
    /// Placement of an array formula (scalar formulas stay [`ArrayState::Claimed`] and ignore it).
    pub state: ArrayState,
    /// Creation sequence: older array formulas keep their areas when claims are re-validated.
    pub created: u64,
}

/// Where an array formula stands — HyperFormula's `ArrayFormulaVertex` bookkeeping, which is stateful:
///
/// - a new array formula claims its whole area at once when that area is free (`exchangeOrAddFormulaVertex`);
///   otherwise it never claims it later;
/// - writing a value or formula into a claimed cell shrinks the array to its corner and leaves it `#SPILL!`
///   for good (`shrinkPossibleArrayAndGetCell` → `setNoSpace`, whose error size makes every later recompute
///   return `#SPILL!`), even after the blocker is cleared; only entering the formula again resets it;
/// - clearing a claimed cell is a no-op (`Operations.setCellEmpty` skips array-internal cells);
/// - an array formula evaluated without space becomes `#SPILL!` the same sticky way;
/// - a formula on a cycle is never checked for space: `#CYCLE!` over its area when it claimed one, else in
///   its anchor only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayState {
    /// The area is mapped to the formula (it spills there).
    Claimed,
    /// Created without space; has not yet been evaluated off a cycle.
    Unclaimed,
    /// `setNoSpace()`: `#SPILL!` until the formula is entered again.
    NoSpace,
}

/// Raw content of a cell after `CellContentParser`.
#[derive(Debug, Clone)]
pub enum Content {
    Value(Scalar),
    Formula(Box<Formula>),
    /// A formula that failed to parse (kept verbatim; evaluates to `#ERROR!`).
    ParseError(String),
}

#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub name: String,
    pub cells: HashMap<(u32, u32), Content>,
    /// `DenseStrategy` dimensions: grow with every cell the graph touches, shrink only on removals.
    pub height: u32,
    pub width: u32,
}

/// Computed result of a formula cell.
#[derive(Debug, Clone)]
enum Computed {
    S(Scalar),
    /// A scalar formula whose value is a range (`=A1:B2`): exports `#VALUE!`.
    R(RangeVal),
    /// Array formula values (resized to the predicted size).
    Array(Vec<Vec<Scalar>>),
    /// Errored array: every cell of the area shows the error.
    ArrayErr(ErrorType),
    /// No space to spill: `#SPILL!` in the anchor only.
    NoSpace,
}

/// A structural edit that cannot be applied (beyond the engine limits).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StructError {
    #[error("no such sheet")]
    NoSheet,
    #[error("the change exceeds the sheet limits")]
    Limits,
}

pub struct Workbook {
    sheets: Vec<Option<Sheet>>,
    /// Display names of referenced but missing sheets (lower-case key → first spelling seen).
    placeholders: HashMap<String, String>,
    values: HashMap<Addr, Computed>,
    /// Spilled cells → anchor of the array formula covering them.
    spills: HashMap<Addr, (u32, u32)>,
    /// Array formulas that claimed their area.
    spill_ok: HashSet<Addr>,
    /// Next [`Formula::created`].
    seq: u64,
    host: Box<dyn Host>,
}

impl std::fmt::Debug for Workbook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workbook").field("sheets", &self.sheets.len()).finish()
    }
}

/// `CellContentParser.parse` for a string input (the TS engine only feeds strings, `null` for empty).
pub fn parse_content(raw: &str) -> Option<RawContent> {
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_lowercase();
    if lower == "true" || lower == "false" {
        return Some(RawContent::Value(Scalar::Bool(lower == "true")));
    }
    if raw.starts_with('=') {
        return Some(RawContent::Formula(raw.to_string()));
    }
    let upper = raw.to_uppercase();
    if let Some(e) = ErrorType::from_literal(&upper)
        && looks_like_error(&upper)
    {
        return Some(RawContent::Value(Scalar::Err(e)));
    }
    let mut trimmed = oc_schema::js::trim(raw).to_string();
    let mut mode = 0;
    let mut currency = false;
    if let Some(t) = trimmed.strip_suffix('%') {
        mode = 1;
        trimmed = t.to_string();
    } else if let Some((_, rest)) = coerce::currency_match(&trimmed, false) {
        mode = 2;
        currency = true;
        trimmed = rest;
    }
    if let Some(v) = coerce::numeric_string_to_number(&trimmed) {
        let s = match mode {
            1 => Scalar::Num(fix_zero(v / 100.0), NumType::Percent),
            2 if currency => Scalar::Num(fix_zero(v), NumType::Currency),
            _ => Scalar::Num(fix_zero(v), NumType::Raw),
        };
        return Some(RawContent::Value(s));
    }
    if let Some(d) = date_string_to_number(&trimmed) {
        return Some(RawContent::Value(d));
    }
    Some(RawContent::Value(Scalar::str(raw.strip_prefix('\'').unwrap_or(raw))))
}

fn looks_like_error(upper: &str) -> bool {
    // `/#[A-Za-z0-9\/]+[?!]?/.test(...)` — any literal in the error mapping contains such a run.
    upper.contains('#')
}

fn fix_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// Parsed cell input before it is stored.
#[derive(Debug, Clone)]
pub enum RawContent {
    Value(Scalar),
    Formula(String),
}

/// A claimed spill area without its anchor: (r0, c0, r1, c1, anchor's node index).
type SpillArea = (u32, u32, u32, u32, usize);

/// A static dependency of a formula.
#[derive(Debug, Clone, Copy)]
enum Dep {
    Cell(SheetId, u32, u32),
    Range(AbsRange),
    Cols(SheetId, u32, u32),
    Rows(SheetId, u32, u32),
}

fn sheet_of(s: &Option<SheetRef>, fs: SheetId) -> Option<SheetId> {
    match s {
        None => Some(fs),
        Some(SheetRef::Id(id)) => Some(*id),
        Some(SheetRef::Missing(_)) => None,
    }
}

/// `collectDependencies`, absolutised.
fn collect_deps(ast: &Ast, need: bool, fs: SheetId, out: &mut Vec<Dep>) {
    match ast {
        Ast::Cell { r, .. } => {
            if need
                && !r.is_invalid()
                && let Some(s) = sheet_of(&r.sheet, fs)
            {
                out.push(Dep::Cell(s, r.row as u32, r.col as u32));
            }
        }
        Ast::Range { start, end, .. } => {
            if need
                && start.sheet == end.sheet
                && !start.is_invalid()
                && !end.is_invalid()
                && let Some(s) = sheet_of(&start.sheet, fs)
            {
                out.push(Dep::Range(AbsRange { sheet: s, r0: start.row as u32, c0: start.col as u32, r1: end.row as u32, c1: end.col as u32 }));
            }
        }
        Ast::ColRange { start, end, .. } => {
            if need
                && start.sheet == end.sheet
                && start.col >= 0
                && end.col >= 0
                && let Some(s) = sheet_of(&start.sheet, fs)
            {
                out.push(Dep::Cols(s, start.col as u32, end.col as u32));
            }
        }
        Ast::RowRange { start, end, .. } => {
            if need
                && start.sheet == end.sheet
                && start.row >= 0
                && end.row >= 0
                && let Some(s) = sheet_of(&start.sheet, fs)
            {
                out.push(Dep::Rows(s, start.row as u32, end.row as u32));
            }
        }
        Ast::Plus { v, .. } | Ast::Minus { v, .. } | Ast::Percent { v, .. } => collect_deps(v, true, fs, out),
        Ast::Bin { l, r, .. } => {
            collect_deps(l, true, fs, out);
            collect_deps(r, true, fs, out);
        }
        Ast::Paren { e, .. } => collect_deps(e, need, fs, out),
        Ast::Func { name, args, .. } => {
            let needs = !super::functions::meta_of(name).is_some_and(|m| m.no_arg_deps);
            for a in args {
                collect_deps(a, needs, fs, out);
            }
        }
        // Array literal elements are not visited by `collectDependencies`.
        _ => {}
    }
}

/// HyperFormula's `cellValueRounding` with the default `precisionRounding: 10` (11 significant digits).
pub fn smart_round(v: f64) -> f64 {
    if v == 0.0 {
        return v;
    }
    let mag = v.abs().log10().floor();
    let k = 10.0 - mag;
    let mult = if k.is_finite() && k.abs() <= 400.0 { format!("1e{}", k as i64).parse::<f64>().unwrap_or(f64::NAN) } else { 10f64.powf(k) };
    if v < 0.0 { -oc_schema::js::round(-v * mult) / mult } else { oc_schema::js::round(v * mult) / mult }
}

impl Workbook {
    pub fn new(host: Box<dyn Host>) -> Workbook {
        Workbook {
            sheets: Vec::new(),
            placeholders: HashMap::new(),
            values: HashMap::new(),
            spills: HashMap::new(),
            spill_ok: HashSet::new(),
            seq: 0,
            host,
        }
    }

    pub fn host(&self) -> &dyn Host {
        &*self.host
    }

    /// Take the host back (when rebuilding).
    pub fn into_host(self) -> Box<dyn Host> {
        self.host
    }

    pub fn set_host(&mut self, host: Box<dyn Host>) {
        self.host = host;
    }

    // ------------------------------------------------------------------ sheets

    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.get(id as usize).and_then(Option::as_ref)
    }

    fn sheet_mut(&mut self, id: SheetId) -> Option<&mut Sheet> {
        self.sheets.get_mut(id as usize).and_then(Option::as_mut)
    }

    /// Live sheet ids in id order.
    pub fn sheet_ids(&self) -> Vec<SheetId> {
        (0..self.sheets.len() as u32).filter(|i| self.sheet(*i).is_some()).collect()
    }

    pub fn sheet_id(&self, name: &str) -> Option<SheetId> {
        let l = name.to_lowercase();
        self.sheet_ids().into_iter().find(|i| self.sheet(*i).is_some_and(|s| s.name.to_lowercase() == l))
    }

    pub fn sheet_name(&self, id: SheetId) -> Option<&str> {
        self.sheet(id).map(|s| s.name.as_str())
    }

    /// Add an (empty) sheet; formulas referencing a missing sheet of that name now resolve to it.
    pub fn add_sheet(&mut self, name: &str) -> SheetId {
        let id = self.sheets.len() as SheetId;
        self.sheets.push(Some(Sheet { name: name.to_string(), ..Sheet::default() }));
        self.resolve_placeholders(name, id);
        id
    }

    fn resolve_placeholders(&mut self, name: &str, id: SheetId) {
        let l = name.to_lowercase();
        if self.placeholders.remove(&l).is_none() {
            return;
        }
        self.map_all_sheet_refs(&|s| match s {
            SheetRef::Missing(n) if n.to_lowercase() == l => Some(SheetRef::Id(id)),
            _ => None,
        });
    }

    fn map_all_sheet_refs(&mut self, f: &impl Fn(&SheetRef) -> Option<SheetRef>) {
        for s in self.sheets.iter_mut().flatten() {
            for c in s.cells.values_mut() {
                if let Content::Formula(fm) = c {
                    transform::map_sheets(&mut fm.ast, f);
                }
            }
        }
    }

    /// Rename a sheet (formula texts follow; a missing-sheet reference with the new name now resolves).
    pub fn rename_sheet(&mut self, id: SheetId, name: &str) -> Result<(), StructError> {
        let s = self.sheet_mut(id).ok_or(StructError::NoSheet)?;
        s.name = name.to_string();
        self.resolve_placeholders(name, id);
        Ok(())
    }

    /// Remove a sheet: references to it keep its name and evaluate to `#REF!`.
    pub fn remove_sheet(&mut self, id: SheetId) -> Result<(), StructError> {
        let name = self.sheet(id).ok_or(StructError::NoSheet)?.name.clone();
        self.sheets[id as usize] = None;
        let display = self.placeholders.entry(name.to_lowercase()).or_insert(name).clone();
        self.map_all_sheet_refs(&|s| match s {
            SheetRef::Id(x) if *x == id => Some(SheetRef::Missing(display.clone())),
            _ => None,
        });
        self.revalidate_claims();
        Ok(())
    }

    // ------------------------------------------------------------------ contents

    fn resolve_name(&self, name: &str) -> Option<u32> {
        self.sheet_id(name)
    }

    /// Parse a formula for a cell (sheet references resolved now).
    fn parse_formula(&mut self, text: &str, sheet: SheetId, row: u32, col: u32) -> Content {
        let resolver = |n: &str| self.resolve_name(n);
        match parser::parse_formula(text, &resolver, row as i64, col as i64) {
            Ok(mut ast) => {
                // Canonical spelling of missing sheet names (one placeholder per name).
                let mut missing: Vec<String> = Vec::new();
                ast.walk(&mut |a| {
                    let mut push = |s: &Option<SheetRef>| {
                        if let Some(SheetRef::Missing(n)) = s {
                            missing.push(n.clone());
                        }
                    };
                    match a {
                        Ast::Cell { r, .. } => push(&r.sheet),
                        Ast::Range { start, end, .. } => {
                            push(&start.sheet);
                            push(&end.sheet);
                        }
                        Ast::ColRange { start, end, .. } => {
                            push(&start.sheet);
                            push(&end.sheet);
                        }
                        Ast::RowRange { start, end, .. } => {
                            push(&start.sheet);
                            push(&end.sheet);
                        }
                        _ => {}
                    }
                });
                for n in missing {
                    self.placeholders.entry(n.to_lowercase()).or_insert(n);
                }
                let ph = self.placeholders.clone();
                transform::map_sheets(&mut ast, &|s| match s {
                    SheetRef::Missing(n) => ph.get(&n.to_lowercase()).filter(|d| *d != n).map(|d| SheetRef::Missing(d.clone())),
                    _ => None,
                });
                let size = self.predict(&ast, sheet, row, col);
                self.seq += 1;
                Content::Formula(Box::new(Formula { ast, size, state: ArrayState::Claimed, created: self.seq }))
            }
            Err(_) => Content::ParseError(text.to_string()),
        }
    }

    fn predict(&self, ast: &Ast, sheet: SheetId, row: u32, col: u32) -> ArraySize {
        let dims = |a: &Ast| -> Option<(f64, f64)> {
            match a {
                Ast::Range { start, end, .. } => {
                    let s0 = sheet_of(&start.sheet, sheet)?;
                    let s1 = sheet_of(&end.sheet, sheet)?;
                    if s0 != s1 || start.is_invalid() || end.is_invalid() {
                        return None;
                    }
                    Some(((end.col - start.col + 1) as f64, (end.row - start.row + 1) as f64))
                }
                Ast::ColRange { start, end, .. } => {
                    if sheet_of(&start.sheet, sheet)? != sheet_of(&end.sheet, sheet)? || start.col < 0 || end.col < 0 {
                        return None;
                    }
                    Some(((end.col - start.col + 1) as f64, f64::INFINITY))
                }
                Ast::RowRange { start, end, .. } => {
                    if sheet_of(&start.sheet, sheet)? != sheet_of(&end.sheet, sheet)? || start.row < 0 || end.row < 0 {
                        return None;
                    }
                    Some((f64::INFINITY, (end.row - start.row + 1) as f64))
                }
                _ => None,
            }
        };
        array_size::predict(ast, row as i64, col as i64, &dims)
    }

    fn grow(&mut self, sheet: SheetId, row: u32, col: u32) {
        if let Some(s) = self.sheet_mut(sheet) {
            s.height = s.height.max(row + 1);
            s.width = s.width.max(col + 1);
        }
    }

    /// Set the raw input of a cell (`None` / "" clears it). Leaves the workbook dirty.
    pub fn set_contents(&mut self, sheet: SheetId, row: u32, col: u32, raw: Option<&str>) {
        if self.sheet(sheet).is_none() || row as i64 >= MAX_ROWS || col as i64 >= MAX_COLUMNS {
            return;
        }
        let parsed = raw.and_then(parse_content);
        if let Some(&(ar, ac)) = self.spills.get(&(sheet, row, col)) {
            // A cell another array formula spilled into: clearing it changes nothing, writing it shrinks the
            // array for good (see [`ArrayState`]).
            if parsed.is_none() {
                return;
            }
            self.set_no_space((sheet, ar, ac));
        }
        // A formula replaced here gives its area up.
        self.release_claim((sheet, row, col));
        let content = match parsed {
            None => {
                if let Some(s) = self.sheet_mut(sheet) {
                    s.cells.remove(&(row, col));
                }
                return;
            }
            Some(RawContent::Value(v)) => Content::Value(v),
            Some(RawContent::Formula(f)) => self.parse_formula(&f, sheet, row, col),
        };
        let array = matches!(&content, Content::Formula(f) if !f.size.is_scalar());
        if let Some(s) = self.sheet_mut(sheet) {
            s.cells.insert((row, col), content);
        }
        self.grow(sheet, row, col);
        if array {
            self.try_claim((sheet, row, col));
        }
    }

    /// Area of an array formula when it fits on the sheet: (rows, cols).
    fn array_area(&self, a: Addr) -> Option<(u32, u32)> {
        let Some(Content::Formula(f)) = self.content(a.0, a.1, a.2) else { return None };
        if f.size.is_scalar() {
            return None;
        }
        let (w, h) = (f.size.width, f.size.height);
        let fits = w.is_finite()
            && h.is_finite()
            && w >= 1.0
            && h >= 1.0
            && (a.1 as f64 + h) <= MAX_ROWS as f64
            && (a.2 as f64 + w) <= MAX_COLUMNS as f64;
        fits.then_some((h as u32, w as u32))
    }

    /// `isThereSpaceForArray`: every other cell of the area is empty and not claimed by another array.
    fn area_free(&self, a: Addr) -> bool {
        let Some((h, w)) = self.array_area(a) else { return false };
        for r in a.1..a.1 + h {
            for c in a.2..a.2 + w {
                if (r, c) == (a.1, a.2) {
                    continue;
                }
                if self.content(a.0, r, c).is_some() || self.spills.get(&(a.0, r, c)).is_some_and(|x| *x != (a.1, a.2)) {
                    return false;
                }
            }
        }
        true
    }

    fn set_state(&mut self, a: Addr, state: ArrayState) {
        if let Some(Content::Formula(f)) = self.sheet_mut(a.0).and_then(|s| s.cells.get_mut(&(a.1, a.2))) {
            f.state = state;
        }
    }

    /// Claim the area of a just-entered array formula, or mark it unclaimed.
    fn try_claim(&mut self, a: Addr) {
        if self.area_free(a) {
            self.claim(a);
        } else {
            self.set_state(a, ArrayState::Unclaimed);
        }
    }

    fn claim(&mut self, a: Addr) {
        let Some((h, w)) = self.array_area(a) else { return };
        self.set_state(a, ArrayState::Claimed);
        self.spill_ok.insert(a);
        for r in a.1..a.1 + h {
            for c in a.2..a.2 + w {
                if (r, c) != (a.1, a.2) {
                    self.spills.insert((a.0, r, c), (a.1, a.2));
                }
            }
        }
        self.grow(a.0, a.1 + h - 1, a.2 + w - 1);
    }

    fn release_claim(&mut self, a: Addr) {
        if self.spill_ok.remove(&a) {
            self.spills.retain(|k, v| !(k.0 == a.0 && *v == (a.1, a.2)));
        }
    }

    /// `setNoSpace()`: shrink to the corner, `#SPILL!` from now on.
    fn set_no_space(&mut self, a: Addr) {
        self.release_claim(a);
        self.set_state(a, ArrayState::NoSpace);
    }

    /// Raw content of a cell.
    pub fn content(&self, sheet: SheetId, row: u32, col: u32) -> Option<&Content> {
        self.sheet(sheet)?.cells.get(&(row, col))
    }

    /// `getCellFormula`: the normalised formula text (`=…`) of a formula cell.
    pub fn formula(&self, sheet: SheetId, row: u32, col: u32) -> Option<String> {
        match self.content(sheet, row, col)? {
            Content::Formula(f) => Some(self.unparse(&f.ast)),
            Content::ParseError(t) => Some(t.clone()),
            Content::Value(_) => None,
        }
    }

    pub fn unparse(&self, ast: &Ast) -> String {
        let names = |id: u32| self.sheet_name(id).map(str::to_string);
        unparse::unparse(ast, &names)
    }

    // ------------------------------------------------------------------ structural edits

    fn check_sheet(&self, sheet: SheetId) -> Result<&Sheet, StructError> {
        self.sheet(sheet).ok_or(StructError::NoSheet)
    }

    /// `isItPossibleToAddRows` + `addRows`.
    pub fn add_rows(&mut self, sheet: SheetId, at: u32, n: u32) -> Result<(), StructError> {
        let s = self.check_sheet(sheet)?;
        if n == 0 {
            return Ok(());
        }
        if (s.height as i64 + n as i64) > MAX_ROWS {
            return Err(StructError::Limits);
        }
        self.apply_change(Change::AddRows { sheet, at: at as i64, n: n as i64 });
        Ok(())
    }

    pub fn add_cols(&mut self, sheet: SheetId, at: u32, n: u32) -> Result<(), StructError> {
        let s = self.check_sheet(sheet)?;
        if n == 0 {
            return Ok(());
        }
        if (s.width as i64 + n as i64) > MAX_COLUMNS {
            return Err(StructError::Limits);
        }
        self.apply_change(Change::AddCols { sheet, at: at as i64, n: n as i64 });
        Ok(())
    }

    pub fn remove_rows(&mut self, sheet: SheetId, at: u32, n: u32) -> Result<(), StructError> {
        self.check_sheet(sheet)?;
        if n == 0 {
            return Ok(());
        }
        self.apply_change(Change::RemoveRows { sheet, start: at as i64, end: at as i64 + n as i64 - 1 });
        Ok(())
    }

    pub fn remove_cols(&mut self, sheet: SheetId, at: u32, n: u32) -> Result<(), StructError> {
        self.check_sheet(sheet)?;
        if n == 0 {
            return Ok(());
        }
        self.apply_change(Change::RemoveCols { sheet, start: at as i64, end: at as i64 + n as i64 - 1 });
        Ok(())
    }

    fn apply_change(&mut self, c: Change) {
        // 1. Move / delete the cells of the changed sheet.
        let (sheet, rows) = match c {
            Change::AddRows { sheet, .. } | Change::RemoveRows { sheet, .. } => (sheet, true),
            Change::AddCols { sheet, .. } | Change::RemoveCols { sheet, .. } => (sheet, false),
        };
        if let Some(s) = self.sheet_mut(sheet) {
            let old = std::mem::take(&mut s.cells);
            for ((r, col), content) in old {
                let v = if rows { r as i64 } else { col as i64 };
                let nv = match c {
                    Change::AddRows { at, n, .. } | Change::AddCols { at, n, .. } => Some(if v >= at { v + n } else { v }),
                    Change::RemoveRows { start, end, .. } | Change::RemoveCols { start, end, .. } => {
                        if v < start {
                            Some(v)
                        } else if v > end {
                            Some(v - (end - start + 1))
                        } else {
                            None
                        }
                    }
                };
                let Some(nv) = nv else { continue };
                let key = if rows { (nv as u32, col) } else { (r, nv as u32) };
                s.cells.insert(key, content);
            }
            // DenseStrategy dimensions.
            match c {
                Change::AddRows { n, .. } => s.height += n as u32,
                Change::AddCols { n, .. } => s.width += n as u32,
                Change::RemoveRows { start, end, .. } => {
                    let last = (s.height as i64 - 1).min(end);
                    let removed = (last - start + 1).max(0);
                    s.height = (s.height as i64 - removed).max(0) as u32;
                }
                Change::RemoveCols { start, end, .. } => {
                    let last = (s.width as i64 - 1).min(end);
                    let removed = (last - start + 1).max(0);
                    s.width = (s.width as i64 - removed).max(0) as u32;
                }
            }
        }
        // 2. Rewrite every formula (every sheet may reference the changed one).
        for (id, s) in self.sheets.iter_mut().enumerate() {
            let Some(s) = s else { continue };
            for content in s.cells.values_mut() {
                if let Content::Formula(f) = content {
                    transform::transform_ast(&mut f.ast, &c, id as SheetId);
                }
            }
        }
        // 3. Array sizes may depend on the rewritten references.
        let ids = self.sheet_ids();
        for id in ids {
            let keys: Vec<(u32, u32)> = self
                .sheet(id)
                .map(|s| s.cells.iter().filter(|(_, c)| matches!(c, Content::Formula(_))).map(|(k, _)| *k).collect())
                .unwrap_or_default();
            for (r, col) in keys {
                let size = match self.content(id, r, col) {
                    Some(Content::Formula(f)) => self.predict(&f.ast, id, r, col),
                    _ => continue,
                };
                if let Some(Content::Formula(f)) = self.sheet_mut(id).and_then(|s| s.cells.get_mut(&(r, col))) {
                    f.size = size;
                }
            }
        }
        // 4. Array areas moved with their anchors; content may have moved into one.
        self.revalidate_claims();
    }

    // ------------------------------------------------------------------ evaluation

    /// Recompute every formula (static dependency graph, cycles → `#CYCLE!`).
    pub fn recompute(&mut self) {
        let mut formulas: Vec<Addr> = Vec::new();
        for id in self.sheet_ids() {
            let s = self.sheet(id).expect("live sheet");
            for (k, c) in &s.cells {
                if matches!(c, Content::Formula(_)) {
                    formulas.push((id, k.0, k.1));
                }
            }
        }
        formulas.sort();
        self.revalidate_claims();
        self.grow_for_dependencies(&formulas);
        let index: HashMap<Addr, usize> = formulas.iter().enumerate().map(|(i, a)| (*a, i)).collect();
        // Formula cells per (sheet, col) → rows, for range dependencies.
        let mut by_col: HashMap<(SheetId, u32), BTreeMap<u32, usize>> = HashMap::new();
        for (i, (s, r, c)) in formulas.iter().enumerate() {
            by_col.entry((*s, *c)).or_default().insert(*r, i);
        }
        // Spill areas (without their anchors) per sheet: (r0, c0, r1, c1, anchor node).
        let mut anchors: HashMap<SheetId, Vec<SpillArea>> = HashMap::new();
        for a in &self.spill_ok {
            let (Some(&i), Some(Content::Formula(f))) = (index.get(a), self.content(a.0, a.1, a.2)) else { continue };
            let (w, h) = (f.size.width as u32, f.size.height as u32);
            anchors.entry(a.0).or_default().push((a.1, a.2, a.1 + h - 1, a.2 + w - 1, i));
        }
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); formulas.len()];
        for (i, (s, r, c)) in formulas.iter().enumerate() {
            let Some(Content::Formula(f)) = self.content(*s, *r, *c) else { continue };
            let mut deps = Vec::new();
            collect_deps(&f.ast, true, *s, &mut deps);
            let mut out: HashSet<usize> = HashSet::new();
            for d in deps {
                let (sheet, r0, c0, r1, c1) = match d {
                    Dep::Cell(sh, row, col) => (sh, row, col, row, col),
                    Dep::Range(a) => (a.sheet, a.r0, a.c0, a.r1, a.c1),
                    Dep::Cols(sh, a, b) => (sh, 0, a, u32::MAX, b),
                    Dep::Rows(sh, a, b) => (sh, a, 0, b, u32::MAX),
                };
                if r0 > r1 || c0 > c1 {
                    continue;
                }
                if let Some(list) = anchors.get(&sheet) {
                    for &(ar0, ac0, ar1, ac1, j) in list {
                        if !(ar1 < r0 || ar0 > r1 || ac1 < c0 || ac0 > c1) {
                            out.insert(j);
                        }
                    }
                }
                let col_hi = c1.min(MAX_COLUMNS as u32);
                if col_hi < c0 {
                    continue;
                }
                if (col_hi - c0) as usize > by_col.len() {
                    for ((sh, col), rowsmap) in &by_col {
                        if *sh == sheet && *col >= c0 && *col <= col_hi {
                            out.extend(rowsmap.range(r0..=r1).map(|(_, j)| *j));
                        }
                    }
                } else {
                    for col in c0..=col_hi {
                        if let Some(rowsmap) = by_col.get(&(sheet, col)) {
                            out.extend(rowsmap.range(r0..=r1).map(|(_, j)| *j));
                        }
                    }
                }
            }
            let mut v: Vec<usize> = out.into_iter().collect();
            v.sort_unstable();
            edges[i] = v;
        }
        let (order, cycled) = tarjan(&edges);
        self.values.clear();
        for i in &order {
            let a = formulas[*i];
            let array = match self.content(a.0, a.1, a.2) {
                Some(Content::Formula(f)) if !f.size.is_scalar() => Some(f.state),
                _ => None,
            };
            // See [`ArrayState`].
            let computed = match (array, cycled[*i]) {
                (None, true) => Computed::S(Scalar::Err(ErrorType::Cycle)),
                (None, false) | (Some(ArrayState::Claimed), false) => self.evaluate(a),
                (Some(ArrayState::Claimed), true) => Computed::ArrayErr(ErrorType::Cycle),
                (Some(_), true) => Computed::S(Scalar::Err(ErrorType::Cycle)),
                // Space now, but the area was never mapped: the value shows in the anchor only.
                (Some(ArrayState::Unclaimed), false) if self.area_free(a) => self.evaluate(a),
                (Some(_), false) => {
                    self.set_no_space(a);
                    Computed::NoSpace
                }
            };
            self.values.insert(a, computed);
        }
    }

    fn evaluate(&self, a: Addr) -> Computed {
        let Some(Content::Formula(f)) = self.content(a.0, a.1, a.2) else { return Computed::S(Scalar::Empty) };
        let array = !f.size.is_scalar();
        let ctx = Ctx { wb: self };
        let it = Interp::new(&ctx);
        let v = it.eval(&f.ast, &State { sheet: a.0, row: a.1, col: a.2, arrays: false });
        if !array {
            return match v {
                Val::S(s) => Computed::S(s),
                Val::R(r) => Computed::R(r),
            };
        }
        let (w, h) = (f.size.width as usize, f.size.height as usize);
        let mut data = match v {
            Val::S(Scalar::Err(e)) => return Computed::ArrayErr(e),
            Val::S(s) => vec![vec![s]],
            Val::R(r) => r.data,
        };
        data.truncate(h);
        while data.len() < h {
            data.push(Vec::new());
        }
        for row in &mut data {
            row.truncate(w);
            while row.len() < w {
                row.push(Scalar::Empty);
            }
        }
        Computed::Array(data)
    }
}

impl Workbook {
    /// Re-validate the claims after cells moved (structural edits, removed sheets): claimed array formulas
    /// keep their areas, oldest first; one whose area is no longer free (or no longer fits) is `#SPILL!` for
    /// good, as HyperFormula's `fixArraysAfterRemovingRows` / `…Columns` leave it.
    fn revalidate_claims(&mut self) {
        let mut claimed: Vec<(u64, Addr)> = Vec::new();
        for id in self.sheet_ids() {
            for (k, c) in &self.sheet(id).expect("live sheet").cells {
                if let Content::Formula(f) = c
                    && !f.size.is_scalar()
                    && f.state == ArrayState::Claimed
                {
                    claimed.push((f.created, (id, k.0, k.1)));
                }
            }
        }
        claimed.sort_unstable();
        self.spills.clear();
        self.spill_ok.clear();
        for (_, a) in claimed {
            if self.area_free(a) {
                self.claim(a);
            } else {
                self.set_state(a, ArrayState::NoSpace);
            }
        }
    }

    /// Referenced cells and finite ranges create (empty) vertices that extend the sheet dimensions.
    fn grow_for_dependencies(&mut self, formulas: &[Addr]) {
        let mut grow: Vec<(SheetId, u32, u32)> = Vec::new();
        for a in formulas {
            let Some(Content::Formula(f)) = self.content(a.0, a.1, a.2) else { continue };
            let mut deps = Vec::new();
            collect_deps(&f.ast, true, a.0, &mut deps);
            for d in deps {
                match d {
                    Dep::Cell(s, r, c) => grow.push((s, r, c)),
                    Dep::Range(x) => grow.push((x.sheet, x.r1, x.c1)),
                    _ => {}
                }
            }
        }
        for (s, r, c) in grow {
            if (r as i64) < MAX_ROWS && (c as i64) < MAX_COLUMNS {
                self.grow(s, r, c);
            }
        }
    }

    // ------------------------------------------------------------------ reads

    fn computed_scalar(&self, a: Addr, direct: bool) -> Val {
        let Some(sheet) = self.sheet(a.0) else { return Val::err(ErrorType::Ref) };
        match sheet.cells.get(&(a.1, a.2)) {
            Some(Content::Value(v)) => Val::S(v.clone()),
            Some(Content::ParseError(_)) => Val::err(ErrorType::Error),
            Some(Content::Formula(_)) => match self.values.get(&a) {
                None => Val::S(Scalar::Empty),
                Some(Computed::S(s)) => Val::S(s.clone()),
                Some(Computed::R(r)) => {
                    if direct {
                        Val::R(r.clone())
                    } else {
                        Val::err(ErrorType::Value)
                    }
                }
                Some(Computed::Array(d)) => Val::S(d[0][0].clone()),
                Some(Computed::ArrayErr(e)) => Val::err(*e),
                Some(Computed::NoSpace) => Val::err(ErrorType::Spill),
            },
            None => match self.spills.get(&a) {
                None => Val::S(Scalar::Empty),
                Some((ar, ac)) => match self.values.get(&(a.0, *ar, *ac)) {
                    Some(Computed::Array(d)) => {
                        Val::S(d.get((a.1 - ar) as usize).and_then(|row| row.get((a.2 - ac) as usize)).cloned().unwrap_or(Scalar::Empty))
                    }
                    Some(Computed::ArrayErr(e)) => Val::err(*e),
                    _ => Val::S(Scalar::Empty),
                },
            },
        }
    }

    /// Internal value of a cell (typed number, unrounded).
    pub fn raw_value(&self, sheet: SheetId, row: u32, col: u32) -> Val {
        self.computed_scalar((sheet, row, col), true)
    }

    /// `getCellValue` as the TS engine exports it (`toValue`): smart-rounded numbers, error strings, `null`.
    pub fn value(&self, sheet: SheetId, row: u32, col: u32) -> CellValue {
        match self.raw_value(sheet, row, col) {
            Val::R(_) => CellValue::Error(ErrorType::Value.literal().into()),
            Val::S(s) => match s {
                Scalar::Empty => CellValue::Null,
                Scalar::Num(v, _) => CellValue::Number(smart_round(v)),
                Scalar::Str(t) => CellValue::String(t.to_string()),
                Scalar::Bool(b) => CellValue::Bool(b),
                Scalar::Err(e) => CellValue::Error(e.literal().into()),
            },
        }
    }

    /// `getCellValueDetailedType` mapped to the TS `ValueKind`.
    pub fn kind(&self, sheet: SheetId, row: u32, col: u32) -> ValueKind {
        match self.raw_value(sheet, row, col) {
            Val::S(Scalar::Num(_, t)) => match t {
                NumType::Raw => ValueKind::Number,
                NumType::Date => ValueKind::Date,
                NumType::Time => ValueKind::Time,
                NumType::DateTime => ValueKind::Datetime,
                NumType::Currency => ValueKind::Currency,
                NumType::Percent => ValueKind::Percent,
            },
            _ => ValueKind::Other,
        }
    }

    /// Sheet dimensions (`getSheetHeight`, `getSheetWidth`).
    pub fn dims(&self, sheet: SheetId) -> (u32, u32) {
        self.sheet(sheet).map_or((0, 0), |s| (s.height, s.width))
    }
}

/// The interpreter's view of the workbook.
struct Ctx<'a> {
    wb: &'a Workbook,
}

impl Env for Ctx<'_> {
    fn cell(&self, sheet: u32, row: u32, col: u32) -> Scalar {
        match self.wb.computed_scalar((sheet, row, col), false) {
            Val::S(s) => s,
            Val::R(_) => Scalar::Err(ErrorType::Value),
        }
    }
    fn cell_val(&self, sheet: u32, row: u32, col: u32) -> Val {
        self.wb.computed_scalar((sheet, row, col), true)
    }
    fn sheet_exists(&self, sheet: u32) -> bool {
        self.wb.sheet(sheet).is_some()
    }
    fn sheet_height(&self, sheet: u32) -> u32 {
        self.wb.dims(sheet).0
    }
    fn sheet_width(&self, sheet: u32) -> u32 {
        self.wb.dims(sheet).1
    }
    fn formula_text(&self, sheet: u32, row: u32, col: u32) -> Option<String> {
        match self.wb.content(sheet, row, col)? {
            Content::Formula(f) => Some(self.wb.unparse(&f.ast)),
            Content::ParseError(t) => Some(t.clone()),
            Content::Value(_) => None,
        }
    }
    fn sheet_number(&self, sheet: u32) -> Option<usize> {
        self.wb.sheet(sheet).map(|_| sheet as usize + 1)
    }
    fn sheet_count(&self) -> usize {
        self.wb.sheet_ids().len()
    }
    fn sheet_by_name(&self, name: &str) -> Option<u32> {
        self.wb.sheet_id(name)
    }
    fn now_ms(&self) -> f64 {
        self.wb.host.now_ms()
    }
    fn tz_offset_minutes(&self) -> f64 {
        self.wb.host.tz_offset_minutes()
    }
    fn random(&self) -> f64 {
        self.wb.host.random()
    }
}

/// Iterative Tarjan over `edges` (formula → formulas it depends on). Returns the evaluation order
/// (dependencies first) and which nodes lie on a cycle (non-trivial SCC or self loop).
fn tarjan(edges: &[Vec<usize>]) -> (Vec<usize>, Vec<bool>) {
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut cycled = vec![false; n];
    let mut counter = 0usize;
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut ei)) = call.last_mut() {
            if *ei < edges[v].len() {
                let w = edges[v][*ei];
                *ei += 1;
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(p, _)) = call.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut scc = Vec::new();
                    loop {
                        let w = stack.pop().expect("scc stack");
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let cyc = scc.len() > 1 || edges[v].contains(&v);
                    for w in &scc {
                        cycled[*w] = cyc;
                    }
                    order.extend(scc);
                }
            }
        }
    }
    (order, cycled)
}
