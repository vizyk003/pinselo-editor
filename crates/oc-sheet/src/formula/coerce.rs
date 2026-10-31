// SPDX-License-Identifier: AGPL-3.0-only
//! Port of HyperFormula's `ArithmeticHelper` (coercions, comparisons, typed arithmetic, wildcard matching)
//! and `NumberLiteralHelper`, for the engine config (`.` decimal separator, no thousand separator,
//! currency symbols `$ € £ ¥`, case- and accent-insensitive, wildcards on, regexes off, whole-cell match).

use std::cmp::Ordering;
use std::sync::Arc;

use unicode_normalization::UnicodeNormalization;

use super::datetime::date_string_to_number;
use super::value::{ErrorType, NumType, Scalar};

pub const EPS: f64 = 1e-13;
pub const CURRENCY_SYMBOLS: [&str; 4] = ["$", "€", "£", "¥"];

/// `^[+-]?((\.\d+)|(\d+(\.\d*)?))([eE][+-]?\d+)?$`
pub fn is_number_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+') | Some(b'-')) {
        i += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == st {
            return false;
        }
    } else {
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == st {
            return false;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    if matches!(b.get(i), Some(b'e') | Some(b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+') | Some(b'-')) {
            i += 1;
        }
        let st = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == st {
            return false;
        }
    }
    i == b.len()
}

/// `numericStringToMaybeNumber`.
pub fn numeric_string_to_number(s: &str) -> Option<f64> {
    if !is_number_literal(s) {
        return None;
    }
    let v = oc_schema::js::string_to_number(s);
    (!v.is_nan()).then_some(v)
}

fn js_trim(s: &str) -> &str {
    oc_schema::js::trim(s)
}

/// `currencyMatcher` (the arithmetic variant trims the remainder).
pub fn currency_match(token: &str, trim_rest: bool) -> Option<(&'static str, String)> {
    for c in CURRENCY_SYMBOLS {
        if let Some(r) = token.strip_prefix(c) {
            return Some((c, if trim_rest { js_trim(r) } else { r }.to_string()));
        }
        if let Some(r) = token.strip_suffix(c) {
            return Some((c, if trim_rest { js_trim(r) } else { r }.to_string()));
        }
    }
    None
}

fn percent_number(s: &str) -> Option<Scalar> {
    let t = js_trim(s);
    let body = t.strip_suffix('%')?;
    let n = numeric_string_to_number(js_trim(body))?;
    Some(Scalar::Num(n / 100.0, NumType::Percent))
}

fn currency_number(s: &str) -> Option<Scalar> {
    let (_, rest) = currency_match(js_trim(s), true)?;
    let n = numeric_string_to_number(&rest)?;
    Some(Scalar::Num(n, NumType::Currency))
}

/// `coerceNonDateScalarToMaybeNumber`.
pub fn non_date_to_number(s: &Scalar) -> Option<Scalar> {
    match s {
        Scalar::Empty => Some(Scalar::num(0.0)),
        Scalar::Str(t) => {
            if t.is_empty() {
                return Some(Scalar::num(0.0));
            }
            if let Some(p) = percent_number(t) {
                return Some(p);
            }
            if let Some(c) = currency_number(t) {
                return Some(c);
            }
            numeric_string_to_number(js_trim(t)).map(Scalar::num)
        }
        Scalar::Num(..) => Some(s.clone()),
        Scalar::Bool(b) => Some(Scalar::num(if *b { 1.0 } else { 0.0 })),
        Scalar::Err(_) => None,
    }
}

/// `coerceToMaybeNumber`.
pub fn to_maybe_number(s: &Scalar) -> Option<Scalar> {
    non_date_to_number(s).or_else(|| match s {
        Scalar::Str(t) => date_string_to_number(t),
        _ => None,
    })
}

/// `coerceScalarToNumberOrError` (keeps the number subtype).
pub fn to_number_or_error(s: &Scalar) -> Scalar {
    if let Scalar::Err(_) = s {
        return s.clone();
    }
    to_maybe_number(s).unwrap_or(Scalar::Err(ErrorType::Value))
}

/// `coerceScalarToString`.
pub fn to_string(s: &Scalar) -> Scalar {
    match s {
        Scalar::Err(_) | Scalar::Str(_) => s.clone(),
        Scalar::Empty => Scalar::str(""),
        Scalar::Num(v, _) => Scalar::Str(Arc::from(oc_schema::js::num_to_string(*v))),
        Scalar::Bool(b) => Scalar::str(if *b { "TRUE" } else { "FALSE" }),
    }
}

/// `coerceScalarToBoolean` (`None` = not coercible).
pub fn to_boolean(s: &Scalar) -> Option<Scalar> {
    match s {
        Scalar::Err(_) | Scalar::Bool(_) => Some(s.clone()),
        Scalar::Empty => Some(Scalar::Bool(false)),
        Scalar::Num(v, _) => Some(Scalar::Bool(*v != 0.0)),
        Scalar::Str(t) => {
            let u = t.to_uppercase();
            match u.as_str() {
                "TRUE" => Some(Scalar::Bool(true)),
                "FALSE" | "" => Some(Scalar::Bool(false)),
                _ => None,
            }
        }
    }
}

/// `floatCmp` with the config epsilon.
pub fn float_cmp(left: f64, right: f64) -> i32 {
    let m = 1.0 + EPS;
    if (right >= 0.0 && left * m >= right && left <= right * m) || (right <= 0.0 && left * m <= right && left >= right * m) {
        0
    } else if left > right {
        1
    } else {
        -1
    }
}

fn number_cmp(a: f64, b: f64) -> i32 {
    if a > b {
        1
    } else if a < b {
        -1
    } else {
        0
    }
}

fn type_ord(s: &Scalar) -> i32 {
    match s {
        Scalar::Empty => 0,
        Scalar::Num(..) => 1,
        Scalar::Str(_) => 2,
        Scalar::Bool(_) => 3,
        Scalar::Err(_) => 4,
    }
}

fn coerce_empty_to(other: &Scalar) -> Scalar {
    match other {
        Scalar::Str(_) => Scalar::str(""),
        Scalar::Num(..) => Scalar::num(0.0),
        Scalar::Bool(_) => Scalar::Bool(false),
        _ => Scalar::Empty,
    }
}

/// `compare(left, right)` → -1 / 0 / 1.
pub fn compare(left: &Scalar, right: &Scalar) -> i32 {
    if matches!(left, Scalar::Str(_)) || matches!(right, Scalar::Str(_)) {
        let l = match left {
            Scalar::Str(t) => date_string_to_number(t),
            o => Some(o.clone()),
        };
        let r = match right {
            Scalar::Str(t) => date_string_to_number(t),
            o => Some(o.clone()),
        };
        if let (Some(Scalar::Num(a, _)), Some(Scalar::Num(b, _))) = (&l, &r) {
            return float_cmp(*a, *b);
        }
    }
    let (l, r) = match (left, right) {
        (Scalar::Empty, o) => (coerce_empty_to(o), o.clone()),
        (o, Scalar::Empty) => (o.clone(), coerce_empty_to(o)),
        (a, b) => (a.clone(), b.clone()),
    };
    match (&l, &r) {
        (Scalar::Str(a), Scalar::Str(b)) => string_cmp(a, b),
        (Scalar::Bool(a), Scalar::Bool(b)) => number_cmp(f64::from(u8::from(*a)), f64::from(u8::from(*b))),
        (Scalar::Num(a, _), Scalar::Num(b, _)) => float_cmp(*a, *b),
        (Scalar::Empty, Scalar::Empty) => 0,
        _ => number_cmp(type_ord(&l) as f64, type_ord(&r) as f64),
    }
}

/// ICU root primary weights for printable ASCII (`Intl.Collator('en', { sensitivity: 'base' })`), indexed
/// from 0x20; tab and newline sort before space.
const ASCII_RANK: [u8; 95] = [
    2, 8, 12, 24, 34, 25, 23, 11, 13, 14, 20, 28, 5, 4, 10, 21, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 7, 6, 29, 30, 31, 9, 19, 45, 46,
    47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 15, 22, 16, 27, 3, 26, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 17, 32, 18, 33,
];

fn weight(c: char) -> u32 {
    match c {
        '\t' => 0,
        '\n' => 1,
        ' '..='~' => ASCII_RANK[c as usize - 0x20] as u32,
        _ => 1000 + c as u32,
    }
}

/// Base-sensitivity collation (approximation of ICU: accents and case ignored, punctuation significant).
pub fn string_cmp(a: &str, b: &str) -> i32 {
    let na = normalize_base(a);
    let nb = normalize_base(b);
    let mut ia = na.chars();
    let mut ib = nb.chars();
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(x), Some(y)) => match weight(x).cmp(&weight(y)) {
                Ordering::Less => return -1,
                Ordering::Greater => return 1,
                Ordering::Equal => {}
            },
        }
    }
}

