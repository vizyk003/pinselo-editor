// SPDX-License-Identifier: AGPL-3.0-only
//! Recursive-descent port of HyperFormula's `FormulaParser` grammar (same precedence, same OFFSET heuristic,
//! same error cases).

use super::ast::{Ast, BinOp, CellRef, ColRef, RangeSheet, RowRef, SheetRef, Ws};
use super::lexer::{SyntaxError, Tok, Token, tokenize};
use super::value::ErrorType;
use crate::a1::col_index;

/// Sheet-size limits of the engine config (`maxRows` / `maxColumns`).
pub const MAX_ROWS: i64 = 200_000;
pub const MAX_COLUMNS: i64 = 1000;

/// Resolves a sheet name (case-insensitive) to an engine sheet id.
pub trait SheetResolver {
    fn resolve(&self, name: &str) -> Option<u32>;
}

impl<F: Fn(&str) -> Option<u32>> SheetResolver for F {
    fn resolve(&self, name: &str) -> Option<u32> {
        self(name)
    }
}

struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    sheets: &'a dyn SheetResolver,
    /// Formula cell (row, col) — used by the OFFSET heuristic.
    at: (i64, i64),
}

type PResult = Result<Ast, SyntaxError>;

fn ws_of(t: &Token) -> Ws {
    t.ws.clone()
}

/// Parse `text` (starting with `=`). [`SyntaxError`] = parse error (`#ERROR!`).
pub fn parse_formula(text: &str, sheets: &dyn SheetResolver, row: i64, col: i64) -> PResult {
    let toks = tokenize(text)?;
    let mut p = Parser { toks, pos: 0, sheets, at: (row, col) };
    p.expect(Tok::Eq)?;
    let ast = p.boolean()?;
    if p.pos != p.toks.len() {
        return Err(SyntaxError);
    }
    Ok(ast)
}

