// SPDX-License-Identifier: AGPL-3.0-only
//! DateTimePlugin: DATE, TIME, YEAR/MONTH/DAY, HOUR/MINUTE/SECOND, EDATE/EOMONTH, WEEKDAY/WEEKNUM/ISOWEEKNUM,
//! DATEVALUE/TIMEVALUE, NOW/TODAY, DAYS/DAYS360/DATEDIF/YEARFRAC, INTERVAL, NETWORKDAYS(.INTL), WORKDAY(.INTL).
//! (TEXT lives in the text module.)

use super::super::datetime::{
    self as dt, EPOCH_YEAR_ZERO, Parsed, SimpleDate, SimpleTime, date_to_number, days_in_month, end_of_month, is_valid_date,
    leap_years_count_pub, number_to_simple_date, number_to_simple_time, offset_month, relative_to_absolute, round_to_nearest_second,
    time_to_number, to_basis_eu, to_basis_us, truncate_day_in_month, within_bounds, year_length_for_basis,
};
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, Scalar, Val};
use super::{Kind, err, num, s, simple};

fn bounded(ret: f64) -> Val {
    match within_bounds(ret) {
        Some(v) => num(v),
        None => err(ErrorType::Num),
    }
}

fn date(_: &Call, a: &[Arg]) -> Val {
    let d = a[2].num().trunc();
    let mut m = a[1].num().trunc();
    let mut y = a[0].num().trunc();
    if y < EPOCH_YEAR_ZERO {
        y += EPOCH_YEAR_ZERO;
    }
    let delta = ((m - 1.0) / 12.0).floor();
    y += delta;
    m -= delta * 12.0;
    let date = SimpleDate { year: y, month: m, day: 1.0 };
    if is_valid_date(&date) {
        return bounded(date_to_number(&date) + (d - 1.0));
    }
    err(ErrorType::Value)
}

fn time(_: &Call, a: &[Arg]) -> Val {
    let ret = time_to_number(&SimpleTime { hours: a[0].num().trunc(), minutes: a[1].num().trunc(), seconds: a[2].num().trunc() });
    if ret < 0.0 {
        return err(ErrorType::Num);
    }
    num(ret % 1.0)
}

fn eomonth(_: &Call, a: &[Arg]) -> Val {
    let date = number_to_simple_date(a[0].num());
    bounded(date_to_number(&end_of_month(&offset_month(&date, a[1].num()))))
}

fn edate(_: &Call, a: &[Arg]) -> Val {
    let date = number_to_simple_date(a[0].num());
    bounded(date_to_number(&truncate_day_in_month(&offset_month(&date, a[1].num()))))
}

fn day(_: &Call, a: &[Arg]) -> Val {
    num(number_to_simple_date(a[0].num()).day)
}

fn month(_: &Call, a: &[Arg]) -> Val {
    num(number_to_simple_date(a[0].num()).month)
}

fn year(_: &Call, a: &[Arg]) -> Val {
    num(number_to_simple_date(a[0].num()).year)
}

fn days(_: &Call, a: &[Arg]) -> Val {
    num(a[0].num().trunc() - a[1].num().trunc())
}

fn time_part(a: &[Arg]) -> SimpleTime {
    number_to_simple_time(round_to_nearest_second(a[0].num()) % 1.0)
}

fn hour(_: &Call, a: &[Arg]) -> Val {
    num(time_part(a).hours)
}

fn minute(_: &Call, a: &[Arg]) -> Val {
    num(time_part(a).minutes)
}

fn second(_: &Call, a: &[Arg]) -> Val {
    num(time_part(a).seconds)
}

fn weekday_offset(t: f64) -> Option<f64> {
    Some(match t {
        1.0 | 17.0 => 0.0,
        2.0 | 11.0 => 1.0,
        12.0 => 2.0,
        13.0 => 3.0,
        14.0 => 4.0,
        15.0 => 5.0,
        16.0 => 6.0,
        _ => return None,
    })
}

fn weekday(_: &Call, a: &[Arg]) -> Val {
    let abs = relative_to_absolute(a[0].num()).floor();
    let t = a[1].num();
    if t == 3.0 {
        return num((abs - 1.0) % 7.0);
    }
    match weekday_offset(t) {
        Some(off) => num((abs - off) % 7.0 + 1.0),
        None => err(ErrorType::Num),
    }
}