/// Lower-case + NFD with combining diacritics (U+0300–U+036F) removed (`normalizeString` for a
/// case- and accent-insensitive config).
pub fn normalize_base(s: &str) -> String {
    s.to_lowercase().nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect()
}

// ------------------------------------------------------------------ complex numbers

fn is_ws(c: char) -> bool {
    oc_schema::js::is_js_space(c)
}

/// One term of the complex literal grammar: `[\d.,]+(e[+-]?\d+)?\s*[ij]?` or a bare `[ij]`.
/// Returns (number text or None for a bare unit, unit, end index).
fn complex_term(s: &[char], mut i: usize, trailing_ws: bool) -> Option<(Option<String>, String, usize)> {
    let st = i;
    while i < s.len() && (s[i].is_ascii_digit() || s[i] == '.' || s[i] == ',') {
        i += 1;
    }
    if i == st {
        return match s.get(i) {
            Some(c @ ('i' | 'j')) => Some((None, c.to_string(), i + 1)),
            _ => None,
        };
    }
    if s.get(i) == Some(&'e') {
        let mut k = i + 1;
        if matches!(s.get(k), Some('+') | Some('-')) {
            k += 1;
        }
        let ds = k;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > ds {
            i = k;
        }
    }
    let num: String = s[st..i].iter().collect();
    let mut j = i;
    while j < s.len() && is_ws(s[j]) {
        j += 1;
    }
    if let Some(c @ ('i' | 'j')) = s.get(j) {
        return Some((Some(num), c.to_string(), j + 1));
    }
    // `\s*([ij]?)` with an empty unit consumes the whitespace only when nothing else follows it.
    Some((Some(num), String::new(), if trailing_ws { j } else { i }))
}