/// Split a sheet-prefixed address image: (`Some(sheet name)`, rest).
fn split_sheet(image: &str) -> (Option<String>, &str) {
    if let Some(stripped) = image.strip_prefix('\'') {
        // Quoted: find the closing quote followed by '!'.
        let b: Vec<char> = stripped.chars().collect();
        let mut i = 0;
        let mut name = String::new();
        while i < b.len() {
            if b[i] == '\'' {
                if b.get(i + 1) == Some(&'\'') {
                    name.push('\'');
                    i += 2;
                    continue;
                }
                break;
            }
            name.push(b[i]);
            i += 1;
        }
        let consumed: usize = 1 + b[..=i.min(b.len().saturating_sub(1))].iter().map(|c| c.len_utf8()).sum::<usize>();
        let rest = &image[consumed..];
        return (Some(name), rest.strip_prefix('!').unwrap_or(rest));
    }
    match image.find('!') {
        Some(i) => (Some(image[..i].to_string()), &image[i + 1..]),
        None => (None, image),
    }
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.kind)
    }
    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos + k).map(|t| &t.kind)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, k: Tok) -> Result<Token, SyntaxError> {
        if self.peek() == Some(&k) {
            Ok(self.next().expect("peeked"))
        } else {
            Err(SyntaxError)
        }
    }

    fn sheet(&self, name: Option<String>) -> Result<Option<SheetRef>, SyntaxError> {
        Ok(name.map(|n| match self.sheets.resolve(&n) {
            Some(id) => SheetRef::Id(id),
            None => SheetRef::Missing(n),
        }))
    }

    /// `cellAddressFromString` (sheet resolution failures are kept as placeholders).
    fn cell_addr(&self, image: &str) -> Result<CellRef, SyntaxError> {
        let (sheet, rest) = split_sheet(image);
        let b = rest.as_bytes();
        let mut i = 0;
        let abs_col = b.first() == Some(&b'$');
        if abs_col {
            i += 1;
        }
        let cs = i;
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let col_s = &rest[cs..i];
        let abs_row = b.get(i) == Some(&b'$');
        if abs_row {
            i += 1;
        }
        let row_s = &rest[i..];
        Ok(CellRef {
            sheet: self.sheet(sheet)?,
            row: row_s.parse::<f64>().map_err(|_| SyntaxError)? as i64 - 1,
            col: column_label_to_index(col_s),
            abs_row,
            abs_col,
        })
    }

    fn col_addr(&self, image: &str) -> Result<ColRef, SyntaxError> {
        let (sheet, rest) = split_sheet(image);
        let abs = rest.starts_with('$');
        let label = rest.trim_start_matches('$');
        Ok(ColRef { sheet: self.sheet(sheet)?, col: column_label_to_index(label), abs })
    }

    fn row_addr(&self, image: &str) -> Result<RowRef, SyntaxError> {
        let (sheet, rest) = split_sheet(image);
        let abs = rest.starts_with('$');
        let digits = rest.trim_start_matches('$');
        Ok(RowRef { sheet: self.sheet(sheet)?, row: digits.parse::<f64>().map_err(|_| SyntaxError)? as i64 - 1, abs })
    }

    fn boolean(&mut self) -> PResult {
        let mut lhs = self.concat()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Ge) => BinOp::Ge,
                Some(Tok::Le) => BinOp::Le,
                _ => break,
            };
            let t = self.next().expect("op");
            let rhs = self.concat()?;
            lhs = Ast::Bin { op, l: Box::new(lhs), r: Box::new(rhs), ws: ws_of(&t) };
        }
        Ok(lhs)
    }

    fn concat(&mut self) -> PResult {
        let mut lhs = self.addition()?;
        while self.peek() == Some(&Tok::Amp) {
            let t = self.next().expect("op");
            let rhs = self.addition()?;
            lhs = Ast::Bin { op: BinOp::Concat, l: Box::new(lhs), r: Box::new(rhs), ws: ws_of(&t) };
        }
        Ok(lhs)
    }

    fn addition(&mut self) -> PResult {
        let mut lhs = self.multiplication()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Plus,
                Some(Tok::Minus) => BinOp::Minus,
                _ => break,
            };
            let t = self.next().expect("op");
            let rhs = self.multiplication()?;
            lhs = Ast::Bin { op, l: Box::new(lhs), r: Box::new(rhs), ws: ws_of(&t) };
        }
        Ok(lhs)
    }

    fn multiplication(&mut self) -> PResult {
        let mut lhs = self.power()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Times) => BinOp::Times,
                Some(Tok::Div) => BinOp::Div,
                _ => break,
            };
            let t = self.next().expect("op");
            let rhs = self.power()?;
            lhs = Ast::Bin { op, l: Box::new(lhs), r: Box::new(rhs), ws: ws_of(&t) };
        }
        Ok(lhs)
    }

    fn power(&mut self) -> PResult {
        let mut lhs = self.atomic()?;
        while self.peek() == Some(&Tok::Pow) {
            let t = self.next().expect("op");
            let rhs = self.atomic()?;
            lhs = Ast::Bin { op: BinOp::Pow, l: Box::new(lhs), r: Box::new(rhs), ws: ws_of(&t) };
        }
        Ok(lhs)
    }

    fn atomic(&mut self) -> PResult {
        match self.peek() {
            Some(Tok::Plus) => {
                let t = self.next().expect("op");
                let v = self.atomic()?;
                Ok(Ast::Plus { v: Box::new(v), ws: ws_of(&t) })
            }
            Some(Tok::Minus) => {
                let t = self.next().expect("op");
                let v = self.atomic()?;
                Ok(Ast::Minus { v: Box::new(v), ws: ws_of(&t) })
            }
            _ => {
                let v = self.positive_atomic()?;
                if self.peek() == Some(&Tok::Percent) {
                    let t = self.next().expect("op");
                    return Ok(Ast::Percent { v: Box::new(v), ws: ws_of(&t) });
                }
                Ok(v)
            }
        }
    }

    fn positive_atomic(&mut self) -> PResult {
        match self.peek().cloned() {
            Some(Tok::LBrace) => self.array(),
            Some(Tok::LParen) => {
                let l = self.next().expect("(");
                let e = self.boolean()?;
                let r = self.expect(Tok::RParen)?;
                Ok(Ast::Paren { e: Box::new(e), ws: ws_of(&l), internal_ws: ws_of(&r) })
            }
            Some(Tok::CellRef) if self.peek_at(1) == Some(&Tok::RangeSep) && matches!(self.peek_at(2), Some(Tok::CellRef) | Some(Tok::Offset)) => {
                let start = self.next().expect("ref");
                self.next();
                if self.peek() == Some(&Tok::CellRef) {
                    let end = self.next().expect("ref");
                    let a = self.cell_addr(&start.image)?;
                    let b = self.cell_addr(&end.image)?;
                    let raw = format!("{}:{}", start.image, end.image);
                    if let Some(e) = self.limits_error(&[&a, &b], &raw, &start.ws) {
                        return Ok(e);
                    }
                    self.build_range(a, b, ws_of(&start))
                } else {
                    let off = self.offset()?;
                    let a = self.cell_addr(&start.image)?;
                    match off {
                        Ast::Cell { r, .. } => self.build_range(a, r, ws_of(&start)),
                        _ => Err(SyntaxError),
                    }
                }
            }
            Some(Tok::ColRange) => {
                let t = self.next().expect("range");
                let (sa, sb) = t.image.split_once(':').ok_or(SyntaxError)?;
                let a = self.col_addr(sa)?;
                let b = self.col_addr(sb)?;
                if self.stored_col(a.col, a.abs) >= MAX_COLUMNS || self.stored_col(b.col, b.abs) >= MAX_COLUMNS {
                    return Ok(Ast::ErrorRaw { raw: t.image.clone(), e: ErrorType::Name, ws: ws_of(&t) });
                }
                if a.sheet.is_none() && b.sheet.is_some() {
                    return Err(SyntaxError);
                }
                let sheet = range_sheet(a.sheet.is_some(), b.sheet.is_some());
                let b = if a.sheet.is_some() && b.sheet.is_none() { ColRef { sheet: a.sheet.clone(), ..b } } else { b };
                let (start, end) = order_cols(a, b);
                Ok(Ast::ColRange { start, end, sheet, ws: ws_of(&t) })
            }
            Some(Tok::RowRange) => {
                let t = self.next().expect("range");
                let (sa, sb) = t.image.split_once(':').ok_or(SyntaxError)?;
                let a = self.row_addr(sa)?;
                let b = self.row_addr(sb)?;
                if self.stored_row(a.row, a.abs) >= MAX_ROWS || self.stored_row(b.row, b.abs) >= MAX_ROWS {
                    return Ok(Ast::ErrorRaw { raw: t.image.clone(), e: ErrorType::Name, ws: ws_of(&t) });
                }
                if a.sheet.is_none() && b.sheet.is_some() {
                    return Err(SyntaxError);
                }
                let sheet = range_sheet(a.sheet.is_some(), b.sheet.is_some());
                let b = if a.sheet.is_some() && b.sheet.is_none() { RowRef { sheet: a.sheet.clone(), ..b } } else { b };
                let (start, end) = order_rows(a, b);
                Ok(Ast::RowRange { start, end, sheet, ws: ws_of(&t) })
            }
            Some(Tok::Offset) => {
                let off = self.offset()?;
                if self.peek() == Some(&Tok::RangeSep) {
                    self.next();
                    if matches!(off, Ast::Range { .. }) {
                        return Err(SyntaxError);
                    }
                    let Ast::Cell { r: start, .. } = off else { return Err(SyntaxError) };
                    if self.peek() == Some(&Tok::CellRef) {
                        let end = self.next().expect("ref");
                        let b = self.cell_addr(&end.image)?;
                        return self.build_range(start, b, None);
                    }
                    if self.peek() == Some(&Tok::Offset) {
                        let o2 = self.offset()?;
                        return match o2 {
                            Ast::Cell { r, .. } => self.build_range(start, r, None),
                            _ => Err(SyntaxError),
                        };
                    }
                    return Err(SyntaxError);
                }
                Ok(off)
            }
            Some(Tok::CellRef) => {
                let t = self.next().expect("ref");
                let a = self.cell_addr(&t.image)?;
                if self.exceeds(&a) {
                    return Ok(Ast::ErrorRaw { raw: t.image.clone(), e: ErrorType::Name, ws: ws_of(&t) });
                }
                Ok(Ast::Cell { r: a, ws: ws_of(&t) })
            }
            Some(Tok::Proc) => self.procedure(),
            Some(Tok::Named) => {
                let t = self.next().expect("name");
                Ok(Ast::Named { name: t.image.clone(), ws: ws_of(&t) })
            }
            Some(Tok::Number) => {
                let t = self.next().expect("num");
                Ok(Ast::Number { v: oc_schema::js::string_to_number(&t.image), ws: ws_of(&t) })
            }
            Some(Tok::Str) => {
                let t = self.next().expect("str");
                let chars: Vec<char> = t.image.chars().collect();
                let v: String = chars[1..chars.len() - 1].iter().collect();
                Ok(Ast::Str { v, ws: ws_of(&t) })
            }
            Some(Tok::ErrorLit) => {
                let t = self.next().expect("err");
                match ErrorType::from_literal(&t.image.to_uppercase()) {
                    Some(e) => Ok(Ast::Error { e, ws: ws_of(&t) }),
                    None => Err(SyntaxError),
                }
            }
            _ => Err(SyntaxError),
        }
    }

    /// HyperFormula stores relative coordinates as offsets from the formula cell and checks the sheet limits
    /// on the stored value (`exceedsSheetSizeLimits`), so a far relative reference can pass the check.
    fn stored_row(&self, row: i64, abs: bool) -> i64 {
        if abs { row } else { row - self.at.0 }
    }
    fn stored_col(&self, col: i64, abs: bool) -> i64 {
        if abs { col } else { col - self.at.1 }
    }
    fn exceeds(&self, a: &CellRef) -> bool {
        self.stored_col(a.col, a.abs_col) >= MAX_COLUMNS || self.stored_row(a.row, a.abs_row) >= MAX_ROWS
    }

    fn limits_error(&self, addrs: &[&CellRef], raw: &str, ws: &Ws) -> Option<Ast> {
        addrs
            .iter()
            .any(|a| self.exceeds(a))
            .then(|| Ast::ErrorRaw { raw: raw.to_string(), e: ErrorType::Name, ws: ws.clone() })
    }

    fn build_range(&self, a: CellRef, b: CellRef, ws: Ws) -> PResult {
        if a.sheet.is_none() && b.sheet.is_some() {
            return Err(SyntaxError);
        }
        let sheet = range_sheet(a.sheet.is_some(), b.sheet.is_some());
        let b = if a.sheet.is_some() && b.sheet.is_none() { CellRef { sheet: a.sheet.clone(), ..b } } else { b };
        let (start, end) = order_cells(a, b);
        Ok(Ast::Range { start, end, sheet, ws })
    }

    fn procedure(&mut self) -> PResult {
        let t = self.next().expect("proc");
        let upper = t.image.to_uppercase();
        let name = upper[..upper.len() - 1].to_string();
        let mut args = Vec::new();
        let mut arg = self.boolean_or_empty();
        while self.peek() == Some(&Tok::Comma) {
            let sep = self.next().expect(",");
            let mut a = arg?;
            if let Ast::Empty { ws } = &mut a {
                *ws = sep.ws.clone();
            }
            args.push(a);
            arg = self.boolean_or_empty();
        }
        args.push(arg?);
        if args.len() == 1 && matches!(args[0], Ast::Empty { .. }) {
            args.clear();
        }
        let r = self.expect(Tok::RParen)?;
        Ok(Ast::Func { name, args, ws: ws_of(&t), internal_ws: ws_of(&r) })
    }

    fn boolean_or_empty(&mut self) -> PResult {
        if matches!(self.peek(), Some(Tok::Comma) | Some(Tok::RParen) | None) {
            return Ok(Ast::Empty { ws: None });
        }
        self.boolean()
    }

    fn array(&mut self) -> PResult {
        let l = self.expect(Tok::LBrace)?;
        let mut rows: Vec<Vec<Ast>> = vec![vec![self.boolean()?]];
        loop {
            match self.peek() {
                Some(Tok::Comma) => {
                    self.next();
                    let e = self.boolean()?;
                    rows.last_mut().expect("row").push(e);
                }
                Some(Tok::Semi) => {
                    self.next();
                    let e = self.boolean()?;
                    rows.push(vec![e]);
                }
                _ => break,
            }
        }
        let r = self.expect(Tok::RBrace)?;
        Ok(Ast::Array { rows, ws: ws_of(&l), internal_ws: ws_of(&r) })
    }

    /// `OFFSET(...)` resolved statically (`handleOffsetHeuristic`).
    fn offset(&mut self) -> PResult {
        self.expect(Tok::Offset)?;
        self.expect(Tok::LParen)?;
        let mut args = Vec::new();
        if self.peek() != Some(&Tok::RParen) {
            args.push(self.boolean()?);
            while self.peek() == Some(&Tok::Comma) {
                self.next();
                args.push(self.boolean()?);
            }
        }
        self.expect(Tok::RParen)?;
        let Some(Ast::Cell { r: cell, .. }) = args.first() else { return Err(SyntaxError) };
        let int_arg = |a: Option<&Ast>| -> Option<i64> {
            match a? {
                Ast::Number { v, .. } if oc_schema::js::is_integer(*v) => Some(*v as i64),
                Ast::Plus { v, .. } => match &**v {
                    Ast::Number { v, .. } if oc_schema::js::is_integer(*v) => Some(*v as i64),
                    _ => None,
                },
                Ast::Minus { v, .. } => match &**v {
                    Ast::Number { v, .. } if oc_schema::js::is_integer(*v) => Some(-(*v as i64)),
                    _ => None,
                },
                _ => None,
            }
        };
        let rs = int_arg(args.get(1)).ok_or(SyntaxError)?;
        let cs = int_arg(args.get(2)).ok_or(SyntaxError)?;
        let size = |a: Option<&Ast>| -> Result<i64, SyntaxError> {
            match a {
                None => Ok(1),
                Some(Ast::Number { v, .. }) if *v >= 1.0 && oc_schema::js::is_integer(*v) => Ok(*v as i64),
                _ => Err(SyntaxError),
            }
        };
        let h = size(args.get(3))?;
        let w = size(args.get(4))?;
        let top = CellRef { row: cell.row + rs, col: cell.col + cs, ..cell.clone() };
        if top.row < 0 || top.col < 0 {
            return Ok(Ast::Error { e: ErrorType::Ref, ws: None });
        }
        if w == 1 && h == 1 {
            return Ok(Ast::Cell { r: top, ws: None });
        }
        let end = CellRef { row: top.row + h - 1, col: top.col + w - 1, ..top.clone() };
        let sheet = if cell.sheet.is_none() { RangeSheet::Relative } else { RangeSheet::BothAbsolute };
        Ok(Ast::Range { start: top, end, sheet, ws: None })
    }
}

