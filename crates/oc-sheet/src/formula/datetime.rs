// SPDX-License-Identifier: AGPL-3.0-only
//! Port of HyperFormula's `DateTimeHelper` + `defaultParseToDateTime` with the engine config
//! (`dateFormats: YYYY-MM-DD, YYYY/MM/DD, DD/MM/YYYY, DD.MM.YYYY`, `timeFormats: hh:mm, hh:mm:ss`,
//! null date 1899-12-30, `leapYear1900: false`, `nullYear: 30`).

use super::value::{NumType, Scalar};

const NUM_DAYS: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const PREF_SUM_DAYS: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

pub const DATE_FORMATS: [&str; 4] = ["YYYY-MM-DD", "YYYY/MM/DD", "DD/MM/YYYY", "DD.MM.YYYY"];
pub const TIME_FORMATS: [&str; 2] = ["hh:mm", "hh:mm:ss"];
const NULL_YEAR: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleDate {
    pub year: f64,
    pub month: f64,
    pub day: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleTime {
    pub hours: f64,
    pub minutes: f64,
    pub seconds: f64,
}

fn leap_years_count(year: f64) -> f64 {
    (year / 4.0).floor() - (year / 100.0).floor() + (year / 400.0).floor()
}

pub fn is_leap_year(year: f64) -> bool {
    if year % 4.0 != 0.0 {
        false
    } else if year % 100.0 != 0.0 {
        true
    } else {
        year % 400.0 == 0.0
    }
}

fn date_to_number_from_zero(d: &SimpleDate) -> f64 {
    let m = (d.month as i64 - 1).clamp(0, 11) as usize;
    365.0 * d.year + PREF_SUM_DAYS[m] as f64 + d.day - 1.0
        + if d.month <= 2.0 { leap_years_count(d.year - 1.0) } else { leap_years_count(d.year) }
}

/// Absolute day number of the null date (1899-12-30).
fn min_date_abs() -> f64 {
    date_to_number_from_zero(&SimpleDate { year: 1899.0, month: 12.0, day: 30.0 })
}

/// `relativeNumberToAbsoluteNumber` (no 1900 leap-year bug in this config).
pub fn relative_to_absolute(arg: f64) -> f64 {
    arg + min_date_abs()
}

/// `getEpochYearZero`: two-digit-ish years below this get it added by DATE (1900).
pub const EPOCH_YEAR_ZERO: f64 = 1900.0;

pub fn date_to_number(d: &SimpleDate) -> f64 {
    date_to_number_from_zero(d) - min_date_abs()
}

pub fn max_date_value() -> f64 {
    date_to_number(&SimpleDate { year: 9999.0, month: 12.0, day: 31.0 })
}

/// `getWithinBounds`.
pub fn within_bounds(day: f64) -> Option<f64> {
    (day <= max_date_value() && day >= 0.0).then_some(day)
}

fn day_to_month(day_of_year: f64) -> usize {
    let mut month = 0;
    if PREF_SUM_DAYS[month + 6] as f64 <= day_of_year {
        month += 6;
    }
    if PREF_SUM_DAYS[month + 3] as f64 <= day_of_year {
        month += 3;
    }
    if PREF_SUM_DAYS[month + 2] as f64 <= day_of_year {
        month += 2;
    } else if PREF_SUM_DAYS[month + 1] as f64 <= day_of_year {
        month += 1;
    }
    month
}

pub fn number_to_simple_date(arg: f64) -> SimpleDate {
    let date_number = arg.floor() + min_date_abs();
    let mut year = (date_number / 365.2425).floor();
    if date_to_number_from_zero(&SimpleDate { year: year + 1.0, month: 1.0, day: 1.0 }) <= date_number {
        year += 1.0;
    } else if date_to_number_from_zero(&SimpleDate { year: year - 1.0, month: 1.0, day: 1.0 }) > date_number {
        year -= 1.0;
    }
    let day_of_year = date_number - date_to_number_from_zero(&SimpleDate { year, month: 1.0, day: 1.0 });
    let leap = is_leap_year(year);
    let month = day_to_month(day_of_year - if leap && day_of_year >= 59.0 { 1.0 } else { 0.0 });
    let day = day_of_year - PREF_SUM_DAYS[month] as f64 - if leap && month > 1 { 1.0 } else { 0.0 };
    SimpleDate { year, month: month as f64 + 1.0, day: day + 1.0 }
}

fn js_round(x: f64) -> f64 {
    oc_schema::js::round(x)
}

fn round_to_epsilon(arg: f64, eps: f64) -> f64 {
    js_round(arg * eps) / eps
}

/// `numberToSimpleTime` (hours may reach 24 for values just below 1).
pub fn number_to_simple_time(arg: f64) -> SimpleTime {
    let secs_total = arg * 24.0 * 60.0 * 60.0;
    let seconds = round_to_epsilon(secs_total % 60.0, 100_000.0) % 60.0;
    let as_minutes = (secs_total - seconds) / 60.0;
    let minutes = js_round(as_minutes % 60.0) % 60.0;
    let as_hours = (as_minutes - minutes) / 60.0;
    let hours = js_round(as_hours);
    SimpleTime { hours, minutes, seconds }
}

pub fn number_to_simple_date_time(arg: f64) -> (SimpleDate, SimpleTime) {
    let mut time = number_to_simple_time(arg % 1.0);
    let carry = (time.hours / 24.0).floor();
    time.hours %= 24.0;
    (number_to_simple_date(arg.floor() + carry), time)
}

pub fn time_to_number(t: &SimpleTime) -> f64 {
    ((t.seconds / 60.0 + t.minutes) / 60.0 + t.hours) / 24.0
}

pub fn round_to_nearest_second(arg: f64) -> f64 {
    js_round(arg * 3600.0 * 24.0) / (3600.0 * 24.0)
}

pub fn is_valid_date(d: &SimpleDate) -> bool {
    if d.year.is_nan() || d.month.is_nan() || d.day.is_nan() {
        return false;
    }
    if d.day != js_round(d.day) || d.month != js_round(d.month) || d.year != js_round(d.year) {
        return false;
    }
    if d.year < 1582.0 || d.month < 1.0 || d.month > 12.0 || d.day < 1.0 {
        return false;
    }
    if is_leap_year(d.year) && d.month == 2.0 {
        return d.day <= 29.0;
    }
    d.day <= NUM_DAYS[d.month as usize - 1] as f64
}

pub fn days_in_month(year: f64, month: f64) -> f64 {
    if is_leap_year(year) && month == 2.0 { 29.0 } else { NUM_DAYS[(month as usize).clamp(1, 12) - 1] as f64 }
}

pub fn end_of_month(d: &SimpleDate) -> SimpleDate {
    SimpleDate { year: d.year, month: d.month, day: days_in_month(d.year, d.month) }
}

pub fn offset_month(d: &SimpleDate, offset: f64) -> SimpleDate {
    let total = 12.0 * d.year + d.month - 1.0 + offset;
    SimpleDate { year: (total / 12.0).floor(), month: total.rem_euclid(12.0).trunc() + 1.0, day: d.day }
}

pub fn truncate_day_in_month(d: &SimpleDate) -> SimpleDate {
    // Uses the non-leap table (HyperFormula quirk).
    SimpleDate { year: d.year, month: d.month, day: d.day.min(NUM_DAYS[(d.month as usize).clamp(1, 12) - 1] as f64) }
}

pub fn leap_years_count_pub(year: f64) -> f64 {
    leap_years_count(year)
}

pub fn count_leap_days(d: &SimpleDate) -> f64 {
    if d.month > 2.0 || (d.month == 2.0 && d.day >= 29.0) { leap_years_count(d.year) } else { leap_years_count(d.year - 1.0) }
}

pub fn year_length_for_basis(start: &SimpleDate, end: &SimpleDate) -> f64 {
    if start.year != end.year {
        if start.year + 1.0 != end.year || start.month < end.month || (start.month == end.month && start.day < end.day) {
            return (leap_years_count(end.year) - leap_years_count(start.year - 1.0)) / (end.year - start.year + 1.0) + 365.0;
        }
        if count_leap_days(end) != count_leap_days(&SimpleDate { year: start.year, month: start.month, day: start.day - 1.0 }) {
            return 366.0;
        }
        return 365.0;
    }
    if is_leap_year(start.year) { 366.0 } else { 365.0 }
}

pub fn to_basis_us(mut start: SimpleDate, mut end: SimpleDate) -> (SimpleDate, SimpleDate) {
    if start.day == 31.0 {
        start.day = 30.0;
    }
    if start.day == 30.0 && end.day == 31.0 {
        end.day = 30.0;
    }
    if start.month == 2.0 && start.day == days_in_month(start.year, start.month) {
        start.day = 30.0;
        if end.month == 2.0 && end.day == days_in_month(end.year, end.month) {
            end.day = 30.0;
        }
    }
    (start, end)
}

pub fn to_basis_eu(d: SimpleDate) -> SimpleDate {
    SimpleDate { day: d.day.min(30.0), ..d }
}

// ------------------------------------------------------------------ parsing

/// JS `Number(str)` for date/time items (`''` → 0).
fn js_number(s: &str) -> f64 {
    oc_schema::js::string_to_number(s)
}

fn is_int(x: f64) -> bool {
    oc_schema::js::is_integer(x)
}

struct DateFmt {
    count: usize,
    day: Option<usize>,
    month: Option<usize>,
    short_year: Option<usize>,
    long_year: Option<usize>,
}

fn split_date_sep(s: &str) -> Vec<&str> {
    s.split([' ', '/', '.', '-']).collect()
}

fn parse_date_format(f: &str) -> DateFmt {
    let lower = f.to_lowercase();
    let items = split_date_sep(lower.trim());
    let pos = |k: &str| items.iter().position(|x| *x == k);
    DateFmt { count: items.len(), day: pos("dd"), month: pos("mm"), short_year: pos("yy"), long_year: pos("yyyy") }
}

struct TimeFmt {
    count: usize,
    hour: Option<usize>,
    minute: Option<usize>,
    second: Option<usize>,
}

fn is_seconds_item(s: &str) -> bool {
    // ^ss(\.(s+|0+))?$
    let Some(rest) = s.strip_prefix("ss") else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(r) = rest.strip_prefix('.') else { return false };
    !r.is_empty() && (r.chars().all(|c| c == 's') || r.chars().all(|c| c == '0'))
}

fn parse_time_format(f: &str) -> TimeFmt {
    let lower = f.to_lowercase();
    let t = lower.trim();
    let without = if let Some(x) = t.strip_suffix("am/pm") {
        x.to_string()
    } else if t.ends_with("a/p") {
        // `formatLowercase.substring(0, timeFormat.length - 3)` (length of the untrimmed format).
        t.chars().take(f.chars().count().saturating_sub(3)).collect()
    } else {
        t.to_string()
    };
    let items: Vec<&str> = without.trim().split(':').collect();
    TimeFmt {
        count: items.len(),
        hour: items.iter().position(|x| *x == "hh"),
        minute: items.iter().position(|x| *x == "mm"),
        second: items.iter().position(|x| is_seconds_item(x)),
    }
}

fn get<'a>(items: &[&'a str], i: Option<usize>) -> Option<&'a str> {
    i.and_then(|i| items.get(i).copied())
}