fn isoweeknum_core(day: f64) -> f64 {
    let abs = relative_to_absolute(day).floor();
    let date = number_to_simple_date(day);
    let year_start = date_to_number(&SimpleDate { year: date.year, month: 1.0, day: 1.0 });
    let ys_abs = relative_to_absolute(year_start);
    let first_thursday = ys_abs + ((4.0 - ys_abs) % 7.0 + 7.0) % 7.0;
    let ret = ((abs - 1.0) / 7.0).floor() - ((first_thursday - 1.0) / 7.0).floor() + 1.0;
    if ret == 0.0 {
        return isoweeknum_core(day - 7.0) + 1.0;
    }
    ret
}

fn weeknum(_: &Call, a: &[Arg]) -> Val {
    let day = a[0].num();
    let t = a[1].num();
    let abs = relative_to_absolute(day).floor();
    let date = number_to_simple_date(day);
    let year_start = date_to_number(&SimpleDate { year: date.year, month: 1.0, day: 1.0 });
    let ys_abs = relative_to_absolute(year_start);
    if t == 21.0 {
        return num(isoweeknum_core(day));
    }
    match weekday_offset(t) {
        Some(off) => num(((abs - off) / 7.0).floor() - ((ys_abs - off) / 7.0).floor() + 1.0),
        None => err(ErrorType::Num),
    }
}

fn isoweeknum(_: &Call, a: &[Arg]) -> Val {
    num(isoweeknum_core(a[0].num()))
}

fn datevalue(_: &Call, a: &[Arg]) -> Val {
    match dt::parse_date_time(a[0].str()) {
        None => err(ErrorType::Value),
        Some(Parsed::Time(_)) => num(0.0),
        Some(Parsed::Date(d)) => num(date_to_number(&d)),
        Some(Parsed::DateTime(d, t)) => num(time_to_number(&t).trunc() + date_to_number(&d)),
    }
}

fn timevalue(_: &Call, a: &[Arg]) -> Val {
    match dt::date_string_to_number(a[0].str()) {
        Some(Scalar::Num(v, _)) => num(v % 1.0),
        _ => err(ErrorType::Value),
    }
}

/// Local calendar parts of "now" (`new Date(Date.now())` getters in the engine's timezone).
fn local_now(c: &Call) -> (SimpleDate, SimpleTime) {
    let env = c.it.env;
    let t = env.now_ms() - env.tz_offset_minutes() * 60_000.0;
    let p = crate::format::time_parts(t).unwrap_or(crate::format::DateParts { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, ms: 0 });
    (
        SimpleDate { year: p.year as f64, month: f64::from(p.month), day: f64::from(p.day) },
        SimpleTime { hours: f64::from(p.hour), minutes: f64::from(p.minute), seconds: f64::from(p.second) },
    )
}

fn now(c: &Call, _: &[Arg]) -> Val {
    let (d, t) = local_now(c);
    num(time_to_number(&t) + date_to_number(&d))
}

fn today(c: &Call, _: &[Arg]) -> Val {
    let (d, _) = local_now(c);
    num(date_to_number(&d))
}

fn days360_core(start_date: f64, end_date: f64, eu: bool) -> f64 {
    let start = number_to_simple_date(start_date);
    let end = number_to_simple_date(end_date);
    let (ns, ne) = if eu { (to_basis_eu(start), to_basis_eu(end)) } else { to_basis_us(start, end) };
    360.0 * (ne.year - ns.year) + 30.0 * (ne.month - ns.month) + ne.day - ns.day
}

fn days360(_: &Call, a: &[Arg]) -> Val {
    num(days360_core(a[0].num(), a[1].num(), a[2].bool()))
}

fn datedif(_: &Call, a: &[Arg]) -> Val {
    let (start_date, end_date) = (a[0].num(), a[1].num());
    let unit = a[2].str();
    if start_date > end_date {
        return err(ErrorType::Num);
    }
    if unit == "D" {
        return num(end_date.floor() - start_date.floor());
    }
    let start = number_to_simple_date(start_date);
    let end = number_to_simple_date(end_date);
    let before = if end.day < start.day { 1.0 } else { 0.0 };
    let after_anniversary = end.month > start.month || (end.month == start.month && end.day >= start.day);
    num(match unit {
        "M" => (end.year - start.year) * 12.0 + (end.month - start.month) - before,
        "YM" => (12.0 + (end.month - start.month) - before) % 12.0,
        "Y" => {
            if after_anniversary {
                end.year - start.year
            } else {
                end.year - start.year - 1.0
            }
        }
        "MD" => {
            if end.day >= start.day {
                end.day - start.day
            } else {
                let m = if end.month == 1.0 { 12.0 } else { end.month - 1.0 };
                let y = if end.month == 1.0 { end.year - 1.0 } else { end.year };
                days_in_month(y, m) + end.day - start.day
            }
        }
        "YD" => {
            if after_anniversary {
                end_date.floor() - date_to_number(&SimpleDate { year: end.year, month: start.month, day: start.day })
            } else {
                end_date.floor() - start_date.floor() - 365.0 * (end.year - start.year - 1.0) - leap_years_count_pub(end.year - 1.0)
                    + leap_years_count_pub(start.year)
            }
        }
        _ => return err(ErrorType::Num),
    })
}

