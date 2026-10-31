// SPDX-License-Identifier: AGPL-3.0-only
//! Interpreter values (HyperFormula's `InterpreterValue`): scalars with typed numbers, errors, ranges.

use std::sync::Arc;

/// HyperFormula's detailed number types (`NumberType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NumType {
    #[default]
    Raw,
    Date,
    Time,
    DateTime,
    Currency,
    Percent,
}

/// Error kinds with their enGB literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorType {
    Div0,
    Na,
    Name,
    Num,
    Ref,
    Value,
    Cycle,
    Error,
    Spill,
    Lic,
}

impl ErrorType {
    pub fn literal(self) -> &'static str {
        match self {
            ErrorType::Div0 => "#DIV/0!",
            ErrorType::Na => "#N/A",
            ErrorType::Name => "#NAME?",
            ErrorType::Num => "#NUM!",
            ErrorType::Ref => "#REF!",
            ErrorType::Value => "#VALUE!",
            ErrorType::Cycle => "#CYCLE!",
            ErrorType::Error => "#ERROR!",
            ErrorType::Spill => "#SPILL!",
            ErrorType::Lic => "#LIC!",
        }
    }

    /// The enGB error mapping (`config.errorMapping`): upper-cased literal → type.
    pub fn from_literal(s: &str) -> Option<ErrorType> {
        Some(match s {
            "#DIV/0!" => ErrorType::Div0,
            "#N/A" => ErrorType::Na,
            "#NAME?" => ErrorType::Name,
            "#NUM!" => ErrorType::Num,
            "#REF!" => ErrorType::Ref,
            "#VALUE!" => ErrorType::Value,
            "#CYCLE!" => ErrorType::Cycle,
            "#ERROR!" => ErrorType::Error,
            "#SPILL!" => ErrorType::Spill,
            _ => return None,
        })
    }
}

/// A scalar interpreter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Empty,
    Num(f64, NumType),
    Str(Arc<str>),
    Bool(bool),
    Err(ErrorType),
}

impl Scalar {
    pub fn num(v: f64) -> Scalar {
        Scalar::Num(v, NumType::Raw)
    }
    pub fn str(s: impl Into<Arc<str>>) -> Scalar {
        Scalar::Str(s.into())
    }
    pub fn err(e: ErrorType) -> Scalar {
        Scalar::Err(e)
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Scalar::Num(..))
    }
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Scalar::Num(v, _) => Some(*v),
            _ => None,
        }
    }
    pub fn num_type(&self) -> NumType {
        match self {
            Scalar::Num(_, t) => *t,
            _ => NumType::Raw,
        }
    }
    /// `getRawValue`: drop the number subtype.
    pub fn raw(self) -> Scalar {
        match self {
            Scalar::Num(v, _) => Scalar::Num(v, NumType::Raw),
            o => o,
        }
    }
}

/// An absolute cell range on one sheet (inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AbsRange {
    pub sheet: u32,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl AbsRange {
    pub fn height(&self) -> usize {
        (self.r1 - self.r0 + 1) as usize
    }
    pub fn width(&self) -> usize {
        (self.c1 - self.c0 + 1) as usize
    }
    pub fn contains(&self, sheet: u32, r: u32, c: u32) -> bool {
        self.sheet == sheet && r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
}

/// HyperFormula's `SimpleRangeValue`: materialised data, plus the source range when it came from a reference
/// (`isAdHoc()` is `range.is_none()`).
#[derive(Debug, Clone, PartialEq)]
pub struct RangeVal {
    pub data: Vec<Vec<Scalar>>,
    pub range: Option<AbsRange>,
}

impl RangeVal {
    pub fn only_values(data: Vec<Vec<Scalar>>) -> RangeVal {
        RangeVal { data, range: None }
    }
    pub fn from_scalar(s: Scalar) -> RangeVal {
        RangeVal { data: vec![vec![s]], range: None }
    }
    pub fn height(&self) -> usize {
        self.data.len()
    }
    pub fn width(&self) -> usize {
        self.data.first().map_or(0, Vec::len)
    }
    pub fn is_adhoc(&self) -> bool {
        self.range.is_none()
    }
    /// Row-major values (`valuesFromTopLeftCorner`).
    pub fn values(&self) -> impl Iterator<Item = &Scalar> {
        self.data.iter().flatten()
    }
    /// `hasOnlyNumbers`: range-backed data counts typed numbers; ad-hoc data is checked with
    /// `typeof v === 'number'`, which typed (rich) numbers fail.
    pub fn has_only_numbers(&self) -> bool {
        if self.is_adhoc() {
            self.values().all(|v| matches!(v, Scalar::Num(_, NumType::Raw)))
        } else {
            self.values().all(Scalar::is_number)
        }
    }
    pub fn same_dimensions(&self, o: &RangeVal) -> bool {
        self.width() == o.width() && self.height() == o.height()
    }
}

/// Any interpreter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    S(Scalar),
    R(RangeVal),
}

impl Val {
    pub fn num(v: f64) -> Val {
        Val::S(Scalar::num(v))
    }
    pub fn typed(v: f64, t: NumType) -> Val {
        Val::S(Scalar::Num(v, t))
    }
    pub fn err(e: ErrorType) -> Val {
        Val::S(Scalar::Err(e))
    }
    pub fn str(s: impl Into<Arc<str>>) -> Val {
        Val::S(Scalar::Str(s.into()))
    }
    pub fn bool(b: bool) -> Val {
        Val::S(Scalar::Bool(b))
    }
    pub fn empty() -> Val {
        Val::S(Scalar::Empty)
    }
    pub fn as_err(&self) -> Option<ErrorType> {
        match self {
            Val::S(Scalar::Err(e)) => Some(*e),
            _ => None,
        }
    }
}

impl From<Scalar> for Val {
    fn from(s: Scalar) -> Val {
        Val::S(s)
    }
}

impl From<ErrorType> for Val {
    fn from(e: ErrorType) -> Val {
        Val::S(Scalar::Err(e))
    }
}

impl From<f64> for Val {
    fn from(v: f64) -> Val {
        Val::num(v)
    }
}

impl From<bool> for Val {
    fn from(v: bool) -> Val {
        Val::bool(v)
    }
}

impl From<RangeVal> for Val {
    fn from(r: RangeVal) -> Val {
        Val::R(r)
    }
}
