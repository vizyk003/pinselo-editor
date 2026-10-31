// SPDX-License-Identifier: Apache-2.0
//! JS `Math` semantics the geometry relies on, reproduced bit-for-bit.
//!
//! * Trig goes through the `libm` crate (fdlibm lineage, like V8's `base::ieee754`), so results do not depend on
//!   the platform libm and match the browser.
//! * [`hypot`] is V8's algorithm (scaled, Kahan-compensated), not C `hypot`.
//! * [`fixed3`] is `+n.toFixed(3)` with a fast path that avoids the exact decimal expansion when the value is not
//!   near a rounding tie.

pub use oc_schema::js::{num_to_string, round};

pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// `Math.hypot(a, b)` exactly as V8 computes it.
pub fn hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = a.max(b);
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for v in [a, b] {
        let n = v / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// `Math.sign(x) || fallback` (zero and NaN fall back).
pub fn sign_or(x: f64, fallback: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        fallback
    }
}

/// JS truthiness of a number.
pub fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

/// `Math.min(a, b)` (NaN-propagating, unlike `f64::min`).
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else if a < b || (a == b && a.is_sign_negative()) { a } else { b }
}

/// `Math.max(a, b)` (NaN-propagating, unlike `f64::max`).
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else if a > b || (a == b && b.is_sign_negative()) { a } else { b }
}

/// `+n.toFixed(3)`.
pub fn fixed3(n: f64) -> f64 {
    let a = n.abs();
    if a < 1e9 {
        // |y| < 2^40, so the product is within 2^-12 of the exact value: decide directly unless near a tie.
        let y = a * 1000.0;
        let fl = y.floor();
        let frac = y - fl;
        if (frac - 0.5).abs() > 1e-3 {
            let k = if frac > 0.5 { fl + 1.0 } else { fl };
            if k == 0.0 {
                return 0.0;
            }
            // k / 1000 is correctly rounded, i.e. the double nearest to the decimal string toFixed produced.
            let v = k / 1000.0;
            return if n < 0.0 { -v } else { v };
        }
    }
    oc_schema::js::to_fixed_num(n, 3)
}

/// `${+n.toFixed(3)}` — how the path builders print coordinates.
pub fn f3(n: f64) -> String {
    num_to_string(fixed3(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed3_matches_exact_tofixed() {
        let mut s: u32 = 12345;
        for i in 0..200_000 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let r = f64::from(s) / 4_294_967_296.0;
            let x = match i % 4 {
                0 => (r - 0.5) * 2000.0,
                1 => (r * 1e6).round() / 2000.0, // exact ties
                2 => (r - 0.5) * 1e-2,
                _ => (r - 0.5) * 3e9,
            };
            let exact = oc_schema::js::to_fixed_num(x, 3);
            assert_eq!(fixed3(x).to_bits(), exact.to_bits(), "{x}");
        }
    }

    #[test]
    fn hypot_basic() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, 0.0), 0.0);
        assert_eq!(hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
    }
}