fn yearfrac(_: &Call, a: &[Arg]) -> Val {
    let mut start = a[0].num().trunc();
    let mut end = a[1].num().trunc();
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    num(match a[2].num() as i64 {
        0 => days360_core(start, end, false) / 360.0,
        1 => (end - start) / year_length_for_basis(&number_to_simple_date(start), &number_to_simple_date(end)),
        2 => (end - start) / 360.0,
        3 => (end - start) / 365.0,
        _ => days360_core(start, end, true) / 360.0,
    })
}

fn interval(_: &Call, a: &[Arg]) -> Val {
    let mut arg = a[0].num().trunc();
    let second = arg % 60.0;
    arg = (arg / 60.0).trunc();
    let minute = arg % 60.0;
    arg = (arg / 60.0).trunc();
    let hour = arg % 24.0;
    arg = (arg / 24.0).trunc();
    let day = arg % 30.0;
    arg = (arg / 30.0).trunc();
    let month = arg % 12.0;
    let year = (arg / 12.0).trunc();
    let part = |v: f64, u: &str| if v > 0.0 { format!("{}{u}", crate::jsnum::num_to_string(v)) } else { String::new() };
    s(format!(
        "P{}{}{}T{}{}{}",
        part(year, "Y"),
        part(month, "M"),
        part(day, "D"),
        part(hour, "H"),
        part(minute, "M"),
        part(second, "S")
    ))
}

// ------------------------------------------------------------------ workdays

fn weekend_pattern(w: &Scalar) -> Result<String, ErrorType> {
    match w {
        Scalar::Str(s) => {
            if s.chars().count() != 7 || !s.chars().all(|c| c == '0' || c == '1') || &**s == "1111111" {
                Err(ErrorType::Num)
            } else {
                Ok(s.to_string())
            }
        }
        Scalar::Num(v, _) => {
            let p = match *v {
                1.0 => "0000011",
                2.0 => "1000001",
                3.0 => "1100000",
                4.0 => "0110000",
                5.0 => "0011000",
                6.0 => "0001100",
                7.0 => "0000110",
                11.0 => "0000001",
                12.0 => "1000000",
                13.0 => "0100000",
                14.0 => "0010000",
                15.0 => "0001000",
                16.0 => "0000100",
                17.0 => "0000010",
                _ => return Err(ErrorType::Num),
            };
            Ok(p.to_string())
        }
        _ => Err(ErrorType::Value),
    }
}

fn is_workday(pattern: &[u8], abs: f64) -> bool {
    // `(val - 1) % 7` with JS remainder semantics (non-negative for valid dates).
    let i = (abs - 1.0) % 7.0;
    i >= 0.0 && pattern.get(i as usize) == Some(&b'0')
}

fn filtered_holidays(pattern: &[u8], holidays: &Arg) -> Result<Vec<f64>, ErrorType> {
    let vals: Vec<Scalar> = match holidays {
        Arg::Missing => vec![],
        other => other.to_range().values().cloned().collect(),
    };
    for v in &vals {
        if let Scalar::Err(e) = v {
            return Err(*e);
        }
    }
    let mut out: Vec<f64> = Vec::new();
    for v in &vals {
        match v {
            Scalar::Empty => {}
            Scalar::Num(x, _) => out.push(x.trunc()),
            _ => return Err(ErrorType::Value),
        }
    }
    let mut uniq: Vec<f64> = Vec::new();
    for x in out {
        if !uniq.contains(&x) {
            uniq.push(x);
        }
    }
    uniq.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(uniq.into_iter().filter(|x| is_workday(pattern, relative_to_absolute(*x))).collect())
}

fn lower_bound(val: f64, arr: &[f64]) -> usize {
    if arr.is_empty() || val <= arr[0] {
        return 0;
    }
    if arr[arr.len() - 1] < val {
        return arr.len();
    }
    let mut lower = 0;
    let mut upper = arr.len() - 1;
    while lower + 1 < upper {
        let mid = (upper + lower) / 2;
        if arr[mid] >= val {
            upper = mid;
        } else {
            lower = mid;
        }
    }
    upper
}

