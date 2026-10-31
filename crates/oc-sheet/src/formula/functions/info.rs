// SPDX-License-Identifier: AGPL-3.0-only
//! InformationPlugin (+ FORMULATEXT, HYPERLINK, ADDRESS): IS* predicates, ROW(S), COLUMN(S), INDEX, NA,
//! SHEET(S).

use super::super::ast::Ast;
use super::super::func::{Arg, Call, Meta};
use super::super::interp::{Interp, State};
use super::super::value::{ErrorType, Scalar, Val};
use super::{Custom, Kind, err, num, s, simple};

fn is_err(a: &Arg) -> Option<ErrorType> {
    match a {
        Arg::S(Scalar::Err(e)) => Some(*e),
        _ => None,
    }
}

/// `runFunctionWithReferenceArgument`: (sheet, row, col) of a reference argument.
enum RefArg {
    None,
    Cell(u32, i64, i64),
    Other,
    Error(ErrorType),
}

fn reference_arg(it: &Interp, args: &[Ast], state: &State) -> RefArg {
    if args.is_empty() {
        return RefArg::None;
    }
    let a = args[0].unparen();
    match a {
        Ast::Cell { r, .. } => {
            let sheet = match &r.sheet {
                None => state.sheet,
                Some(super::super::ast::SheetRef::Id(id)) => *id,
                Some(super::super::ast::SheetRef::Missing(_)) => u32::MAX,
            };
            RefArg::Cell(sheet, r.row, r.col)
        }
        Ast::Range { .. } | Ast::ColRange { .. } | Ast::RowRange { .. } => match it.abs_range(a, state) {
            Ok(r) => RefArg::Cell(r.sheet, r.r0 as i64, r.c0 as i64),
            Err(_) => match a {
                Ast::Range { start, end, .. } if start.sheet != end.sheet => RefArg::Error(ErrorType::Ref),
                // HyperFormula reads the start of the range even when the sheet does not exist.
                Ast::Range { start, .. } => RefArg::Cell(u32::MAX, start.row, start.col),
                Ast::ColRange { start, .. } => RefArg::Cell(u32::MAX, 0, start.col),
                Ast::RowRange { start, .. } => RefArg::Cell(u32::MAX, start.row, 0),
                _ => RefArg::Error(ErrorType::Ref),
            },
        },
        _ => RefArg::Other,
    }
}

fn with_reference(
    it: &Interp,
    args: &[Ast],
    state: &State,
    meta: &Meta,
    no_arg: impl Fn() -> Val,
    reference: impl Fn(u32, i64, i64) -> Val,
    non_reference: super::super::func::Impl,
) -> Val {
    if args.len() > 1 {
        return err(ErrorType::Na);
    }
    match reference_arg(it, args, state) {
        RefArg::None => no_arg(),
        RefArg::Cell(sh, r, c) => reference(sh, r, c),
        RefArg::Error(e) => err(e),
        RefArg::Other => it.run_function(args, state, meta, non_reference),
    }
}

fn column(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(it, args, state, meta, || num(state.col as f64 + 1.0), |_, _, c| num(c as f64 + 1.0), |_, _| err(ErrorType::Na))
}

fn row(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(it, args, state, meta, || num(state.row as f64 + 1.0), |_, r, _| num(r as f64 + 1.0), |_, _| err(ErrorType::Na))
}

fn is_formula(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(
        it,
        args,
        state,
        meta,
        || err(ErrorType::Na),
        |sh, r, c| Val::bool(r >= 0 && c >= 0 && it.env.formula_text(sh, r as u32, c as u32).is_some()),
        |_, _| err(ErrorType::Na),
    )
}

fn formula_text(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(
        it,
        args,
        state,
        meta,
        || err(ErrorType::Na),
        |sh, r, c| {
            if r < 0 || c < 0 {
                return err(ErrorType::Na);
            }
            it.env.formula_text(sh, r as u32, c as u32).map_or(err(ErrorType::Na), s)
        },
        |_, _| err(ErrorType::Na),
    )
}

fn sheet(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(
        it,
        args,
        state,
        meta,
        || it.env.sheet_number(state.sheet).map_or(err(ErrorType::Na), |n| num(n as f64)),
        |sh, _, _| it.env.sheet_number(sh).map_or(err(ErrorType::Ref), |n| num(n as f64)),
        |c, a| match c.it.env.sheet_by_name(a[0].str()).and_then(|id| c.it.env.sheet_number(id)) {
            Some(n) => num(n as f64),
            None => err(ErrorType::Na),
        },
    )
}

fn sheets(it: &Interp, args: &[Ast], state: &State, meta: &Meta) -> Val {
    with_reference(it, args, state, meta, || num(it.env.sheet_count() as f64), |_, _, _| num(1.0), |_, _| err(ErrorType::Value))
}

fn dims(it: &Interp, args: &[Ast], state: &State, rows: bool) -> Val {
    if args.len() != 1 {
        return err(ErrorType::Na);
    }
    if args.iter().any(|a| matches!(a, Ast::Empty { .. })) {
        return err(ErrorType::Num);
    }
    let a = args[0].unparen();
    match a {
        Ast::Range { start, end, .. } => num(if rows { end.row - start.row + 1 } else { end.col - start.col + 1 } as f64),
        Ast::ColRange { start, end, .. } => num(if rows { super::super::parser::MAX_ROWS as f64 } else { (end.col - start.col + 1) as f64 }),
        Ast::RowRange { start, end, .. } => num(if rows { (end.row - start.row + 1) as f64 } else { super::super::parser::MAX_COLUMNS as f64 }),
        Ast::Cell { .. } => num(1.0),
        _ => match it.eval(a, state) {
            Val::R(r) => num(if rows { r.height() } else { r.width() } as f64),
            Val::S(Scalar::Err(e)) => err(e),
            Val::S(_) => num(1.0),
        },
    }
}

