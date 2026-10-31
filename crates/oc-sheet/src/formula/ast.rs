// SPDX-License-Identifier: AGPL-3.0-only
//! Formula AST (HyperFormula's `Ast`), with absolute coordinates and the source whitespace kept so formulas
//! rewritten by structural edits unparse the way HyperFormula's `Unparser` prints them.

use super::value::ErrorType;

/// Which sheet a reference points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SheetRef {
    /// A sheet of the workbook (engine sheet id).
    Id(u32),
    /// A sheet name that does not exist (HyperFormula placeholder sheet): evaluates to `#REF!`.
    Missing(String),
}

/// A cell address. `row`/`col` are absolute (negative = deleted, prints `#REF!`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellRef {
    pub sheet: Option<SheetRef>,
    pub row: i64,
    pub col: i64,
    pub abs_row: bool,
    pub abs_col: bool,
}

impl CellRef {
    pub fn is_invalid(&self) -> bool {
        self.row < 0 || self.col < 0
    }
}

/// A column address (`A`, `$B`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColRef {
    pub sheet: Option<SheetRef>,
    pub col: i64,
    pub abs: bool,
}

/// A row address (`1`, `$2`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowRef {
    pub sheet: Option<SheetRef>,
    pub row: i64,
    pub abs: bool,
}

/// `RangeSheetReferenceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeSheet {
    /// `A1:B2`
    Relative,
    /// `Sheet1!A1:B2`
    StartAbsolute,
    /// `Sheet1!A1:Sheet1!B2`
    BothAbsolute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Concat,
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    Plus,
    Minus,
    Times,
    Div,
    Pow,
}

impl BinOp {
    pub fn token(self) -> &'static str {
        match self {
            BinOp::Concat => "&",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Gt => ">",
            BinOp::Lt => "<",
            BinOp::Ge => ">=",
            BinOp::Le => "<=",
            BinOp::Plus => "+",
            BinOp::Minus => "-",
            BinOp::Times => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
        }
    }
}

/// Leading whitespace of the node's first token (as in the source).
pub type Ws = Option<String>;

#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Empty { ws: Ws },
    Number { v: f64, ws: Ws },
    Str { v: String, ws: Ws },
    Cell { r: CellRef, ws: Ws },
    Range { start: CellRef, end: CellRef, sheet: RangeSheet, ws: Ws },
    ColRange { start: ColRef, end: ColRef, sheet: RangeSheet, ws: Ws },
    RowRange { start: RowRef, end: RowRef, sheet: RangeSheet, ws: Ws },
    Func { name: String, args: Vec<Ast>, ws: Ws, internal_ws: Ws },
    Named { name: String, ws: Ws },
    Plus { v: Box<Ast>, ws: Ws },
    Minus { v: Box<Ast>, ws: Ws },
    Percent { v: Box<Ast>, ws: Ws },
    Bin { op: BinOp, l: Box<Ast>, r: Box<Ast>, ws: Ws },
    Paren { e: Box<Ast>, ws: Ws, internal_ws: Ws },
    Array { rows: Vec<Vec<Ast>>, ws: Ws, internal_ws: Ws },
    Error { e: ErrorType, ws: Ws },
    ErrorRaw { raw: String, e: ErrorType, ws: Ws },
}

impl Ast {
    /// Visit every node (pre-order).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Ast)) {
        f(self);
        match self {
            Ast::Func { args, .. } => args.iter().for_each(|a| a.walk(f)),
            Ast::Plus { v, .. } | Ast::Minus { v, .. } | Ast::Percent { v, .. } => v.walk(f),
            Ast::Bin { l, r, .. } => {
                l.walk(f);
                r.walk(f);
            }
            Ast::Paren { e, .. } => e.walk(f),
            Ast::Array { rows, .. } => rows.iter().flatten().for_each(|a| a.walk(f)),
            _ => {}
        }
    }

    /// Visit every node mutably (post-order children first is not needed; pre-order).
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Ast)) {
        f(self);
        match self {
            Ast::Func { args, .. } => args.iter_mut().for_each(|a| a.walk_mut(f)),
            Ast::Plus { v, .. } | Ast::Minus { v, .. } | Ast::Percent { v, .. } => v.walk_mut(f),
            Ast::Bin { l, r, .. } => {
                l.walk_mut(f);
                r.walk_mut(f);
            }
            Ast::Paren { e, .. } => e.walk_mut(f),
            Ast::Array { rows, .. } => rows.iter_mut().flatten().for_each(|a| a.walk_mut(f)),
            _ => {}
        }
    }

    /// The node with parentheses stripped.
    pub fn unparen(&self) -> &Ast {
        match self {
            Ast::Paren { e, .. } => e.unparen(),
            o => o,
        }
    }
}
