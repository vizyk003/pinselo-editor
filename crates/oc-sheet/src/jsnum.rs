// SPDX-License-Identifier: AGPL-3.0-only
//! JS number formatting not covered by `oc_schema::js`: `toPrecision`, `toExponential`, plus the `Math`
//! helpers HyperFormula relies on (exact half-up rounding of decimal expansions, as the spec requires).

pub use oc_schema::js::{num_to_string, round as js_round, to_fixed};

/// Exact decimal digits of |x| (finite, non-zero): (digits without leading zeros, exponent of first digit).
fn exact_digits(x: f64) -> (Vec<u8>, i32) {
    let s = format!("{:.1100e}", x.abs());
    let (mant, exp) = s.split_once('e').expect("exp");
    let exp: i32 = exp.parse().expect("exp");
    let mut d: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
    while d.len() > 1 && *d.last().unwrap() == 0 {
        d.pop();
    }
    (d, exp)
}

/// Round the digit string to `n` significant digits, ties away from zero; returns (digits[n], exponent).
fn round_sig(x: f64, n: usize) -> (Vec<u8>, i32) {
    let (d, mut e) = exact_digits(x);
    let mut out: Vec<u8> = d.iter().take(n).copied().collect();
    while out.len() < n {
        out.push(0);
    }
    if d.len() > n && d[n] >= 5 {
        let mut i = n;
        loop {
            if i == 0 {
                out.insert(0, 1);
                out.truncate(n);
                e += 1;
                break;
            }
            i -= 1;
            if out[i] == 9 {
                out[i] = 0;
            } else {
                out[i] += 1;
                break;
            }
        }
    }
    (out, e)
}

fn digits_str(d: &[u8]) -> String {
    d.iter().map(|x| (b'0' + x) as char).collect()
}

/// `Number#toExponential(f)`.
pub fn to_exponential(x: f64, f: usize) -> String {
    if !x.is_finite() {
        return num_to_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let (d, e) = if x == 0.0 { (vec![0; f + 1], 0) } else { round_sig(x, f + 1) };
    let s = digits_str(&d);
    let mant = if f == 0 { s } else { format!("{}.{}", &s[..1], &s[1..]) };
    format!("{sign}{mant}e{}{}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// `Number#toPrecision(p)`.
pub fn to_precision(x: f64, p: usize) -> String {
    if !x.is_finite() {
        return num_to_string(x);
    }
    let sign = if x < 0.0 { "-" } else { "" };
    if x == 0.0 {
        let z = if p > 1 { format!("0.{}", "0".repeat(p - 1)) } else { "0".into() };
        return format!("{sign}{z}").replace("-0", "0");
    }
    let (d, e) = round_sig(x, p);
    let s = digits_str(&d);
    if e < -6 || e >= p as i32 {
        let mant = if p == 1 { s } else { format!("{}.{}", &s[..1], &s[1..]) };
        return format!("{sign}{mant}e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    if e == p as i32 - 1 {
        return format!("{sign}{s}");
    }
    if e >= 0 {
        let k = (e + 1) as usize;
        return format!("{sign}{}.{}", &s[..k], &s[k..]);
    }
    format!("{sign}0.{}{s}", "0".repeat((-e - 1) as usize))
}

/// `Number(str)` for strings produced by the formatters above.
pub fn parse_num(s: &str) -> f64 {
    oc_schema::js::string_to_number(s)
}

/// JS `parseFloat`: the longest prefix (after leading white space) that is a decimal literal — sign,
/// `Infinity`, digits with an optional fraction and exponent — or NaN.
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(oc_schema::js::is_js_space);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if t[i..].starts_with("Infinity") {
        return if b.first() == Some(&b'-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let mut n = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        let f = digits(&mut j);
        if n + f > 0 {
            n += f;
            i = j;
        }
    }
    if n == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if digits(&mut j) > 0 {
            i = j;
        }
    }
    t[..i].parse().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision() {
        assert_eq!(to_precision(1.25, 2), "1.3");
        assert_eq!(to_precision(123456.0, 2), "1.2e+5");
        assert_eq!(to_precision(0.000123, 2), "0.00012");
        assert_eq!(to_precision(0.0000001, 2), "1.0e-7");
        assert_eq!(to_precision(99.99, 3), "100");
        assert_eq!(to_precision(1.0 / 3.0, 10), "0.3333333333");
        assert_eq!(to_exponential(12345.0, 4), "1.2345e+4");
        assert_eq!(to_exponential(-1.5e-10, 4), "-1.5000e-10");
        assert_eq!(to_exponential(0.0, 2), "0.00e+0");
    }

    #[test]
    fn parse_float_prefixes() {
        assert_eq!(parse_float("1e+21"), 1e21);
        assert_eq!(parse_float(" -2.5e-7xyz"), -2.5e-7);
        assert_eq!(parse_float("12abc"), 12.0);
        assert_eq!(parse_float(".5"), 0.5);
        assert_eq!(parse_float("5."), 5.0);
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_float("-Infinityx"), f64::NEG_INFINITY);
        assert!(parse_float("abc").is_nan());
        assert!(parse_float(".").is_nan());
        assert!(parse_float("").is_nan());
    }
}