fn parse_to_time(mut items: Vec<String>, fmt: &TimeFmt) -> Option<SimpleTime> {
    let mut ampm = None;
    match items.last().map(String::as_str) {
        Some("am") | Some("a") => {
            ampm = Some(false);
            items.pop();
        }
        Some("pm") | Some("p") => {
            ampm = Some(true);
            items.pop();
        }
        _ => {}
    }
    if items.len() != fmt.count {
        return None;
    }
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let secs_parsed = js_number(get(&refs, fmt.second).unwrap_or("0"));
    if !secs_parsed.is_finite() {
        return None;
    }
    let seconds = js_round(secs_parsed * 1000.0) / 1000.0;
    let minutes = js_number(get(&refs, fmt.minute).unwrap_or("0"));
    if !(minutes.is_finite() && is_int(minutes)) {
        return None;
    }
    let hours_parsed = js_number(get(&refs, fmt.hour).unwrap_or("0"));
    if !(hours_parsed.is_finite() && is_int(hours_parsed)) {
        return None;
    }
    if ampm.is_some() && !(0.0..=12.0).contains(&hours_parsed) {
        return None;
    }
    let hours = match ampm {
        Some(pm) => hours_parsed % 12.0 + if pm { 12.0 } else { 0.0 },
        None => hours_parsed,
    };
    Some(SimpleTime { hours, minutes, seconds })
}