fn complex_token(text: &str, unit: &str) -> Result<(f64, f64), ErrorType> {
    let v = match non_date_to_number(&Scalar::str(text)) {
        Some(Scalar::Num(v, _)) => v,
        _ => return Err(ErrorType::Num),
    };
    Ok(if unit.is_empty() { (v, 0.0) } else { (0.0, v) })
}

/// `coerceStringToComplex` (HyperFormula's `complexParsingRegexp`).
pub fn string_to_complex(arg: &str) -> Result<(f64, f64), ErrorType> {
    let s: Vec<char> = arg.chars().collect();
    let mut i = 0;
    let skip = |s: &[char], mut i: usize| {
        while i < s.len() && is_ws(s[i]) {
            i += 1;
        }
        i
    };
    i = skip(&s, i);
    let sign1 = match s.get(i) {
        Some(c @ ('+' | '-')) => {
            i += 1;
            c.to_string()
        }
        _ => String::new(),
    };
    i = skip(&s, i);
    let (num1, unit1, e1) = complex_term(&s, i, true).ok_or(ErrorType::Num)?;
    i = skip(&s, e1);
    let v1 = match &num1 {
        None => {
            if sign1 == "-" {
                (0.0, -1.0)
            } else {
                (0.0, 1.0)
            }
        }
        Some(n) => complex_token(&format!("{sign1}{n}"), &unit1)?,
    };
    if i == s.len() {
        return Ok(v1);
    }
    let op = match s.get(i) {
        Some(c @ ('+' | '-')) => *c,
        _ => return Err(ErrorType::Num),
    };
    i = skip(&s, i + 1);
    let sign2 = match s.get(i) {
        Some(c @ ('+' | '-')) => {
            i += 1;
            c.to_string()
        }
        _ => String::new(),
    };
    i = skip(&s, i);
    let (num2, unit2, e2) = complex_term(&s, i, true).ok_or(ErrorType::Num)?;
    if e2 != s.len() {
        return Err(ErrorType::Num);
    }
    let v2 = match &num2 {
        None => {
            if sign2 == "-" {
                (0.0, -1.0)
            } else {
                (0.0, 1.0)
            }
        }
        Some(n) => complex_token(&format!("{sign2}{n}"), &unit2)?,
    };
    // `match[5] !== '' || match[13] === ''`: the first term must be a plain number (a bare unit leaves
    // group 5 undefined, which also fails), the second imaginary.
    let unit1_nonempty = num1.is_none() || !unit1.is_empty();
    let unit2_empty = num2.is_some() && unit2.is_empty();
    if unit1_nonempty || unit2_empty {
        return Err(ErrorType::Num);
    }
    Ok(if op == '+' { (v1.0 + v2.0, v1.1 + v2.1) } else { (v1.0 - v2.0, v1.1 - v2.1) })
}

