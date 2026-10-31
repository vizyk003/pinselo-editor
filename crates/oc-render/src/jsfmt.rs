// SPDX-License-Identifier: Apache-2.0
//! JS number semantics the render-core port relies on (beyond `oc_schema::js`): `+n.toFixed(2)` path
//! coordinates, NaN-propagating `Math.min/max`, fdlibm `**`/`Math.log10`, `parseFloat` and
//! `toLocaleString('en-US', …)` as V8/ICU compute them.

pub use oc_schema::js::{num_to_string, round, to_fixed};

/// `Math.min(a, b)` (NaN-propagating, `-0 < +0`).
pub fn min(a: f64, b: f64) -> f64 {
    oc_geometry::jsmath::min(a, b)
}

/// `Math.max(a, b)` (NaN-propagating, `+0 > -0`).
pub fn max(a: f64, b: f64) -> f64 {
    oc_geometry::jsmath::max(a, b)
}

/// `Math.max(...vs)` (`-Infinity` for an empty list).
pub fn max_of(vs: impl IntoIterator<Item = f64>) -> f64 {
    vs.into_iter().fold(f64::NEG_INFINITY, max)
}

/// `Math.min(...vs)` (`Infinity` for an empty list).
pub fn min_of(vs: impl IntoIterator<Item = f64>) -> f64 {
    vs.into_iter().fold(f64::INFINITY, min)
}

/// `a ** b` / `Math.pow` (fdlibm, like V8's `ieee754::pow`).
pub fn pow(a: f64, b: f64) -> f64 {
    libm::pow(a, b)
}

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}

pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

pub fn hypot(a: f64, b: f64) -> f64 {
    oc_geometry::jsmath::hypot(a, b)
}

/// JS truthiness of a number.
pub fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

/// `x % y` (`fmod`, sign of the dividend), as JS computes it.
pub fn rem(x: f64, y: f64) -> f64 {
    x % y
}

/// `+n.toFixed(2)`.
pub fn fixed2(n: f64) -> f64 {
    let a = n.abs();
    if !n.is_finite() {
        return n;
    }
    if a < 1e9 {
        // |y| < 2^37, so the product is within 2^-15 of the exact value: decide directly unless near a tie.
        let y = a * 100.0;
        let fl = y.floor();
        let frac = y - fl;
        if (frac - 0.5).abs() > 1e-3 {
            let k = if frac > 0.5 { fl + 1.0 } else { fl };
            if k == 0.0 {
                return 0.0;
            }
            // k / 100 is correctly rounded: the double nearest to the decimal string toFixed produced.
            let v = k / 100.0;
            return if n < 0.0 { -v } else { v };
        }
    }
    oc_schema::js::to_fixed_num(n, 2)
}

/// `` `${+n.toFixed(2)}` `` — how the builders print coordinates (`f` in builders.ts).
pub fn f(n: f64) -> String {
    num_to_string(fixed2(n))
}

/// `` `${n}` `` — `String(n)`.
pub fn s(n: f64) -> String {
    num_to_string(n)
}

/// JS `parseFloat`: the longest `StrDecimalLiteral` prefix after leading whitespace (`NaN` when there is none).
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(oc_schema::js::is_js_space);
    let b = t.as_bytes();
    let mut i = 0;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    if t[i..].starts_with("Infinity") {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let start = i;
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        let mut frac = 0;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
            frac += 1;
        }
        if digits > 0 || frac > 0 {
            i = j;
            digits += frac;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > k {
            i = j;
        }
    }
    let lit = &t[start..i];
    let v: f64 = lit.parse().unwrap_or(f64::NAN);
    if neg { -v } else { v }
}

/// Shortest round-trip decimal digits of a finite, non-zero `|v|` and the decimal exponent `n` such that
/// `|v| = 0.d1d2… × 10^n`.
fn shortest_digits(v: f64) -> (Vec<u8>, i32) {
    let e = format!("{:e}", v.abs());
    let (mant, exp) = e.split_once('e').expect("exponent form");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: Vec<u8> = mant.bytes().filter(|c| *c != b'.').map(|c| c - b'0').collect();
    (digits, exp + 1)
}

