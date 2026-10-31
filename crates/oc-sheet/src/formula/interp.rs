// SPDX-License-Identifier: AGPL-3.0-only
//! Port of HyperFormula's `Interpreter.evaluateAst`: operators with implicit intersection (or element-wise
//! application in array mode), references, ranges, array literals, named expressions and function dispatch.

use super::ast::{Ast, BinOp, CellRef, SheetRef};
use super::coerce;
use super::functions;
use super::value::{AbsRange, ErrorType, NumType, RangeVal, Scalar, Val};

/// What the interpreter needs from the workbook.
pub trait Env {
    /// Computed scalar value of a cell as ranges read it (`getScalarValue`: a formula holding a range reads as
    /// `#VALUE!`; `Empty` when there is nothing).
    fn cell(&self, sheet: u32, row: u32, col: u32) -> Scalar;
    /// Value of a cell as a direct reference reads it (`getCellValue`: may be a range).
    fn cell_val(&self, sheet: u32, row: u32, col: u32) -> Val {
        Val::S(self.cell(sheet, row, col))
    }
    fn sheet_exists(&self, sheet: u32) -> bool;
    /// Rows holding data (`getSheetHeight`).
    fn sheet_height(&self, sheet: u32) -> u32;
    /// Columns holding data (`getSheetWidth`).
    fn sheet_width(&self, sheet: u32) -> u32;
    /// Formula text of a cell, if it holds a formula.
    fn formula_text(&self, sheet: u32, row: u32, col: u32) -> Option<String>;
    /// 1-based position of a sheet (`SHEET()`), and the number of sheets.
    fn sheet_number(&self, sheet: u32) -> Option<usize>;
    fn sheet_count(&self) -> usize;
    fn sheet_by_name(&self, name: &str) -> Option<u32>;
    /// Current time (ms since the Unix epoch, UTC) for TODAY/NOW.
    fn now_ms(&self) -> f64;
    /// Local timezone offset in minutes (`Date#getTimezoneOffset`), 0 = UTC.
    fn tz_offset_minutes(&self) -> f64 {
        0.0
    }
    /// Uniform random number in [0, 1).
    fn random(&self) -> f64;
}

/// `InterpreterState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub sheet: u32,
    pub row: u32,
    pub col: u32,
    pub arrays: bool,
}

pub struct Interp<'a> {
    pub env: &'a dyn Env,
}