fn range_sheet(a: bool, b: bool) -> RangeSheet {
    match (a, b) {
        (false, _) => RangeSheet::Relative,
        (true, false) => RangeSheet::StartAbsolute,
        (true, true) => RangeSheet::BothAbsolute,
    }
}

/// HyperFormula's `columnLabelToIndex` (no length limit).
pub fn column_label_to_index(label: &str) -> i64 {
    if label.len() <= 3
        && let Some(i) = col_index(label)
    {
        return i as i64;
    }
    let mut n: i64 = 0;
    for b in label.bytes() {
        n = n.saturating_mul(26).saturating_add((b.to_ascii_uppercase() - b'A' + 1) as i64);
    }
    n - 1
}

fn sheet_order(a: &Option<SheetRef>, b: &Option<SheetRef>) -> (Option<SheetRef>, Option<SheetRef>) {
    // `compareSheetIds`: undefined first, then by id.
    let key = |s: &Option<SheetRef>| match s {
        None => (0, 0, String::new()),
        Some(SheetRef::Id(i)) => (1, *i as i64, String::new()),
        Some(SheetRef::Missing(n)) => (2, 0, n.clone()),
    };
    if key(a) <= key(b) { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) }
}

fn order_cells(a: CellRef, b: CellRef) -> (CellRef, CellRef) {
    let ((c0, ac0), (c1, ac1)) = if a.col <= b.col { ((a.col, a.abs_col), (b.col, b.abs_col)) } else { ((b.col, b.abs_col), (a.col, a.abs_col)) };
    let ((r0, ar0), (r1, ar1)) = if a.row <= b.row { ((a.row, a.abs_row), (b.row, b.abs_row)) } else { ((b.row, b.abs_row), (a.row, a.abs_row)) };
    let (s0, s1) = sheet_order(&a.sheet, &b.sheet);
    (
        CellRef { sheet: s0, row: r0, col: c0, abs_row: ar0, abs_col: ac0 },
        CellRef { sheet: s1, row: r1, col: c1, abs_row: ar1, abs_col: ac1 },
    )
}

fn order_cols(a: ColRef, b: ColRef) -> (ColRef, ColRef) {
    let (s0, s1) = sheet_order(&a.sheet, &b.sheet);
    let (x, y) = if a.col <= b.col { (a, b) } else { (b, a) };
    (ColRef { sheet: s0, ..x }, ColRef { sheet: s1, ..y })
}

fn order_rows(a: RowRef, b: RowRef) -> (RowRef, RowRef) {
    let (s0, s1) = sheet_order(&a.sheet, &b.sheet);
    let (x, y) = if a.row <= b.row { (a, b) } else { (b, a) };
    (RowRef { sheet: s0, ..x }, RowRef { sheet: s1, ..y })
}