/// `coerceScalarToComplex`.
pub fn to_complex(arg: &Scalar) -> Result<(f64, f64), ErrorType> {
    match arg {
        Scalar::Err(e) => Err(*e),
        Scalar::Empty => Ok((0.0, 0.0)),
        Scalar::Num(v, _) => Ok((*v, 0.0)),
        Scalar::Str(s) => string_to_complex(s),
        Scalar::Bool(_) => Err(ErrorType::Num),
    }
}

/// `coerceComplexToString` (`None` = `#NUM!` for non-finite parts).
pub fn complex_to_string(re: f64, im: f64, symb: &str) -> Option<String> {
    if !re.is_finite() || !im.is_finite() {
        return None;
    }
    let n = oc_schema::js::num_to_string;
    if im == 0.0 {
        return Some(n(re));
    }
    let im_str = format!("{}{symb}", if im == -1.0 || im == 1.0 { String::new() } else { n(im.abs()) });
    if re == 0.0 {
        return Some(format!("{}{im_str}", if im < 0.0 { "-" } else { "" }));
    }
    Some(format!("{}{}{im_str}", n(re), if im < 0.0 { "-" } else { "+" }))
}

// ------------------------------------------------------------------ typed arithmetic

fn additive_type(l: NumType, r: NumType) -> NumType {
    use NumType::*;
    if l == Raw {
        return r;
    }
    if r == Raw {
        return l;
    }
    if matches!(l, DateTime | Date) && matches!(r, DateTime | Date) {
        return Raw;
    }
    if l == Time && matches!(r, Date | DateTime) {
        return DateTime;
    }
    if r == Time && matches!(l, Date | DateTime) {
        return DateTime;
    }
    l
}

fn multiplicative_type(l: NumType, r: NumType) -> NumType {
    use NumType::*;
    let l = if l == Percent { Raw } else { l };
    let r = if r == Percent { Raw } else { r };
    if l == Raw {
        return r;
    }
    if r == Raw {
        return l;
    }
    Raw
}

pub fn add_raw(left: f64, right: f64) -> f64 {
    let ret = left + right;
    if ret.abs() < EPS * left.abs() { 0.0 } else { ret }
}

pub fn add(l: f64, lt: NumType, r: f64, rt: NumType) -> Scalar {
    Scalar::Num(add_raw(l, r), additive_type(lt, rt))
}

pub fn subtract(l: f64, lt: NumType, r: f64, rt: NumType) -> Scalar {
    let mut ret = l - r;
    if ret.abs() < EPS * l.abs() {
        ret = 0.0;
    }
    Scalar::Num(ret, additive_type(lt, rt))
}

pub fn multiply(l: f64, lt: NumType, r: f64, rt: NumType) -> Scalar {
    Scalar::Num(l * r, multiplicative_type(lt, rt))
}

pub fn divide(l: f64, lt: NumType, r: f64, rt: NumType) -> Scalar {
    if r == 0.0 {
        return Scalar::Err(ErrorType::Div0);
    }
    Scalar::Num(l / r, multiplicative_type(lt, rt))
}

/// `nonstrictadd`.
pub fn nonstrict_add(left: &Scalar, right: &Scalar) -> Scalar {
    match (left, right) {
        (Scalar::Err(_), _) => left.clone(),
        (_, Scalar::Err(_)) => right.clone(),
        (Scalar::Num(a, NumType::Raw), Scalar::Num(b, NumType::Raw)) => Scalar::num(add_raw(*a, *b)),
        (Scalar::Num(_, NumType::Raw), _) => left.clone(),
        (_, Scalar::Num(_, NumType::Raw)) => right.clone(),
        _ => Scalar::num(0.0),
    }
}

/// `isNumberOverflow`.
pub fn is_overflow(v: f64) -> bool {
    !v.is_finite()
}

// ------------------------------------------------------------------ wildcard patterns

#[derive(Debug, Clone, PartialEq)]
enum Pat {
    Lit(char),
    /// `.*`
    Star,
    /// `.?` (HyperFormula translates `?` to an optional character)
    Opt,
}