pub fn fix_negative_zero(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

impl<'a> Interp<'a> {
    pub fn new(env: &'a dyn Env) -> Interp<'a> {
        Interp { env }
    }

    fn sheet_id(&self, s: &Option<SheetRef>, state: &State) -> Option<u32> {
        match s {
            None => Some(state.sheet),
            Some(SheetRef::Id(id)) => self.env.sheet_exists(*id).then_some(*id),
            Some(SheetRef::Missing(_)) => None,
        }
    }

    /// Materialise a range.
    pub fn range_val(&self, r: AbsRange) -> RangeVal {
        let mut data = Vec::with_capacity(r.height());
        for row in r.r0..=r.r1 {
            let mut v = Vec::with_capacity(r.width());
            for col in r.c0..=r.c1 {
                v.push(self.env.cell(r.sheet, row, col));
            }
            data.push(v);
        }
        RangeVal { data, range: Some(r) }
    }

    /// Absolute range of a range AST (`AbsoluteCellRange.fromAst`); `Err` = `#REF!`.
    pub fn abs_range(&self, ast: &Ast, state: &State) -> Result<AbsRange, ErrorType> {
        match ast {
            Ast::Range { start, end, .. } => {
                let s0 = self.sheet_id(&start.sheet, state).ok_or(ErrorType::Ref)?;
                let s1 = self.sheet_id(&end.sheet, state).ok_or(ErrorType::Ref)?;
                if s0 != s1 || start.is_invalid() || end.is_invalid() {
                    return Err(ErrorType::Ref);
                }
                Ok(AbsRange { sheet: s0, r0: start.row as u32, c0: start.col as u32, r1: end.row as u32, c1: end.col as u32 })
            }
            Ast::ColRange { start, end, .. } => {
                let s0 = self.sheet_id(&start.sheet, state).ok_or(ErrorType::Ref)?;
                let s1 = self.sheet_id(&end.sheet, state).ok_or(ErrorType::Ref)?;
                if s0 != s1 || start.col < 0 || end.col < 0 {
                    return Err(ErrorType::Ref);
                }
                let h = self.env.sheet_height(s0);
                Ok(AbsRange { sheet: s0, r0: 0, c0: start.col as u32, r1: h.max(1) - 1, c1: end.col as u32 })
            }
            Ast::RowRange { start, end, .. } => {
                let s0 = self.sheet_id(&start.sheet, state).ok_or(ErrorType::Ref)?;
                let s1 = self.sheet_id(&end.sheet, state).ok_or(ErrorType::Ref)?;
                if s0 != s1 || start.row < 0 || end.row < 0 {
                    return Err(ErrorType::Ref);
                }
                let w = self.env.sheet_width(s0);
                Ok(AbsRange { sheet: s0, r0: start.row as u32, c0: 0, r1: end.row as u32, c1: w.max(1) - 1 })
            }
            _ => Err(ErrorType::Ref),
        }
    }

    fn is_empty_axis_range(&self, ast: &Ast, r: &AbsRange) -> bool {
        match ast {
            Ast::ColRange { .. } => self.env.sheet_height(r.sheet) == 0,
            Ast::RowRange { .. } => self.env.sheet_width(r.sheet) == 0,
            _ => false,
        }
    }

    /// Value of a cell reference.
    pub fn cell_ref(&self, r: &CellRef, state: &State) -> Scalar {
        if r.is_invalid() {
            return Scalar::Err(ErrorType::Ref);
        }
        let Some(sheet) = self.sheet_id(&r.sheet, state) else { return Scalar::Err(ErrorType::Ref) };
        self.env.cell(sheet, r.row as u32, r.col as u32)
    }

    /// Value of a cell reference as an expression (`getCellValue`, may be a range).
    pub fn cell_ref_val(&self, r: &CellRef, state: &State) -> Val {
        if r.is_invalid() {
            return Val::err(ErrorType::Ref);
        }
        let Some(sheet) = self.sheet_id(&r.sheet, state) else { return Val::err(ErrorType::Ref) };
        self.env.cell_val(sheet, r.row as u32, r.col as u32)
    }

    /// `evaluateAst` (with post-processing: overflow → `#NUM!`, `-0` → `0`, 1×1 ranges unwrapped).
    pub fn eval(&self, ast: &Ast, state: &State) -> Val {
        let v = self.eval_raw(ast, state);
        match v {
            Val::S(Scalar::Num(x, t)) => {
                if coerce::is_overflow(x) {
                    Val::err(ErrorType::Num)
                } else {
                    Val::S(Scalar::Num(fix_negative_zero(x), t))
                }
            }
            Val::R(r) if r.height() == 1 && r.width() == 1 => Val::S(r.data[0][0].clone()),
            v => v,
        }
    }

    fn eval_raw(&self, ast: &Ast, state: &State) -> Val {
        match ast {
            Ast::Empty { .. } => Val::empty(),
            Ast::Cell { r, .. } => self.cell_ref_val(r, state),
            Ast::Number { v, .. } => Val::num(*v),
            Ast::Str { v, .. } => Val::str(v.as_str()),
            Ast::Bin { op, l, r, .. } => {
                let a = self.eval(l, state);
                let b = self.eval(r, state);
                self.binary_range(*op, a, b, state)
            }
            Ast::Plus { v, .. } => {
                let a = self.eval(v, state);
                self.unary_range(a, state, |s| s)
            }
            Ast::Minus { v, .. } => {
                let a = self.eval(v, state);
                self.unary_range(a, state, |s| match coerce::to_number_or_error(&s) {
                    Scalar::Num(x, t) => Scalar::Num(-x, t),
                    e => e,
                })
            }
            Ast::Percent { v, .. } => {
                let a = self.eval(v, state);
                self.unary_range(a, state, |s| match coerce::to_number_or_error(&s) {
                    Scalar::Num(x, _) => Scalar::Num(x / 100.0, NumType::Percent),
                    e => e,
                })
            }
            Ast::Func { name, args, .. } => functions::call(self, name, args, state),
            Ast::Named { name, .. } => match name.to_uppercase().as_str() {
                "TRUE" => Val::bool(true),
                "FALSE" => Val::bool(false),
                _ => Val::err(ErrorType::Name),
            },
            Ast::Range { .. } | Ast::ColRange { .. } | Ast::RowRange { .. } => match self.abs_range(ast, state) {
                Ok(r) => {
                    if self.is_empty_axis_range(ast, &r) {
                        return Val::R(RangeVal { data: vec![], range: Some(r) });
                    }
                    Val::R(self.range_val(r))
                }
                Err(e) => Val::err(e),
            },
            Ast::Paren { e, .. } => self.eval(e, state),
            Ast::Array { rows, .. } => {
                let mut total_width: Option<usize> = None;
                let mut ret: Vec<Vec<Scalar>> = Vec::new();
                for row in rows {
                    let mut row_height: Option<usize> = None;
                    let mut row_ret: Vec<Vec<Scalar>> = Vec::new();
                    for a in row {
                        let arr = match self.eval(a, state) {
                            Val::R(r) => r,
                            Val::S(s) => RangeVal::from_scalar(s),
                        };
                        let h = arr.height();
                        match row_height {
                            None => {
                                row_height = Some(h);
                                row_ret.extend(arr.data);
                            }
                            Some(rh) if rh == h => {
                                for (i, d) in arr.data.into_iter().enumerate() {
                                    row_ret[i].extend(d);
                                }
                            }
                            _ => return Val::err(ErrorType::Ref),
                        }
                    }
                    let width = row_ret.first().map_or(0, Vec::len);
                    match total_width {
                        None => {
                            total_width = Some(width);
                            ret.extend(row_ret);
                        }
                        Some(tw) if tw == width => ret.extend(row_ret),
                        _ => return Val::err(ErrorType::Ref),
                    }
                }
                Val::R(RangeVal::only_values(ret))
            }
            Ast::Error { e, .. } | Ast::ErrorRaw { e, .. } => Val::err(*e),
        }
    }

    fn unary_range(&self, arg: Val, state: &State, op: impl Fn(Scalar) -> Scalar) -> Val {
        let arg = match arg {
            Val::R(r) if !state.arrays => match super::func::range_to_scalar(&r, state) {
                Some(s) => Val::S(s),
                None => Val::err(ErrorType::Value),
            },
            v => v,
        };
        match arg {
            Val::S(Scalar::Err(e)) => Val::err(e),
            Val::R(r) => Val::R(RangeVal::only_values(r.data.into_iter().map(|row| row.into_iter().map(&op).collect()).collect())),
            Val::S(s) => Val::S(op(s)),
        }
    }

    fn binary_range(&self, op: BinOp, a: Val, b: Val, state: &State) -> Val {
        let to_scalar = |v: Val| -> Val {
            match v {
                Val::R(r) if !state.arrays => match super::func::range_to_scalar(&r, state) {
                    Some(s) => Val::S(s),
                    None => Val::err(ErrorType::Value),
                },
                v => v,
            }
        };
        let a = to_scalar(a);
        if let Val::S(Scalar::Err(e)) = a {
            return Val::err(e);
        }
        let b = to_scalar(b);
        if let Val::S(Scalar::Err(e)) = b {
            return Val::err(e);
        }
        match (a, b) {
            (Val::S(x), Val::S(y)) => Val::S(binary_op(op, &x, &y)),
            (a, b) => {
                let ra = match a {
                    Val::R(r) => r,
                    Val::S(s) => RangeVal::from_scalar(s),
                };
                let rb = match b {
                    Val::R(r) => r,
                    Val::S(s) => RangeVal::from_scalar(s),
                };
                let (ha, wa, hb, wb) = (ra.height(), ra.width(), rb.height(), rb.width());
                let h = ha.max(hb);
                let w = wa.max(wb);
                let mut out = Vec::with_capacity(h);
                for i in 0..h {
                    let i1 = if ha != 1 { i } else { 0 };
                    let i2 = if hb != 1 { i } else { 0 };
                    let mut row = Vec::with_capacity(w);
                    for j in 0..w {
                        let j1 = if wa != 1 { j } else { 0 };
                        let j2 = if wb != 1 { j } else { 0 };
                        if i1 < ha && i2 < hb && j1 < wa && j2 < wb {
                            row.push(binary_op(op, &ra.data[i1][j1], &rb.data[i2][j2]));
                        } else {
                            row.push(Scalar::Err(ErrorType::Na));
                        }
                    }
                    out.push(row);
                }
                Val::R(RangeVal::only_values(out))
            }
        }
    }
}

/// A binary operator on two scalars (`binaryErrorWrapper` around the arithmetic helper).
pub fn binary_op(op: BinOp, a: &Scalar, b: &Scalar) -> Scalar {
    match op {
        BinOp::Concat => {
            let x = coerce::to_string(a);
            let y = coerce::to_string(b);
            match (x, y) {
                (Scalar::Err(e), _) | (_, Scalar::Err(e)) => Scalar::Err(e),
                (Scalar::Str(x), Scalar::Str(y)) => Scalar::str(format!("{x}{y}")),
                _ => Scalar::Err(ErrorType::Value),
            }
        }
        BinOp::Eq | BinOp::Ne | BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le => {
            if let Scalar::Err(e) = a {
                return Scalar::Err(*e);
            }
            if let Scalar::Err(e) = b {
                return Scalar::Err(*e);
            }
            let c = coerce::compare(a, b);
            Scalar::Bool(match op {
                BinOp::Eq => c == 0,
                BinOp::Ne => c != 0,
                BinOp::Gt => c > 0,
                BinOp::Lt => c < 0,
                BinOp::Ge => c >= 0,
                _ => c <= 0,
            })
        }
        _ => {
            let x = coerce::to_number_or_error(a);
            let y = coerce::to_number_or_error(b);
            let (Scalar::Num(l, lt), Scalar::Num(r, rt)) = (&x, &y) else {
                return match (x, y) {
                    (Scalar::Err(e), _) | (_, Scalar::Err(e)) => Scalar::Err(e),
                    _ => Scalar::Err(ErrorType::Value),
                };
            };
            match op {
                BinOp::Plus => coerce::add(*l, *lt, *r, *rt),
                BinOp::Minus => coerce::subtract(*l, *lt, *r, *rt),
                BinOp::Times => coerce::multiply(*l, *lt, *r, *rt),
                BinOp::Div => coerce::divide(*l, *lt, *r, *rt),
                _ => Scalar::num(l.powf(*r)),
            }
        }
    }
}