/// `v.toLocaleString('en-US', { minimumFractionDigits: d, maximumFractionDigits: d })`: ICU rounds the shortest
/// decimal representation half away from zero and groups the integer part by thousands.
pub fn to_locale_fixed(v: f64, d: usize) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let neg = v.is_sign_negative();
    // Integer digits and `d` fraction digits, as one digit string with the point `int_len` from the left.
    let (mut ds, int_len): (Vec<u8>, usize) = if v == 0.0 {
        (vec![0; 1 + d], 1)
    } else {
        let (digits, n) = shortest_digits(v);
        // Position the digits: value = digits × 10^(n − len).
        let int_len = n.max(1) as usize;
        let lead_zeros = if n < 1 { (1 - n) as usize } else { 0 };
        let mut all: Vec<u8> = vec![0; lead_zeros];
        all.extend_from_slice(&digits);
        // `all` now starts at the 10^(int_len-1) place.
        let want = int_len + d;
        let round_up = all.get(want).is_some_and(|x| *x >= 5);
        all.resize(want.max(all.len()), 0);
        all.truncate(want);
        if round_up {
            let mut i = all.len();
            loop {
                if i == 0 {
                    all.insert(0, 1);
                    break;
                }
                i -= 1;
                if all[i] == 9 {
                    all[i] = 0;
                } else {
                    all[i] += 1;
                    break;
                }
            }
        }
        let il = all.len() - d;
        (all, il)
    };
    // Strip leading zeros of the integer part (keep one).
    let mut lead = 0;
    while lead + 1 < int_len && ds[lead] == 0 {
        lead += 1;
    }
    let int: Vec<u8> = ds.drain(..int_len).skip(lead).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    for (i, dg) in int.iter().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push((b'0' + dg) as char);
    }
    if d > 0 {
        out.push('.');
        out.extend(ds.iter().map(|dg| (b'0' + dg) as char));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_fixed() {
        assert_eq!(to_locale_fixed(1234567.891, 2), "1,234,567.89");
        assert_eq!(to_locale_fixed(1.005, 2), "1.01");
        assert_eq!(to_locale_fixed(2.675, 2), "2.68");
        assert_eq!(to_locale_fixed(999.995, 2), "1,000.00");
        assert_eq!(to_locale_fixed(0.0, 0), "0");
        assert_eq!(to_locale_fixed(-0.0, 1), "-0.0");
        assert_eq!(to_locale_fixed(0.004, 2), "0.00");
        assert_eq!(to_locale_fixed(12.0, 0), "12");
        assert_eq!(to_locale_fixed(1e21, 0), "1,000,000,000,000,000,000,000");
        assert_eq!(to_locale_fixed(0.5, 0), "1");
        assert_eq!(to_locale_fixed(-1500.25, 1), "-1,500.3");
    }

    #[test]
    fn parse_float_prefixes() {
        assert_eq!(parse_float("  12px"), 12.0);
        assert_eq!(parse_float("1e3"), 1000.0);
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_float(".5"), 0.5);
        assert_eq!(parse_float("-.5e-1x"), -0.05);
        assert!(parse_float("Q1").is_nan());
        assert!(parse_float(".").is_nan());
        assert_eq!(parse_float("-Infinityx"), f64::NEG_INFINITY);
        assert_eq!(parse_float("2020"), 2020.0);
        assert_eq!(parse_float("3."), 3.0);
    }

    #[test]
    fn fixed2_ties() {
        assert_eq!(fixed2(1.005), 1.0);
        assert_eq!(fixed2(0.125), 0.13);
        assert_eq!(fixed2(-0.001), 0.0);
        assert_eq!(f(12.345678), "12.35");
    }
}