const ESCAPED: [char; 17] = ['{', '}', '[', ']', '(', ')', '<', '>', '=', '.', '+', '-', ',', '\\', '$', '^', '!'];

/// A compiled criterion pattern (`buildRegex` with wildcards).
#[derive(Debug, Clone)]
pub struct Pattern {
    pat: Vec<Pat>,
    whole: bool,
}

impl Pattern {
    pub fn new(pattern: &str, whole: bool) -> Pattern {
        let p: Vec<char> = normalize_base(pattern).chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < p.len() {
            let c = p[i];
            if c == '~' {
                if i == p.len() - 1 {
                    out.push(Pat::Lit('~'));
                    i += 1;
                    continue;
                }
                out.push(Pat::Lit(p[i + 1]));
                i += 2;
                continue;
            }
            out.push(match c {
                '*' => Pat::Star,
                '?' => Pat::Opt,
                c => Pat::Lit(c),
            });
            i += 1;
        }
        Pattern { pat: out, whole }
    }

    fn match_at(&self, pi: usize, s: &[char], si: usize) -> Option<usize> {
        // Returns the end index of a match of pat[pi..] starting at s[si..] (whole: must reach the end).
        if pi == self.pat.len() {
            return if !self.whole || si == s.len() { Some(si) } else { None };
        }
        let dot = |c: char| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
        match &self.pat[pi] {
            Pat::Lit(c) => (si < s.len() && s[si] == *c).then(|| self.match_at(pi + 1, s, si + 1)).flatten(),
            Pat::Opt => {
                if si < s.len()
                    && dot(s[si])
                    && let Some(e) = self.match_at(pi + 1, s, si + 1)
                {
                    return Some(e);
                }
                self.match_at(pi + 1, s, si)
            }
            Pat::Star => {
                let mut end = si;
                while end < s.len() && dot(s[end]) {
                    end += 1;
                }
                let mut k = end as i64;
                while k >= si as i64 {
                    if let Some(e) = self.match_at(pi + 1, s, k as usize) {
                        return Some(e);
                    }
                    k -= 1;
                }
                None
            }
        }
    }

    /// Whole-cell match against a (normalised) string.
    pub fn matches(&self, text: &str) -> bool {
        let s: Vec<char> = normalize_base(text).chars().collect();
        self.match_at(0, &s, 0).is_some()
    }

    /// First match index (in chars) in `text` as given (not normalised), like `regexp.exec(text).index`.
    pub fn search(&self, text: &str) -> Option<usize> {
        let s: Vec<char> = text.chars().collect();
        (0..=s.len()).find(|&i| self.match_at(0, &s, i).is_some())
    }
}

/// `requiresRegex`: the pattern has a wildcard.
pub fn requires_regex(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// `eqMatcherFunction`.
pub fn eq_matcher(pattern: &str) -> impl Fn(&Scalar) -> bool + use<> {
    let p = Pattern::new(pattern, true);
    move |v| matches!(v, Scalar::Str(s) if p.matches(s))
}

/// `neqMatcherFunction`.
pub fn neq_matcher(pattern: &str) -> impl Fn(&Scalar) -> bool + use<> {
    let p = Pattern::new(pattern, true);
    move |v| !matches!(v, Scalar::Str(s) if p.matches(s))
}

/// Characters the regex escaper treats specially (kept for documentation parity).
pub fn needs_escape(c: char) -> bool {
    ESCAPED.contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert!(is_number_literal("1.5e3"));
        assert!(is_number_literal(".5"));
        assert!(is_number_literal("5."));
        assert!(!is_number_literal("1,000"));
        assert!(!is_number_literal("e3"));
        assert_eq!(to_maybe_number(&Scalar::str("12%")), Some(Scalar::Num(0.12, NumType::Percent)));
        assert_eq!(to_maybe_number(&Scalar::str(" $ 5 ")), Some(Scalar::Num(5.0, NumType::Currency)));
    }

    #[test]
    fn wildcards() {
        assert!(Pattern::new("a*", true).matches("Apple"));
        assert!(Pattern::new("p?ar", true).matches("pear"));
        assert!(Pattern::new("p?ar", true).matches("par"));
        assert!(!Pattern::new("p?ar", true).matches("pxxar"));
        assert!(Pattern::new("~*", true).matches("*"));
        assert_eq!(string_cmp("apple", "Banana"), -1);
        assert_eq!(string_cmp("é", "E"), 0);
    }
}
