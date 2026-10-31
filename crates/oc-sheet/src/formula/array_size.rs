// SPDX-License-Identifier: AGPL-3.0-only
//! HyperFormula's `ArraySizePredictor`: the static size of a formula's result. A formula whose predicted size
//! is not 1×1 (and not a plain reference) becomes an array formula that spills into the cells below/right.

use super::ast::{Ast, BinOp};
use super::func::ArgType;
use super::functions;
use super::parser::{MAX_COLUMNS, MAX_ROWS};

/// `ArraySize` (`is_ref`: the size of a reference — always treated as scalar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArraySize {
    pub width: f64,
    pub height: f64,
    pub is_ref: bool,
}

impl ArraySize {
    pub fn new(width: f64, height: f64) -> ArraySize {
        ArraySize { width, height, is_ref: false }
    }
    pub fn error() -> ArraySize {
        ArraySize { width: 1.0, height: 1.0, is_ref: true }
    }
    pub fn scalar() -> ArraySize {
        ArraySize::new(1.0, 1.0)
    }
    pub fn is_scalar(&self) -> bool {
        (self.width == 1.0 && self.height == 1.0) || self.is_ref
    }
}

/// Context: formula address (sheet, row, col) + array arithmetic flag.
#[derive(Debug, Clone, Copy)]
pub struct SizeState {
    pub row: i64,
    pub col: i64,
    pub arrays: bool,
}

/// Width/height of a range AST, `None` when it does not resolve (`fromAstOrUndef`).
pub type RangeDims<'a> = &'a dyn Fn(&Ast) -> Option<(f64, f64)>;

/// `checkArraySize(ast, address)` with `useArrayArithmetic: false`.
pub fn predict(ast: &Ast, row: i64, col: i64, dims: RangeDims) -> ArraySize {
    check(ast, SizeState { row, col, arrays: false }, dims)
}

fn check(ast: &Ast, st: SizeState, dims: RangeDims) -> ArraySize {
    match ast {
        Ast::Func { name, args, .. } => check_function(name, args, st, dims),
        Ast::Range { .. } | Ast::ColRange { .. } | Ast::RowRange { .. } => match dims(ast) {
            None => ArraySize::error(),
            Some((w, h)) => ArraySize { width: w, height: h, is_ref: true },
        },
        Ast::Array { rows, .. } => {
            let mut heights = Vec::new();
            let mut widths = Vec::new();
            for row in rows {
                let sizes: Vec<ArraySize> = row.iter().map(|a| check(a, st, dims)).collect();
                heights.push(sizes.iter().map(|s| s.height).fold(f64::INFINITY, f64::min));
                widths.push(sizes.iter().map(|s| s.width).sum::<f64>());
            }
            let height: f64 = heights.iter().sum();
            let width = widths.iter().copied().fold(f64::INFINITY, f64::min);
            ArraySize::new(width, height)
        }
        Ast::Str { .. } | Ast::Number { .. } => ArraySize::scalar(),
        Ast::Cell { .. } => ArraySize { width: 1.0, height: 1.0, is_ref: true },
        Ast::Bin { op, l, r, .. } => {
            let _: BinOp = *op;
            let left = check(l, st, dims);
            let right = check(r, st, dims);
            if !st.arrays && (left.height > 1.0 || left.width > 1.0 || right.height > 1.0 || right.width > 1.0) {
                return ArraySize::error();
            }
            ArraySize::new(left.width.max(right.width), left.height.max(right.height))
        }
        Ast::Minus { v, .. } | Ast::Plus { v, .. } | Ast::Percent { v, .. } => {
            let val = check(v, st, dims);
            if !st.arrays && (val.height > 1.0 || val.width > 1.0) {
                return ArraySize::error();
            }
            ArraySize::new(val.width, val.height)
        }
        Ast::Paren { e, .. } => check(e, st, dims),
        _ => ArraySize::error(),
    }
}

fn sub_checks(name: &str, args: &[Ast], st: SizeState, dims: RangeDims) -> Vec<ArraySize> {
    let array_fn = functions::meta_of(name).is_some_and(|m| m.array_fn);
    let inner = SizeState { arrays: st.arrays || array_fn, ..st };
    args.iter().map(|a| check(a, inner, dims)).collect()
}

fn literal_number(a: &Ast) -> Option<f64> {
    match a {
        Ast::Number { v, .. } => Some(*v),
        _ => None,
    }
}

