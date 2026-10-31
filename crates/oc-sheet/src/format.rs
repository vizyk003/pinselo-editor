// SPDX-License-Identifier: AGPL-3.0-only
//! Display formatting of computed cell values (deterministic — the editor, exports and server agree).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::jsnum::{num_to_string, parse_num, to_exponential, to_fixed, to_precision};
use crate::model::{CellFormat, NumberFormat};

/// A computed cell value as the TS engine exposes it: `number | string | boolean | null | { error }`.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum CellValue {
    #[default]
    Null,
    Number(f64),
    String(String),
    Bool(bool),
    Error(String),
}

impl CellValue {
    pub fn to_json(&self) -> Value {
        match self {
            CellValue::Null => Value::Null,
            CellValue::Number(n) => oc_crdt::value::f64_to_json(*n),
            CellValue::String(s) => Value::String(s.clone()),
            CellValue::Bool(b) => Value::Bool(*b),
            CellValue::Error(e) => serde_json::json!({ "error": e }),
        }
    }
    pub fn from_json(v: &Value) -> CellValue {
        match v {
            Value::Null => CellValue::Null,
            Value::Bool(b) => CellValue::Bool(*b),
            Value::Number(n) => CellValue::Number(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => CellValue::String(s.clone()),
            Value::Object(m) => CellValue::Error(m.get("error").and_then(Value::as_str).unwrap_or("#ERROR!").to_string()),
            Value::Array(_) => CellValue::Null,
        }
    }
    pub fn is_empty_like(&self) -> bool {
        matches!(self, CellValue::Null) || matches!(self, CellValue::String(s) if s.is_empty())
    }
}

impl Serialize for CellValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for CellValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(CellValue::from_json(&Value::deserialize(d)?))
    }
}

/// HyperFormula's detailed number kinds (used by "auto" number formatting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ValueKind {
    #[default]
    Number,
    Date,
    Time,
    Datetime,
    Currency,
    Percent,
    Other,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueKind::Number => "number",
            ValueKind::Date => "date",
            ValueKind::Time => "time",
            ValueKind::Datetime => "datetime",
            ValueKind::Currency => "currency",
            ValueKind::Percent => "percent",
            ValueKind::Other => "other",
        }
    }
}

const MS_DAY: f64 = 86_400_000.0;
/// Excel/HyperFormula serial day 0 = 1899-12-30 (UTC): `Date.UTC(1899, 11, 30)`.
const EPOCH: f64 = -2_209_161_600_000.0;
const MAX_TIME: f64 = 8.64e15;

/// UTC date-time parts of a JS time value (`None` = Invalid Date).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateParts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub ms: u32,
}

/// Days since 1970-01-01 → (year, month 1-12, day 1-31) (proleptic Gregorian).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month 1-12, day) → days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `new Date(t)` UTC getters.
pub fn time_parts(t: f64) -> Option<DateParts> {
    if !t.is_finite() || t.abs() > MAX_TIME {
        return None;
    }
    let t = t.trunc() as i64;
    let days = t.div_euclid(86_400_000);
    let ms_in_day = t.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    Some(DateParts {
        year,
        month,
        day,
        hour: (ms_in_day / 3_600_000) as u32,
        minute: (ms_in_day / 60_000 % 60) as u32,
        second: (ms_in_day / 1000 % 60) as u32,
        ms: (ms_in_day % 1000) as u32,
    })
}

/// `serialToDate(serial)` as a JS time value (ms since the Unix epoch).
pub fn serial_to_time(serial: f64) -> f64 {
    EPOCH + oc_schema::js::round(serial * MS_DAY)
}

/// `dateToSerial(d)` for a JS time value.
pub fn time_to_serial(t: f64) -> f64 {
    (t - EPOCH) / MS_DAY
}

fn pad(n: u32) -> String {
    format!("{n:02}")
}

