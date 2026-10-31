// SPDX-License-Identifier: AGPL-3.0-only
//! HyperFormula's dependency transformers (`AddRowsTransformer`, `RemoveRowsTransformer`, the column
//! variants and sheet removal/renaming), expressed on absolute coordinates: references shift with the cells
//! they point at, ranges grow or shrink, references into deleted tracks become `#REF!`.

use super::ast::{Ast, CellRef, ColRef, RowRef, SheetRef};
use super::value::ErrorType;

/// A structural change on one sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    AddRows { sheet: u32, at: i64, n: i64 },
    RemoveRows { sheet: u32, start: i64, end: i64 },
    AddCols { sheet: u32, at: i64, n: i64 },
    RemoveCols { sheet: u32, start: i64, end: i64 },
}

impl Change {
    fn sheet(&self) -> u32 {
        match *self {
            Change::AddRows { sheet, .. }
            | Change::RemoveRows { sheet, .. }
            | Change::AddCols { sheet, .. }
            | Change::RemoveCols { sheet, .. } => sheet,
        }
    }
    fn rows(&self) -> bool {
        matches!(self, Change::AddRows { .. } | Change::RemoveRows { .. })
    }
}

/// Result of transforming one coordinate.
enum Coord {
    Same,
    To(i64),
    Ref,
}

fn coord(c: &Change, v: i64) -> Coord {
    match *c {
        Change::AddRows { at, n, .. } | Change::AddCols { at, n, .. } => {
            if v >= at {
                Coord::To(v + n)
            } else {
                Coord::Same
            }
        }
        Change::RemoveRows { start, end, .. } | Change::RemoveCols { start, end, .. } => {
            if v < start {
                Coord::Same
            } else if v > end {
                Coord::To(v - (end - start + 1))
            } else {
                Coord::Ref
            }
        }
    }
}

fn ref_sheet(s: &Option<SheetRef>, formula_sheet: u32) -> Option<u32> {
    match s {
        None => Some(formula_sheet),
        Some(SheetRef::Id(id)) => Some(*id),
        Some(SheetRef::Missing(_)) => None,
    }
}

/// New position of a single address, `Err` = `#REF!`.
fn move_cell(c: &Change, r: &CellRef, fs: u32) -> Result<CellRef, ()> {
    if r.is_invalid() || ref_sheet(&r.sheet, fs) != Some(c.sheet()) {
        return Ok(r.clone());
    }
    let v = if c.rows() { r.row } else { r.col };
    let nv = match coord(c, v) {
        Coord::Same => return Ok(r.clone()),
        Coord::To(x) => x,
        Coord::Ref => return Err(()),
    };
    let mut out = r.clone();
    if c.rows() {
        out.row = nv;
    } else {
        out.col = nv;
    }
    Ok(out)
}

/// `transformRange` for removals: endpoints inside the removed span snap to its edges first.
fn shrink(c: &Change, a: i64, b: i64) -> Result<(i64, i64), ()> {
    match *c {
        Change::RemoveRows { start, end, .. } | Change::RemoveCols { start, end, .. } => {
            if start <= a && end >= b {
                return Err(());
            }
            let a2 = if a >= start && a <= end { end + 1 } else { a };
            let b2 = if b >= start && b <= end { start - 1 } else { b };
            Ok((a2, b2))
        }
        _ => Ok((a, b)),
    }
}

fn transform_range(c: &Change, start: &CellRef, end: &CellRef, fs: u32) -> Result<(CellRef, CellRef), ()> {
    let mut s = start.clone();
    let mut e = end.clone();
    if ref_sheet(&start.sheet, fs) == Some(c.sheet()) {
        let (a, b) = if c.rows() { (s.row, e.row) } else { (s.col, e.col) };
        let (a2, b2) = shrink(c, a, b)?;
        if c.rows() {
            s.row = a2;
            e.row = b2;
        } else {
            s.col = a2;
            e.col = b2;
        }
    }
    Ok((move_cell(c, &s, fs)?, move_cell(c, &e, fs)?))
}

