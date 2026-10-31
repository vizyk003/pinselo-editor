// SPDX-License-Identifier: AGPL-3.0-only
//! CSV / TSV (RFC 4180 quoting) — file import/export and clipboard exchange with other spreadsheets.

use oc_crdt::rng::RandomSource;
use yrs::TransactionMut;

use crate::a1::Range;
use crate::engine::SheetEngine;
use crate::format::format_value;
use crate::model::{Axis, TabSnap, insert_tracks, set_raw, snap_tab};

/// Parse delimited text into rows of fields. Handles quoted fields with embedded delimiters, quotes and
/// newlines; a leading BOM is dropped.
pub fn parse_delimited(text: &str, delim: char) -> Vec<Vec<String>> {
    let src = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let chars: Vec<char> = src.chars().collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if quoted {
            if ch == '"' {
                if chars.get(i + 1) == Some(&'"') {
                    field.push('"');
                    i += 2;
                    continue;
                }
                quoted = false;
                i += 1;
                continue;
            }
            field.push(ch);
            i += 1;
            continue;
        }
        if ch == '"' && field.is_empty() {
            quoted = true;
            i += 1;
            continue;
        }
        if ch == delim {
            row.push(std::mem::take(&mut field));
            i += 1;
            continue;
        }
        if ch == '\r' || ch == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            i += if ch == '\r' && chars.get(i + 1) == Some(&'\n') { 2 } else { 1 };
            continue;
        }
        field.push(ch);
        i += 1;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Quote a field when it needs it (delimiter, quote, newline; leading/trailing whitespace for CSV).
pub fn quote_field(s: &str, delim: char) -> String {
    let edge_space = delim == ',' && (s.chars().next().is_some_and(oc_schema::js::is_js_space) || s.chars().last().is_some_and(oc_schema::js::is_js_space));
    if s.contains(delim) || s.contains('"') || s.contains('\n') || s.contains('\r') || edge_space {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_delimited(rows: &[Vec<String>], delim: char, eol: &str) -> String {
    rows.iter()
        .map(|r| r.iter().map(|f| quote_field(f, delim)).collect::<Vec<_>>().join(&delim.to_string()))
        .collect::<Vec<_>>()
        .join(eol)
}

/// Displayed values of a range (what a user sees), as rows of strings.
pub fn display_rows(engine: &SheetEngine, t: &TabSnap, r: &Range) -> Vec<Vec<String>> {
    (r.r0..=r.r1)
        .map(|i| (r.c0..=r.c1).map(|j| format_value(&engine.value(&t.id, i, j), t.fmt_at(i, j), engine.kind(&t.id, i, j))).collect())
        .collect()
}

/// Raw inputs (formulas as typed) of a range.
pub fn raw_rows(t: &TabSnap, r: &Range) -> Vec<Vec<String>> {
    (r.r0..=r.r1).map(|i| (r.c0..=r.c1).map(|j| t.raw_at(i, j).to_string()).collect()).collect()
}

/// The whole used area of a tab as CSV (displayed values).
pub fn tab_to_csv(engine: &SheetEngine, t: &TabSnap) -> String {
    match t.used_range() {
        Some(used) => to_delimited(&display_rows(engine, t, &used), ',', "\r\n"),
        None => String::new(),
    }
}

/// Write rows of raw inputs at (r, c), growing the grid when needed (inside a transaction).
pub fn write_rows(txn: &mut TransactionMut, t: &TabSnap, r: usize, c: usize, rows: &[Vec<String>], rng: &mut dyn RandomSource) {
    let need_r = (r + rows.len()) as i64 - t.rows.len() as i64;
    let widest = rows.iter().map(Vec::len).max().unwrap_or(0);
    let need_c = (c + widest) as i64 - t.cols.len() as i64;
    if need_r > 0 {
        insert_tracks(txn, t, Axis::Rows, t.rows.len(), need_r as usize, rng);
    }
    if need_c > 0 {
        insert_tracks(txn, t, Axis::Cols, t.cols.len(), need_c as usize, rng);
    }
    let fresh;
    let s = if need_r > 0 || need_c > 0 {
        fresh = snap_tab(txn, &t.id, &t.map);
        &fresh
    } else {
        t
    };
    for (i, row) in rows.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            set_raw(txn, s, r + i, c + j, v);
        }
    }
}