fn js_year(y: i64) -> String {
    // `String(getUTCFullYear())` (no zero padding).
    y.to_string()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimePart {
    None,
    Only,
    Both,
}

fn fmt_date(serial: f64, time: TimePart) -> String {
    let Some(d) = time_parts(serial_to_time(serial)) else {
        return match time {
            TimePart::None => "NaN-NaN-NaN".into(),
            TimePart::Only => "NaN:NaN".into(),
            TimePart::Both => "NaN-NaN-NaN NaN:NaN".into(),
        };
    };
    let date = format!("{}-{}-{}", js_year(d.year), pad(d.month), pad(d.day));
    let hm = format!("{}:{}", pad(d.hour), pad(d.minute));
    match time {
        TimePart::None => date,
        TimePart::Only => hm,
        TimePart::Both => format!("{date} {hm}"),
    }
}

/// JS `str.replace(/\B(?=(\d{3})+(?!\d))/g, ',')`.
fn group_thousands(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for i in 0..b.len() {
        if i > 0 {
            let boundary = is_word(b[i - 1]) != is_word(b[i]);
            if !boundary {
                // (\d{3})+ followed by a non-digit (or the end).
                let mut j = i;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let run = j - i;
                if run > 0 && run % 3 == 0 {
                    out.push(',');
                }
            }
        }
        out.push(b[i]);
    }
    out
}

fn dp_usize(dp: f64) -> usize {
    // `toFixed` accepts 0..=100 (it throws outside; the UI never stores those).
    if dp.is_nan() { 0 } else { dp.trunc().clamp(0.0, 100.0) as usize }
}

fn grouped(n: f64, dp: f64) -> String {
    let s = to_fixed(n.abs(), dp_usize(dp));
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s.clone(), None),
    };
    let g = group_thousands(&int);
    let neg = n < 0.0 && parse_num(&s) != 0.0;
    format!("{}{}{}", if neg { "-" } else { "" }, g, frac.filter(|f| !f.is_empty()).map(|f| format!(".{f}")).unwrap_or_default())
}

/// "General" number display: up to 10 significant digits, no trailing zeros, exponent for extremes.
pub fn general_number(n: f64) -> String {
    if !n.is_finite() {
        return num_to_string(n);
    }
    if n == 0.0 {
        return "0".into();
    }
    let a = n.abs();
    if a >= 1e15 || a < 1e-9 {
        let e = to_exponential(n, 4);
        // `.replace(/\.?0+e/, 'e')`
        return strip_exp_zeros(&e);
    }
    num_to_string(parse_num(&to_precision(n, 10)))
}

fn strip_exp_zeros(s: &str) -> String {
    let Some(epos) = s.find('e') else { return s.to_string() };
    let head = &s[..epos];
    let trimmed = head.trim_end_matches('0');
    if trimmed.len() == head.len() {
        return s.to_string();
    }
    let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
    format!("{trimmed}{}", &s[epos..])
}

/// `formatValue(v, fmt, kind)`.
pub fn format_value(v: &CellValue, fmt: Option<&CellFormat>, kind: ValueKind) -> String {
    let v = match v {
        CellValue::Null => return String::new(),
        CellValue::Error(e) => return e.clone(),
        CellValue::Bool(b) => return if *b { "TRUE".into() } else { "FALSE".into() },
        CellValue::String(s) => return s.clone(),
        CellValue::Number(n) => *n,
    };
    let nf = fmt.and_then(|f| f.nf).unwrap_or(NumberFormat::Auto);
    let dp = fmt.and_then(|f| f.dp);
    match nf {
        NumberFormat::Number => grouped(v, dp.unwrap_or(2.0)),
        NumberFormat::Percent => format!("{}%", grouped(v * 100.0, dp.unwrap_or(0.0))),
        NumberFormat::Currency => {
            let s = grouped(v.abs(), dp.unwrap_or(2.0));
            let cur = fmt.and_then(|f| f.cur.as_deref()).unwrap_or("$");
            format!("{}{cur}{s}", if v < 0.0 { "-" } else { "" })
        }
        NumberFormat::Date => fmt_date(v, TimePart::None),
        NumberFormat::Datetime => fmt_date(v, TimePart::Both),
        NumberFormat::Text => general_number(v),
        NumberFormat::Auto => match kind {
            ValueKind::Date => fmt_date(v, TimePart::None),
            ValueKind::Datetime => fmt_date(v, TimePart::Both),
            ValueKind::Time => fmt_date(v, TimePart::Only),
            ValueKind::Percent => format!("{}%", general_number(parse_num(&to_precision(v * 100.0, 12)))),
            ValueKind::Currency => grouped(v, 2.0),
            _ => match dp {
                Some(d) => grouped(v, d),
                None => general_number(v),
            },
        },
    }
}

/// Default horizontal alignment (`autoAlign`): a stored `align` as is (any non-empty string, like TS),
/// else numbers right, booleans/errors centre, text left.
pub fn auto_align<'a>(v: &CellValue, fmt: Option<&'a CellFormat>) -> &'a str {
    if let Some(a) = fmt.and_then(|f| f.align.as_deref()).filter(|a| !a.is_empty()) {
        return a;
    }
    match v {
        CellValue::Number(_) if !fmt.is_some_and(CellFormat::is_text) => "right",
        CellValue::Bool(_) | CellValue::Error(_) => "center",
        _ => "left",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general() {
        assert_eq!(general_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(general_number(1e15), "1e+15");
        assert_eq!(general_number(1.5e-10), "1.5e-10");
        assert_eq!(general_number(123456789012.0), "123456789000");
        assert_eq!(grouped(-1234567.891, 2.0), "-1,234,567.89");
        assert_eq!(grouped(-0.001, 2.0), "0.00");
        assert_eq!(fmt_date(45306.0, TimePart::None), "2024-01-15");
    }
}