fn transform_axis_range(c: &Change, a: i64, b: i64, sheet: &Option<SheetRef>, fs: u32) -> Result<(i64, i64), ()> {
    if ref_sheet(sheet, fs) != Some(c.sheet()) {
        return Ok((a, b));
    }
    let (a2, b2) = shrink(c, a, b)?;
    let m = |v: i64| match coord(c, v) {
        Coord::Same => Ok(v),
        Coord::To(x) => Ok(x),
        Coord::Ref => Err(()),
    };
    Ok((m(a2)?, m(b2)?))
}

fn ref_error(ws: &Option<String>) -> Ast {
    Ast::Error { e: ErrorType::Ref, ws: ws.clone() }
}

/// Transform every reference in `ast` (a formula on sheet `fs`).
pub fn transform_ast(ast: &mut Ast, c: &Change, fs: u32) {
    let replace = match ast {
        Ast::Cell { r, ws } => match move_cell(c, r, fs) {
            Ok(n) => {
                *r = n;
                None
            }
            Err(()) => Some(ref_error(ws)),
        },
        Ast::Range { start, end, ws, .. } => {
            if start.is_invalid() || end.is_invalid() {
                None
            } else {
                match transform_range(c, start, end, fs) {
                    Ok((s, e)) => {
                        *start = s;
                        *end = e;
                        None
                    }
                    Err(()) => Some(ref_error(ws)),
                }
            }
        }
        Ast::ColRange { start, end, ws, .. } => {
            if c.rows() || start.col < 0 || end.col < 0 {
                None
            } else {
                match transform_axis_range(c, start.col, end.col, &start.sheet, fs) {
                    Ok((a, b)) => {
                        *start = ColRef { col: a, ..start.clone() };
                        *end = ColRef { col: b, ..end.clone() };
                        None
                    }
                    Err(()) => Some(ref_error(ws)),
                }
            }
        }
        Ast::RowRange { start, end, ws, .. } => {
            if !c.rows() || start.row < 0 || end.row < 0 {
                None
            } else {
                match transform_axis_range(c, start.row, end.row, &start.sheet, fs) {
                    Ok((a, b)) => {
                        *start = RowRef { row: a, ..start.clone() };
                        *end = RowRef { row: b, ..end.clone() };
                        None
                    }
                    Err(()) => Some(ref_error(ws)),
                }
            }
        }
        Ast::Func { args, .. } => {
            args.iter_mut().for_each(|a| transform_ast(a, c, fs));
            None
        }
        Ast::Plus { v, .. } | Ast::Minus { v, .. } | Ast::Percent { v, .. } => {
            transform_ast(v, c, fs);
            None
        }
        Ast::Bin { l, r, .. } => {
            transform_ast(l, c, fs);
            transform_ast(r, c, fs);
            None
        }
        Ast::Paren { e, .. } => {
            transform_ast(e, c, fs);
            None
        }
        Ast::Array { rows, .. } => {
            rows.iter_mut().flatten().for_each(|a| transform_ast(a, c, fs));
            None
        }
        _ => None,
    };
    if let Some(r) = replace {
        *ast = r;
    }
}

/// Re-point sheet references: `f(sheet)` returns the replacement.
pub fn map_sheets(ast: &mut Ast, f: &impl Fn(&SheetRef) -> Option<SheetRef>) {
    let fix = |s: &mut Option<SheetRef>| {
        if let Some(x) = s
            && let Some(n) = f(x)
        {
            *s = Some(n);
        }
    };
    ast.walk_mut(&mut |a| match a {
        Ast::Cell { r, .. } => fix(&mut r.sheet),
        Ast::Range { start, end, .. } => {
            fix(&mut start.sheet);
            fix(&mut end.sheet);
        }
        Ast::ColRange { start, end, .. } => {
            fix(&mut start.sheet);
            fix(&mut end.sheet);
        }
        Ast::RowRange { start, end, .. } => {
            fix(&mut start.sheet);
            fix(&mut end.sheet);
        }
        _ => {}
    });
}