fn count_workdays(start: f64, end: f64, pattern: &[u8], holidays: &[f64]) -> f64 {
    let abs_end = relative_to_absolute(end).floor();
    let abs_start = relative_to_absolute(start).floor();
    let mut ans = 0.0;
    for (i, c) in pattern.iter().enumerate().take(7) {
        if *c == b'0' {
            let i = i as f64;
            ans += ((abs_end + 6.0 - i) / 7.0).floor();
            ans -= ((abs_start - 1.0 + 6.0 - i) / 7.0).floor();
        }
    }
    ans - (lower_bound(end + 1.0, holidays) as f64 - lower_bound(start, holidays) as f64)
}

fn networkdays_core(start: f64, end: f64, weekend: &Scalar, holidays: &Arg) -> Val {
    let (mut start, mut end) = (start.trunc(), end.trunc());
    let mut mult = 1.0;
    if start > end {
        std::mem::swap(&mut start, &mut end);
        mult = -1.0;
    }
    let pattern = match weekend_pattern(weekend) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let hol = match filtered_holidays(pattern.as_bytes(), holidays) {
        Ok(h) => h,
        Err(e) => return err(e),
    };
    num(mult * count_workdays(start, end, pattern.as_bytes(), &hol))
}

fn workday_core(start: f64, delta: f64, weekend: &Scalar, holidays: &Arg) -> Val {
    let start = start.trunc();
    let mut delta = delta.trunc();
    let pattern = match weekend_pattern(weekend) {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let p = pattern.as_bytes();
    let hol = match filtered_holidays(p, holidays) {
        Ok(h) => h,
        Err(e) => return err(e),
    };
    if delta > 0.0 {
        let mut upper = 1.0;
        while count_workdays(start + 1.0, start + upper, p, &hol) < delta {
            upper *= 2.0;
        }
        let mut lower = 1.0;
        while lower + 1.0 < upper {
            let mid = ((lower + upper) / 2.0).trunc();
            if count_workdays(start + 1.0, start + mid, p, &hol) < delta {
                lower = mid;
            } else {
                upper = mid;
            }
        }
        num(start + upper)
    } else if delta < 0.0 {
        delta = -delta;
        let mut upper = 1.0;
        while count_workdays(start - upper, start - 1.0, p, &hol) < delta {
            upper *= 2.0;
        }
        let mut lower = 1.0;
        while lower + 1.0 < upper {
            let mid = ((lower + upper) / 2.0).trunc();
            if count_workdays(start - mid, start - 1.0, p, &hol) < delta {
                lower = mid;
            } else {
                upper = mid;
            }
        }
        num(start - upper)
    } else {
        num(start)
    }
}

fn networkdays(_: &Call, a: &[Arg]) -> Val {
    networkdays_core(a[0].num(), a[1].num(), &Scalar::num(1.0), a.get(2).unwrap_or(&Arg::Missing))
}

fn networkdays_intl(_: &Call, a: &[Arg]) -> Val {
    networkdays_core(a[0].num(), a[1].num(), &a[2].scalar(), a.get(3).unwrap_or(&Arg::Missing))
}

fn workday(_: &Call, a: &[Arg]) -> Val {
    workday_core(a[0].num(), a[1].num(), &Scalar::num(1.0), a.get(2).unwrap_or(&Arg::Missing))
}

fn workday_intl(_: &Call, a: &[Arg]) -> Val {
    workday_core(a[0].num(), a[1].num(), &a[2].scalar(), a.get(3).unwrap_or(&Arg::Missing))
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("DATE", date),
        ("TIME", time),
        ("EOMONTH", eomonth),
        ("DAY", day),
        ("DAYS", days),
        ("MONTH", month),
        ("YEAR", year),
        ("HOUR", hour),
        ("MINUTE", minute),
        ("SECOND", second),
        ("WEEKDAY", weekday),
        ("WEEKNUM", weeknum),
        ("ISOWEEKNUM", isoweeknum),
        ("DATEVALUE", datevalue),
        ("TIMEVALUE", timevalue),
        ("NOW", now),
        ("TODAY", today),
        ("EDATE", edate),
        ("DAYS360", days360),
        ("DATEDIF", datedif),
        ("YEARFRAC", yearfrac),
        ("INTERVAL", interval),
        ("NETWORKDAYS", networkdays),
        ("NETWORKDAYS.INTL", networkdays_intl),
        ("WORKDAY", workday),
        ("WORKDAY.INTL", workday_intl),
    ])
}
