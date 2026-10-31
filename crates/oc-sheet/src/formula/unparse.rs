// SPDX-License-Identifier: AGPL-3.0-only
//! HyperFormula's `Unparser`: AST → formula text (what `getCellFormula` returns).

use super::ast::{Ast, CellRef, ColRef, RangeSheet, RowRef, SheetRef};
use crate::a1::col_name;
use crate::jsnum::num_to_string;

/// Sheet names by engine id.
pub trait SheetNames {
    fn name(&self, id: u32) -> Option<String>;
}

impl<F: Fn(u32) -> Option<String>> SheetNames for F {
    fn name(&self, id: u32) -> Option<String> {
        self(id)
    }
}

fn ws(w: &Option<String>) -> &str {
    w.as_deref().unwrap_or("")
}

/// `sheetIndexToString`: quoted unless `^[A-Za-zÀ-ʯ0-9_]+$`.
pub fn sheet_to_string(name: &str) -> String {
    let simple = !name.is_empty() && name.chars().all(|c| super::lexer::is_letter(c) || c.is_ascii_digit() || c == '_');
    if simple { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

fn sheet_name(s: &SheetRef, names: &dyn SheetNames) -> String {
    match s {
        SheetRef::Id(id) => sheet_to_string(&names.name(*id).unwrap_or_default()),
        SheetRef::Missing(n) => sheet_to_string(n),
    }
}

fn cell(r: &CellRef) -> Option<String> {
    if r.is_invalid() {
        return None;
    }
    Some(format!(
        "{}{}{}{}",
        if r.abs_col { "$" } else { "" },
        col_name(r.col as usize),
        if r.abs_row { "$" } else { "" },
        r.row + 1
    ))
}

fn col(r: &ColRef) -> Option<String> {
    (r.col >= 0).then(|| format!("{}{}", if r.abs { "$" } else { "" }, col_name(r.col as usize)))
}

fn row(r: &RowRef) -> Option<String> {
    (r.row >= 0).then(|| format!("{}{}", if r.abs { "$" } else { "" }, r.row + 1))
}

fn range_text(
    kind: RangeSheet,
    s0: &Option<SheetRef>,
    s1: &Option<SheetRef>,
    a: Option<String>,
    b: Option<String>,
    names: &dyn SheetNames,
) -> String {
    let mut start_sheet = String::new();
    let mut end_sheet = String::new();
    if let Some(s) = s0
        && kind != RangeSheet::Relative
    {
        start_sheet = format!("{}!", sheet_name(s, names));
    }
    if let Some(s) = s1
        && kind == RangeSheet::BothAbsolute
    {
        end_sheet = format!("{}!", sheet_name(s, names));
    }
    match (a, b) {
        (Some(a), Some(b)) => format!("{start_sheet}{a}:{end_sheet}{b}"),
        _ => "#REF!".into(),
    }
}

/// Formula text including the leading `=`.
pub fn unparse(ast: &Ast, names: &dyn SheetNames) -> String {
    let mut s = String::from("=");
    write(ast, names, &mut s);
    s
}

fn write(ast: &Ast, names: &dyn SheetNames, out: &mut String) {
    match ast {
        Ast::Empty { ws: w } => out.push_str(ws(w)),
        Ast::Number { v, ws: w } => {
            out.push_str(ws(w));
            out.push_str(&num_to_string(*v));
        }
        Ast::Str { v, ws: w } => {
            out.push_str(ws(w));
            out.push('"');
            out.push_str(v);
            out.push('"');
        }
        Ast::Func { name, args, ws: w, internal_ws } => {
            out.push_str(ws(w));
            out.push_str(name);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(a, names, out);
            }
            out.push_str(ws(internal_ws));
            out.push(')');
        }
        Ast::Named { name, ws: w } => {
            out.push_str(ws(w));
            let up = name.to_uppercase();
            if up == "TRUE" || up == "FALSE" {
                out.push_str(&up);
            } else {
                out.push_str(name);
            }
        }
        Ast::Cell { r, ws: w } => {
            out.push_str(ws(w));
            if let Some(s) = &r.sheet {
                out.push_str(&sheet_name(s, names));
                out.push('!');
            }
            out.push_str(&cell(r).unwrap_or_else(|| "#REF!".into()));
        }
        Ast::Range { start, end, sheet, ws: w } => {
            out.push_str(ws(w));
            out.push_str(&range_text(*sheet, &start.sheet, &end.sheet, cell(start), cell(end), names));
        }
        Ast::ColRange { start, end, sheet, ws: w } => {
            out.push_str(ws(w));
            out.push_str(&range_text(*sheet, &start.sheet, &end.sheet, col(start), col(end), names));
        }
        Ast::RowRange { start, end, sheet, ws: w } => {
            out.push_str(ws(w));
            out.push_str(&range_text(*sheet, &start.sheet, &end.sheet, row(start), row(end), names));
        }
        Ast::Plus { v, ws: w } => {
            out.push_str(ws(w));
            out.push('+');
            write(v, names, out);
        }
        Ast::Minus { v, ws: w } => {
            out.push_str(ws(w));
            out.push('-');
            write(v, names, out);
        }
        Ast::Percent { v, ws: w } => {
            write(v, names, out);
            out.push_str(ws(w));
            out.push('%');
        }
        Ast::Error { e, ws: w } => {
            out.push_str(ws(w));
            out.push_str(e.literal());
        }
        Ast::ErrorRaw { raw, ws: w, .. } => {
            out.push_str(ws(w));
            out.push_str(raw);
        }
        Ast::Paren { e, ws: w, internal_ws } => {
            out.push_str(ws(w));
            out.push('(');
            write(e, names, out);
            out.push_str(ws(internal_ws));
            out.push(')');
        }
        Ast::Array { rows, ws: w, internal_ws } => {
            out.push_str(ws(w));
            out.push('{');
            for (i, r) in rows.iter().enumerate() {
                if i > 0 {
                    out.push(';');
                }
                for (j, a) in r.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    write(a, names, out);
                }
            }
            out.push_str(ws(internal_ws));
            out.push('}');
        }
        Ast::Bin { op, l, r, ws: w } => {
            write(l, names, out);
            out.push_str(ws(w));
            out.push_str(op.token());
            write(r, names, out);
        }
    }
}