fn rows(it: &Interp, args: &[Ast], state: &State, _: &Meta) -> Val {
    dims(it, args, state, true)
}

fn columns(it: &Interp, args: &[Ast], state: &State, _: &Meta) -> Val {
    dims(it, args, state, false)
}

fn na(_: &Interp, _: &[Ast], _: &State, _: &Meta) -> Val {
    err(ErrorType::Na)
}

fn index(_: &Call, a: &[Arg]) -> Val {
    let Some(r) = a[0].range() else { return err(ErrorType::Value) };
    let row = a[1].num();
    let col = a[2].num();
    if col < 1.0 || row < 1.0 {
        return err(ErrorType::Value);
    }
    if col > r.width() as f64 || row > r.height() as f64 {
        return err(ErrorType::Num);
    }
    let v = r
        .data
        .get(row as usize - 1)
        .and_then(|x| x.get(col as usize - 1))
        .or_else(|| r.data.first().and_then(|x| x.first()))
        .cloned();
    v.map_or(err(ErrorType::Value), Val::S)
}

/// `columnIndexToLabel`.
pub fn column_label(col: f64) -> String {
    let mut out = Vec::new();
    let mut c = col;
    while c >= 0.0 {
        let code = (c % 26.0).trunc() as u32 + 97;
        out.push(char::from_u32(code).unwrap_or('a').to_ascii_uppercase());
        c = (c / 26.0).floor() - 1.0;
    }
    out.iter().rev().collect()
}

fn address(_: &Call, a: &[Arg]) -> Val {
    let row = a[0].num();
    let col = a[1].num();
    let abs = a[2].opt_num().unwrap_or(1.0);
    let a1 = a[3].opt_bool().unwrap_or(true);
    if a1 {
        if row < 1.0 || col < 1.0 {
            return err(ErrorType::Value);
        }
    } else if (abs == 1.0 && (row < 1.0 || col < 1.0)) || (abs == 2.0 && row < 1.0) || (abs == 3.0 && col < 1.0) {
        return err(ErrorType::Value);
    }
    let letter = column_label(col - 1.0);
    let prefix = a[4].opt_str().map(|x| format!("{x}!")).unwrap_or_default();
    let n = super::super::super::jsnum::num_to_string;
    let rc = if col == 0.0 { "C".to_string() } else { format!("C[{}]", n(col)) };
    let rr = if row == 0.0 { "R".to_string() } else { format!("R[{}]", n(row)) };
    let (r, c) = (n(row), n(col));
    let out = if abs == 4.0 {
        if a1 { format!("{prefix}{letter}{r}") } else { format!("{prefix}{rr}{rc}") }
    } else if abs == 3.0 {
        if a1 { format!("{prefix}${letter}{r}") } else { format!("{prefix}{rr}C{c}") }
    } else if abs == 2.0 {
        if a1 { format!("{prefix}{letter}${r}") } else { format!("{prefix}R{r}{rc}") }
    } else if a1 {
        format!("{prefix}${letter}${r}")
    } else {
        format!("{prefix}R{r}C{c}")
    };
    s(out)
}

fn isbinary(_: &Call, a: &[Arg]) -> Val {
    let t = a[0].str();
    Val::bool(!t.is_empty() && t.len() <= 10 && t.bytes().all(|b| b == b'0' || b == b'1'))
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    let mut v = simple(&[
        ("ISBINARY", isbinary),
        ("ISERR", |_, a| Val::bool(is_err(&a[0]).is_some_and(|e| e != ErrorType::Na))),
        ("ISERROR", |_, a| Val::bool(is_err(&a[0]).is_some())),
        ("ISBLANK", |_, a| Val::bool(matches!(a[0], Arg::S(Scalar::Empty)))),
        ("ISNA", |_, a| Val::bool(is_err(&a[0]) == Some(ErrorType::Na))),
        ("ISNUMBER", |_, a| Val::bool(matches!(a[0], Arg::S(Scalar::Num(..))))),
        ("ISLOGICAL", |_, a| Val::bool(matches!(a[0], Arg::S(Scalar::Bool(_))))),
        ("ISREF", |_, a| Val::bool(matches!(is_err(&a[0]), Some(ErrorType::Ref) | Some(ErrorType::Cycle)))),
        ("ISTEXT", |_, a| Val::bool(matches!(a[0], Arg::S(Scalar::Str(_))))),
        ("ISNONTEXT", |_, a| Val::bool(!matches!(a[0], Arg::S(Scalar::Str(_))))),
        ("INDEX", index),
        ("ADDRESS", address),
        ("HYPERLINK", |_, a| if a[1].is_missing() { a[0].clone().into_val() } else { a[1].clone().into_val() }),
    ]);
    let custom: [(&'static str, Custom); 9] = [
        ("COLUMN", column),
        ("ROW", row),
        ("COLUMNS", columns),
        ("ROWS", rows),
        ("ISFORMULA", is_formula),
        ("FORMULATEXT", formula_text),
        ("SHEET", sheet),
        ("SHEETS", sheets),
        ("NA", na),
    ];
    v.extend(custom.into_iter().map(|(n, f)| (n, Kind::Custom(f))));
    v
}