fn check_function(name: &str, args: &[Ast], st: SizeState, dims: RangeDims) -> ArraySize {
    let canon = super::meta_table::alias(name).unwrap_or(name);
    match canon {
        "ARRAYFORMULA" => {
            if args.len() != 1 {
                return ArraySize::error();
            }
            sub_checks(canon, args, st, dims)[0]
        }
        "ARRAY_CONSTRAIN" => {
            if args.len() != 3 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            let (mut height, mut width) = (s[0].height, s[0].width);
            if let Some(v) = literal_number(&args[1]) {
                height = height.min(v);
            }
            if let Some(v) = literal_number(&args[2]) {
                width = width.min(v);
            }
            if height < 1.0 || width < 1.0 || height.fract() != 0.0 || width.fract() != 0.0 {
                return ArraySize::error();
            }
            ArraySize::new(width, height)
        }
        "FILTER" => {
            if args.len() <= 1 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s.iter().map(|x| x.width).fold(f64::NEG_INFINITY, f64::max), s.iter().map(|x| x.height).fold(f64::NEG_INFINITY, f64::max))
        }
        "VSTACK" => {
            if args.is_empty() {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s.iter().map(|x| x.width).fold(f64::NEG_INFINITY, f64::max), s.iter().map(|x| x.height).sum())
        }
        "HSTACK" => {
            if args.is_empty() {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s.iter().map(|x| x.width).sum(), s.iter().map(|x| x.height).fold(f64::NEG_INFINITY, f64::max))
        }
        "XLOOKUP" => xlookup_size(args),
        "MMULT" => {
            if args.len() != 2 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s[1].width, s[0].height)
        }
        "MAXPOOL" | "MEDIANPOOL" => {
            if args.len() < 2 || args.len() > 3 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            let array = s[0];
            let window = literal_number(&args[1]).unwrap_or(1.0);
            let stride = if args.len() == 3 { literal_number(&args[2]).unwrap_or(1.0) } else { window };
            if window > array.width
                || window > array.height
                || stride > window
                || (array.width - window) % stride != 0.0
                || (array.height - window) % stride != 0.0
            {
                return ArraySize::error();
            }
            ArraySize::new(1.0 + (array.width - window) / stride, 1.0 + (array.height - window) / stride)
        }
        "TRANSPOSE" => {
            if args.len() != 1 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s[0].height, s[0].width)
        }
        "SEQUENCE" => sequence_size(args),
        "SORT" => {
            if args.is_empty() || args.len() > 4 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s[0].width, s[0].height)
        }
        "UNIQUE" => {
            if args.is_empty() || args.len() > 3 {
                return ArraySize::error();
            }
            let s = sub_checks(canon, args, st, dims);
            ArraySize::new(s[0].width, s[0].height)
        }
        _ => generic_function(canon, args, st, dims),
    }
}

fn generic_function(name: &str, args: &[Ast], st: SizeState, dims: RangeDims) -> ArraySize {
    let Some(meta) = functions::meta_of(name) else { return ArraySize::new(1.0, 1.0) };
    if meta.expand_ranges || !st.arrays || meta.vectorization_forbidden {
        return ArraySize::new(1.0, 1.0);
    }
    let subs = sub_checks(name, args, st, dims);
    let mut defs: Vec<ArgType> = meta.params.iter().map(|p| p.ty).collect();
    let rep = meta.repeat_last;
    if rep > 0 && defs.len() < subs.len() && !(subs.len() - defs.len()).is_multiple_of(rep) {
        return ArraySize::error();
    }
    while defs.len() < subs.len() {
        if rep == 0 {
            return ArraySize::error();
        }
        let tail: Vec<ArgType> = defs[defs.len() - rep..].to_vec();
        defs.extend(tail);
    }
    let (mut w, mut h) = (1.0f64, 1.0f64);
    for (i, s) in subs.iter().enumerate() {
        if !matches!(defs[i], ArgType::Range | ArgType::Any) {
            h = h.max(s.height);
            w = w.max(s.width);
        }
    }
    ArraySize::new(w, h)
}

fn xlookup_size(args: &[Ast]) -> ArraySize {
    // Only literal cell ranges have `start`/`end` (column/row ranges have them too, in HyperFormula).
    let span = |a: Option<&Ast>| -> Option<(f64, f64)> {
        match a? {
            Ast::Range { start, end, .. } => Some(((end.col - start.col + 1) as f64, (end.row - start.row + 1) as f64)),
            Ast::ColRange { start, end, .. } => Some(((end.col - start.col + 1) as f64, f64::NAN)),
            Ast::RowRange { start, end, .. } => Some((f64::NAN, (end.row - start.row + 1) as f64)),
            _ => None,
        }
    };
    let (Some((lw, lh)), Some((rw, rh))) = (span(args.get(1)), span(args.get(2))) else { return ArraySize::error() };
    let vertical = lw == 1.0 && rh == lh;
    let horizontal = lh == 1.0 && rw == lw;
    if !vertical && !horizontal {
        return ArraySize::error();
    }
    if vertical {
        return ArraySize::new(rw, 1.0);
    }
    ArraySize::new(1.0, rh)
}

fn literal_dimension(a: &Ast) -> Option<f64> {
    match a {
        Ast::Number { v, .. } => Some(v.trunc()),
        Ast::Str { v, .. } => {
            let parsed = oc_schema::js::string_to_number(v);
            parsed.is_finite().then(|| parsed.trunc())
        }
        Ast::Plus { v, .. } => literal_number(v).map(f64::trunc),
        Ast::Minus { v, .. } => literal_number(v).map(|x| (-x).trunc()),
        Ast::Func { name, args, .. } if args.is_empty() && name == "TRUE" => Some(1.0),
        Ast::Func { name, args, .. } if args.is_empty() && name == "FALSE" => Some(0.0),
        _ => None,
    }
}

fn sequence_size(args: &[Ast]) -> ArraySize {
    if args.is_empty() || args.len() > 4 {
        return ArraySize::error();
    }
    if matches!(args[0], Ast::Empty { .. }) {
        return ArraySize::error();
    }
    let Some(rows) = literal_dimension(&args[0]) else { return ArraySize::error() };
    let cols = match args.get(1) {
        None | Some(Ast::Empty { .. }) => Some(1.0),
        Some(a) => literal_dimension(a),
    };
    let Some(cols) = cols else { return ArraySize::error() };
    let valid = |n: f64| n.is_finite() && n >= 1.0;
    if !valid(rows) || !valid(cols) {
        return ArraySize::error();
    }
    if rows > MAX_ROWS as f64 || cols > MAX_COLUMNS as f64 {
        return ArraySize::error();
    }
    ArraySize::new(cols, rows)
}