fn parse_to_date(items: &[&str], fmt: &DateFmt) -> Option<SimpleDate> {
    if items.len() != fmt.count {
        return None;
    }
    let day = js_number(get(items, fmt.day)?);
    if !(day.is_finite() && is_int(day)) {
        return None;
    }
    let month = js_number(get(items, fmt.month)?);
    if !(month.is_finite() && is_int(month)) {
        return None;
    }
    let long = get(items, fmt.long_year).filter(|s| !s.is_empty());
    let short = get(items, fmt.short_year).filter(|s| !s.is_empty());
    if long.is_some() && short.is_some() {
        return None;
    }
    let ystr = get(items, fmt.long_year).or_else(|| get(items, fmt.short_year))?;
    let year = js_number(ystr);
    if !(year.is_finite() && is_int(year)) {
        return None;
    }
    if long.is_some() && !(1000.0..=9999.0).contains(&year) {
        return None;
    }
    if short.is_some() && !(0.0..=99.0).contains(&year) {
        return None;
    }
    Some(SimpleDate { year, month, day })
}

/// A parsed date/time string (`SimpleDate`, `SimpleTime` or both).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Parsed {
    Date(SimpleDate),
    Time(SimpleTime),
    DateTime(SimpleDate, SimpleTime),
}

/// `^[0-9/.\-: ]+[ap]?m?$`
fn quick_check(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b'/' | b'.' | b'-' | b':' | b' ')) {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    if i < b.len() && matches!(b[i], b'a' | b'p') {
        i += 1;
    }
    if i < b.len() && b[i] == b'm' {
        i += 1;
    }
    i == b.len()
}

fn default_parse(text: &str, date_fmt: &str, time_fmt: &str) -> Option<Parsed> {
    // `text.replace(/\s+/, ' ')` replaces the FIRST whitespace run only, then trim + lower-case.
    let mut s = String::new();
    let mut replaced = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !replaced && oc_schema::js::is_js_space(c) {
            replaced = true;
            while chars.peek().is_some_and(|c| oc_schema::js::is_js_space(*c)) {
                chars.next();
            }
            s.push(' ');
        } else {
            s.push(c);
        }
    }
    let mut dt = oc_schema::js::trim(&s).to_lowercase();
    if !quick_check(&dt) {
        return None;
    }
    let mut ampm: Option<String> = None;
    if dt.ends_with("am") || dt.ends_with("pm") {
        ampm = Some(dt[dt.len() - 2..].to_string());
        dt = dt[..dt.len() - 2].trim().to_string();
    } else if dt.ends_with('a') || dt.ends_with('p') {
        ampm = Some(dt[dt.len() - 1..].to_string());
        dt = dt[..dt.len() - 1].trim().to_string();
    }
    let mut date_items: Vec<String> = split_date_sep(&dt).into_iter().map(str::to_string).collect();
    let n = date_items.len();
    if n >= 2 && date_items[n - 2].contains(':') {
        let last = date_items.pop().expect("item");
        let l = date_items.len();
        date_items[l - 1] = format!("{}.{}", date_items[l - 1], last);
    }
    let mut time_items: Vec<String> = date_items.last().map(|x| x.split(':').map(str::to_string).collect()).unwrap_or_default();
    if let Some(a) = ampm {
        time_items.push(a);
    }
    let tf = parse_time_format(time_fmt);
    let df = parse_date_format(date_fmt);
    if date_items.len() == 1 {
        return parse_to_time(time_items, &tf).map(Parsed::Time);
    }
    if time_items.len() == 1 {
        let refs: Vec<&str> = date_items.iter().map(String::as_str).collect();
        return parse_to_date(&refs, &df).map(Parsed::Date);
    }
    let refs: Vec<&str> = date_items[..date_items.len() - 1].iter().map(String::as_str).collect();
    let d = parse_to_date(&refs, &df)?;
    let t = parse_to_time(time_items, &tf)?;
    Some(Parsed::DateTime(d, t))
}

fn fix_year(d: &mut SimpleDate) -> bool {
    if d.year >= 0.0 && d.year < 100.0 {
        if d.year < NULL_YEAR as f64 {
            d.year += 2000.0;
        } else {
            d.year += 1900.0;
        }
    }
    is_valid_date(d)
}

/// `parseDateTimeFromConfigFormats`: the first format pair that parses to a valid date/time.
pub fn parse_date_time(s: &str) -> Option<Parsed> {
    for df in DATE_FORMATS {
        for tf in TIME_FORMATS {
            let Some(p) = default_parse(s, df, tf) else { continue };
            match p {
                Parsed::Time(_) => return Some(p),
                Parsed::Date(mut d) => {
                    if fix_year(&mut d) {
                        return Some(Parsed::Date(d));
                    }
                }
                Parsed::DateTime(mut d, t) => {
                    if fix_year(&mut d) {
                        return Some(Parsed::DateTime(d, t));
                    }
                }
            }
        }
    }
    None
}

/// `dateStringToDateNumber`: a date/time/datetime number, or `None`.
pub fn date_string_to_number(s: &str) -> Option<Scalar> {
    Some(match parse_date_time(s)? {
        Parsed::Time(t) => Scalar::Num(time_to_number(&t), NumType::Time),
        Parsed::Date(d) => Scalar::Num(date_to_number(&d), NumType::Date),
        Parsed::DateTime(d, t) => Scalar::Num(time_to_number(&t) + date_to_number(&d), NumType::DateTime),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(date_string_to_number("2024-01-15"), Some(Scalar::Num(45306.0, NumType::Date)));
        assert_eq!(date_string_to_number("15/01/2024"), Some(Scalar::Num(45306.0, NumType::Date)));
        assert_eq!(date_string_to_number("10:30"), Some(Scalar::Num(10.5 / 24.0, NumType::Time)));
        assert!(date_string_to_number("hello").is_none());
        let d = number_to_simple_date(45306.0);
        assert_eq!((d.year, d.month, d.day), (2024.0, 1.0, 15.0));
    }
}
