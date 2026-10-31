// SPDX-License-Identifier: AGPL-3.0-only
//! The numerical routines HyperFormula vendors: jStat (MIT, (c) 2013 jStat) special functions and
//! distributions, and SheetJS bessel.js (Apache-2.0, (c) 2013-present SheetJS LLC) — ported with their
//! JavaScript numeric semantics (ToInt32 truncations, `Math.round`, iteration order).
//!
//! Coefficients are copied digit for digit from the JavaScript sources: over-long literals round to the same
//! f64 in Rust as in JS, and the short approximations those libraries use (`0.70711`, `0.636619772`, …) must
//! stay approximations — the exact `f64::consts` values would change results.
#![allow(clippy::excessive_precision, clippy::approx_constant)]

use super::js_pow;
use crate::jsnum::js_round;

/// JS `ToInt32`.
pub fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc().rem_euclid(4_294_967_296.0);
    (if m >= 2_147_483_648.0 { m - 4_294_967_296.0 } else { m }) as i32
}

/// JS `ToUint32`.
pub fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    x.trunc().rem_euclid(4_294_967_296.0) as u32
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

const PI: f64 = std::f64::consts::PI;

pub fn erf(x: f64) -> f64 {
    const COF: [f64; 28] = [
        -1.3026537197817094,
        6.4196979235649026e-1,
        1.9476473204185836e-2,
        -9.561514786808631e-3,
        -9.46595344482036e-4,
        3.66839497852761e-4,
        4.2523324806907e-5,
        -2.0278578112534e-5,
        -1.624290004647e-6,
        1.303655835580e-6,
        1.5626441722e-8,
        -8.5238095915e-8,
        6.529054439e-9,
        5.059343495e-9,
        -9.91364156e-10,
        -2.27365122e-10,
        9.6467911e-11,
        2.394038e-12,
        -6.886027e-12,
        8.94487e-13,
        3.13092e-13,
        -1.12708e-13,
        3.81e-16,
        7.106e-15,
        -1.523e-15,
        -9.4e-17,
        1.21e-16,
        -2.8e-17,
    ];
    if x == 0.0 {
        return 0.0;
    }
    let (x, neg) = if x < 0.0 { (-x, true) } else { (x, false) };
    let t = 2.0 / (2.0 + x);
    let ty = 4.0 * t - 2.0;
    let (mut d, mut dd) = (0.0, 0.0);
    for j in (1..COF.len()).rev() {
        let tmp = d;
        d = ty * d - dd + COF[j];
        dd = tmp;
    }
    let res = t * (-x * x + 0.5 * (COF[0] + ty * d) - dd).exp();
    if neg { res - 1.0 } else { 1.0 - res }
}

pub fn erfc(x: f64) -> f64 {
    1.0 - erf(x)
}

fn erfcinv(p: f64) -> f64 {
    if p >= 2.0 {
        return -100.0;
    }
    if p <= 0.0 {
        return 100.0;
    }
    let pp = if p < 1.0 { p } else { 2.0 - p };
    let t = (-2.0 * (pp / 2.0).ln()).sqrt();
    let mut x = -0.70711 * ((2.30753 + t * 0.27061) / (1.0 + t * (0.99229 + t * 0.04481)) - t);
    for _ in 0..2 {
        let err = erfc(x) - pp;
        x += err / (1.12837916709551257 * (-x * x).exp() - x * err);
    }
    if p < 1.0 { x } else { -x }
}

pub fn gammafn(x: f64) -> f64 {
    const P: [f64; 8] = [
        -1.716185138865495,
        24.76565080557592,
        -379.80425647094563,
        629.3311553128184,
        866.9662027904133,
        -31451.272968848367,
        -36144.413418691176,
        66456.14382024054,
    ];
    const Q: [f64; 8] = [
        -30.8402300119739,
        315.35062697960416,
        -1015.1563674902192,
        -3107.771671572311,
        22538.118420980151,
        4755.8462775278811,
        -134659.9598649693,
        -115132.2596755535,
    ];
    let mut fact: Option<f64> = None;
    let mut n = 0i64;
    let mut y = x;
    if x > 171.6243769536076 {
        return f64::INFINITY;
    }
    if y <= 0.0 {
        let res = y % 1.0;
        if res != 0.0 && !res.is_nan() {
            fact = Some((if to_int32(y) & 1 == 0 { 1.0 } else { -1.0 }) * PI / (PI * res).sin());
            y = 1.0 - y;
        } else {
            return f64::INFINITY;
        }
    }
    let yi = y;
    let z;
    if y < 1.0 {
        z = y;
        y += 1.0;
    } else {
        n = i64::from(to_int32(y)) - 1;
        y -= n as f64;
        z = y - 1.0;
    }
    let (mut xnum, mut xden) = (0.0, 0.0);
    for i in 0..8 {
        xnum = (xnum + P[i]) * z;
        xden = xden * z + Q[i];
    }
    let mut res = xnum / xden + 1.0;
    if yi < y {
        res /= yi;
    } else if yi > y {
        for _ in 0..n {
            res *= y;
            y += 1.0;
        }
    }
    if let Some(f) = fact {
        res = f / res;
    }
    res
}

pub fn gammaln(x: f64) -> f64 {
    const COF: [f64; 6] = [76.18009172947146, -86.50532032941677, 24.01409824083091, -1.231739572450155, 0.1208650973866179e-2, -0.5395239384953e-5];
    let mut ser = 1.000000000190015;
    let xx = x;
    let mut y = x;
    let mut tmp = x + 5.5;
    tmp -= (xx + 0.5) * tmp.ln();
    for c in COF {
        y += 1.0;
        ser += c / y;
    }
    (2.5066282746310005 * ser / xx).ln() - tmp
}

fn low_reg_gamma(a: f64, x: f64) -> f64 {
    let aln = gammaln(a);
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / 1.0e-30;
    let mut d = 1.0 / b;
    let mut h = d;
    let itmax = to_int32((if a >= 1.0 { a } else { 1.0 / a }).ln() * 8.5 + a * 0.4 + 17.0) + 1;
    if x < 0.0 || a <= 0.0 {
        return f64::NAN;
    } else if x < a + 1.0 {
        for _ in 1..=itmax {
            ap += 1.0;
            del *= x / ap;
            sum += del;
        }
        return sum * (-x + a * x.ln() - aln).exp();
    }
    for i in 1..=itmax {
        let i = f64::from(i);
        let an = -i * (i - a);
        b += 2.0;
        d = an * d + b;
        c = b + an / c;
        d = 1.0 / d;
        h *= d * c;
    }
    1.0 - h * (-x + a * x.ln() - aln).exp()
}

fn gammapinv(p: f64, a: f64) -> f64 {
    let a1 = a - 1.0;
    let eps = 1e-8;
    let gln = gammaln(a);
    let (mut x, mut t);
    let mut lna1 = 0.0;
    let mut afac = 0.0;
    if p >= 1.0 {
        return js_max(100.0, a + 100.0 * a.sqrt());
    }
    if p <= 0.0 {
        return 0.0;
    }
    if a > 1.0 {
        lna1 = a1.ln();
        afac = (a1 * (lna1 - 1.0) - gln).exp();
        let pp = if p < 0.5 { p } else { 1.0 - p };
        t = (-2.0 * pp.ln()).sqrt();
        x = (2.30753 + t * 0.27061) / (1.0 + t * (0.99229 + t * 0.04481)) - t;
        if p < 0.5 {
            x = -x;
        }
        x = js_max(1e-3, a * js_pow(1.0 - 1.0 / (9.0 * a) - x / (3.0 * a.sqrt()), 3.0));
    } else {
        t = 1.0 - a * (0.253 + a * 0.12);
        if p < t {
            x = js_pow(p / t, 1.0 / a);
        } else {
            x = 1.0 - (1.0 - (p - t) / (1.0 - t)).ln();
        }
    }
    for _ in 0..12 {
        if x <= 0.0 {
            return 0.0;
        }
        let err = low_reg_gamma(a, x) - p;
        if a > 1.0 {
            t = afac * (-(x - a1) + a1 * (x.ln() - lna1)).exp();
        } else {
            t = (-x + a1 * x.ln() - gln).exp();
        }
        let u = err / t;
        t = u / (1.0 - 0.5 * js_min(1.0, u * ((a - 1.0) / x - 1.0)));
        x -= t;
        if x <= 0.0 {
            x = 0.5 * (x + t);
        }
        if t.abs() < eps * x {
            break;
        }
    }
    x
}

pub mod exponential {
    pub fn pdf(x: f64, rate: f64) -> f64 {
        if x < 0.0 { 0.0 } else { rate * (-rate * x).exp() }
    }
    pub fn cdf(x: f64, rate: f64) -> f64 {
        if x < 0.0 { 0.0 } else { 1.0 - (-rate * x).exp() }
    }
}

pub mod gamma {
    use super::*;
    pub fn pdf(x: f64, shape: f64, scale: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        if x == 0.0 && shape == 1.0 {
            1.0 / scale
        } else {
            ((shape - 1.0) * x.ln() - x / scale - gammaln(shape) - shape * scale.ln()).exp()
        }
    }
    pub fn cdf(x: f64, shape: f64, scale: f64) -> f64 {
        if x < 0.0 { 0.0 } else { low_reg_gamma(shape, x / scale) }
    }
    pub fn inv(p: f64, shape: f64, scale: f64) -> f64 {
        gammapinv(p, shape) * scale
    }
}

pub mod normal {
    use super::*;
    pub fn pdf(x: f64, mean: f64, std: f64) -> f64 {
        (-0.5 * (2.0 * PI).ln() - std.ln() - js_pow(x - mean, 2.0) / (2.0 * std * std)).exp()
    }
    pub fn cdf(x: f64, mean: f64, std: f64) -> f64 {
        0.5 * (1.0 + erf((x - mean) / (2.0 * std * std).sqrt()))
    }
    pub fn inv(p: f64, mean: f64, std: f64) -> f64 {
        -1.41421356237309505 * std * erfcinv(2.0 * p) + mean
    }
}

fn betafn(x: f64, y: f64) -> f64 {
    if x <= 0.0 || y <= 0.0 {
        return f64::NAN;
    }
    if x + y > 170.0 { betaln(x, y).exp() } else { gammafn(x) * gammafn(y) / gammafn(x + y) }
}

fn betaln(x: f64, y: f64) -> f64 {
    gammaln(x) + gammaln(y) - gammaln(x + y)
}

fn ibetainv(p: f64, a: f64, b: f64) -> f64 {
    let eps = 1e-8;
    let a1 = a - 1.0;
    let b1 = b - 1.0;
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let mut x;
    if a >= 1.0 && b >= 1.0 {
        let pp = if p < 0.5 { p } else { 1.0 - p };
        let t = (-2.0 * pp.ln()).sqrt();
        x = (2.30753 + t * 0.27061) / (1.0 + t * (0.99229 + t * 0.04481)) - t;
        if p < 0.5 {
            x = -x;
        }
        let al = (x * x - 3.0) / 6.0;
        let h = 2.0 / (1.0 / (2.0 * a - 1.0) + 1.0 / (2.0 * b - 1.0));
        let w = x * (al + h).sqrt() / h - (1.0 / (2.0 * b - 1.0) - 1.0 / (2.0 * a - 1.0)) * (al + 5.0 / 6.0 - 2.0 / (3.0 * h));
        x = a / (a + b * (2.0 * w).exp());
    } else {
        let lna = (a / (a + b)).ln();
        let lnb = (b / (a + b)).ln();
        let t = (a * lna).exp() / a;
        let u = (b * lnb).exp() / b;
        let w = t + u;
        if p < t / w {
            x = js_pow(a * w * p, 1.0 / a);
        } else {
            x = 1.0 - js_pow(b * w * (1.0 - p), 1.0 / b);
        }
    }
    let afac = -gammaln(a) - gammaln(b) + gammaln(a + b);
    for j in 0..10 {
        if x == 0.0 || x == 1.0 {
            return x;
        }
        let err = ibeta(x, a, b) - p;
        let mut t = (a1 * x.ln() + b1 * (1.0 - x).ln() + afac).exp();
        let u = err / t;
        t = u / (1.0 - 0.5 * js_min(1.0, u * (a1 / x - b1 / (1.0 - x))));
        x -= t;
        if x <= 0.0 {
            x = 0.5 * (x + t);
        }
        if x >= 1.0 {
            x = 0.5 * (x + t + 1.0);
        }
        if t.abs() < eps * x && j > 0 {
            break;
        }
    }
    x
}

/// Regularised incomplete beta (`ibeta`; jStat returns `false` outside [0, 1] — NaN here).
fn ibeta(x: f64, a: f64, b: f64) -> f64 {
    let bt = if x == 0.0 || x == 1.0 { 0.0 } else { (gammaln(a + b) - gammaln(a) - gammaln(b) + a * x.ln() + b * (1.0 - x).ln()).exp() };
    if !(0.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    if x < (a + 1.0) / (a + b + 2.0) {
        return bt * betacf(x, a, b) / a;
    }
    1.0 - bt * betacf(1.0 - x, b, a) / b
}

fn betacf(x: f64, a: f64, b: f64) -> f64 {
    let fpmin = 1e-30;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < fpmin {
        d = fpmin;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=100 {
        let m = f64::from(m);
        let m2 = 2.0 * m;
        let mut aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < fpmin {
            d = fpmin;
        }
        c = 1.0 + aa / c;
        if c.abs() < fpmin {
            c = fpmin;
        }
        d = 1.0 / d;
        h *= d * c;
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < fpmin {
            d = fpmin;
        }
        c = 1.0 + aa / c;
        if c.abs() < fpmin {
            c = fpmin;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 3e-7 {
            break;
        }
    }
    h
}

pub mod beta {
    use super::*;
    pub fn pdf(x: f64, alpha: f64, beta: f64) -> f64 {
        if !(0.0..=1.0).contains(&x) {
            return 0.0;
        }
        if alpha == 1.0 && beta == 1.0 {
            return 1.0;
        }
        if alpha < 512.0 && beta < 512.0 {
            js_pow(x, alpha - 1.0) * js_pow(1.0 - x, beta - 1.0) / betafn(alpha, beta)
        } else {
            ((alpha - 1.0) * x.ln() + (beta - 1.0) * (1.0 - x).ln() - betaln(alpha, beta)).exp()
        }
    }
    pub fn cdf(x: f64, alpha: f64, beta: f64) -> f64 {
        if x > 1.0 {
            1.0
        } else if x < 0.0 {
            0.0
        } else {
            ibeta(x, alpha, beta)
        }
    }
    pub fn inv(x: f64, alpha: f64, beta: f64) -> f64 {
        ibetainv(x, alpha, beta)
    }
}

fn betinc(x: f64, a: f64, b: f64, eps: f64) -> f64 {
    let (mut a0, mut b0, mut a1, mut b1, mut m9, mut a2): (f64, f64, f64, f64, f64, f64) = (0.0, 1.0, 1.0, 1.0, 0.0, 0.0);
    while ((a1 - a2) / a1).abs() > eps {
        a2 = a1;
        let mut c9 = -(a + m9) * (a + b + m9) * x / (a + 2.0 * m9) / (a + 2.0 * m9 + 1.0);
        a0 = a1 + c9 * a0;
        b0 = b1 + c9 * b0;
        m9 += 1.0;
        c9 = m9 * (b - m9) * x / (a + 2.0 * m9 - 1.0) / (a + 2.0 * m9);
        a1 = a0 + c9 * a1;
        b1 = b0 + c9 * b1;
        a0 /= b1;
        b0 /= b1;
        a1 /= b1;
        b1 = 1.0;
    }
    a1 / a
}

pub fn factorialln(n: f64) -> f64 {
    if n < 0.0 { f64::NAN } else { gammaln(n + 1.0) }
}

pub fn factorial(n: f64) -> f64 {
    if n < 0.0 { f64::NAN } else { gammafn(n + 1.0) }
}

fn combination(n: f64, m: f64) -> f64 {
    if n > 170.0 || m > 170.0 {
        (factorialln(n) - factorialln(m) - factorialln(n - m)).exp()
    } else {
        factorial(n) / factorial(m) / factorial(n - m)
    }
}

pub mod binomial {
    use super::*;
    pub fn pdf(k: f64, n: f64, p: f64) -> f64 {
        if p == 0.0 || p == 1.0 {
            if n * p == k { 1.0 } else { 0.0 }
        } else {
            combination(n, k) * js_pow(p, k) * js_pow(1.0 - p, n - k)
        }
    }
    pub fn cdf(x: f64, n: f64, p: f64) -> f64 {
        let eps = 1e-10;
        if x < 0.0 {
            return 0.0;
        }
        if x >= n {
            return 1.0;
        }
        if !(0.0..=1.0).contains(&p) || n <= 0.0 {
            return f64::NAN;
        }
        let x = x.floor();
        let z = p;
        let a = x + 1.0;
        let b = n - x;
        let s = a + b;
        let bt = (gammaln(s) - gammaln(b) - gammaln(a) + a * z.ln() + b * (1.0 - z).ln()).exp();
        let betacdf = if z < (a + 1.0) / (s + 2.0) { bt * betinc(z, a, b, eps) } else { 1.0 - bt * betinc(1.0 - z, b, a, eps) };
        js_round((1.0 - betacdf) * (1.0 / eps)) / (1.0 / eps)
    }
}

pub mod chisquare {
    use super::*;
    pub fn pdf(x: f64, dof: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        if x == 0.0 && dof == 2.0 {
            0.5
        } else {
            ((dof / 2.0 - 1.0) * x.ln() - x / 2.0 - dof / 2.0 * 2f64.ln() - gammaln(dof / 2.0)).exp()
        }
    }
    pub fn cdf(x: f64, dof: f64) -> f64 {
        if x < 0.0 { 0.0 } else { low_reg_gamma(dof / 2.0, x / 2.0) }
    }
    pub fn inv(p: f64, dof: f64) -> f64 {
        2.0 * gammapinv(p, 0.5 * dof)
    }
}

pub mod central_f {
    use super::*;
    pub fn pdf(x: f64, df1: f64, df2: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        if df1 <= 2.0 {
            if x == 0.0 && df1 < 2.0 {
                return f64::INFINITY;
            }
            if x == 0.0 && df1 == 2.0 {
                return 1.0;
            }
            return 1.0 / betafn(df1 / 2.0, df2 / 2.0)
                * js_pow(df1 / df2, df1 / 2.0)
                * js_pow(x, df1 / 2.0 - 1.0)
                * js_pow(1.0 + df1 / df2 * x, -(df1 + df2) / 2.0);
        }
        let p = df1 * x / (df2 + x * df1);
        let q = df2 / (df2 + x * df1);
        let f = df1 * q / 2.0;
        f * binomial::pdf((df1 - 2.0) / 2.0, (df1 + df2 - 2.0) / 2.0, p)
    }
    pub fn cdf(x: f64, df1: f64, df2: f64) -> f64 {
        if x < 0.0 { 0.0 } else { ibeta(df1 * x / (df1 * x + df2), df1 / 2.0, df2 / 2.0) }
    }
    pub fn inv(x: f64, df1: f64, df2: f64) -> f64 {
        df2 / (df1 * (1.0 / ibetainv(x, df1 / 2.0, df2 / 2.0) - 1.0))
    }
}

pub mod weibull {
    use super::*;
    pub fn pdf(x: f64, scale: f64, shape: f64) -> f64 {
        if x < 0.0 || scale < 0.0 || shape < 0.0 {
            return 0.0;
        }
        shape / scale * js_pow(x / scale, shape - 1.0) * (-js_pow(x / scale, shape)).exp()
    }
    pub fn cdf(x: f64, scale: f64, shape: f64) -> f64 {
        if x < 0.0 { 0.0 } else { 1.0 - (-js_pow(x / scale, shape)).exp() }
    }
}

pub mod poisson {
    use super::*;
    pub fn pdf(k: f64, l: f64) -> f64 {
        if l < 0.0 || k % 1.0 != 0.0 || k < 0.0 {
            return 0.0;
        }
        js_pow(l, k) * (-l).exp() / factorial(k)
    }
    pub fn cdf(x: f64, l: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        let mut sum = 0.0;
        let mut k = 0.0;
        while k <= x {
            sum += pdf(k, l);
            k += 1.0;
        }
        sum
    }
}

pub mod hypgeom {
    use super::*;
    #[allow(non_snake_case)]
    pub fn pdf(k: f64, N: f64, m: f64, n: f64) -> f64 {
        if k < 0.0 || k < m - (N - n) || k > n || k > m {
            return 0.0;
        }
        if m * 2.0 > N {
            if n * 2.0 > N {
                return pdf(N - m - n + k, N, N - m, N - n);
            }
            return pdf(n - k, N, N - m, n);
        }
        if n * 2.0 > N {
            return pdf(m - k, N, m, N - n);
        }
        if m < n {
            return pdf(k, N, n, m);
        }
        let mut scaled = 1.0;
        let mut done = 0.0;
        let mut i = 0.0;
        while i < k {
            while scaled > 1.0 && done < n {
                scaled *= 1.0 - m / (N - done);
                done += 1.0;
            }
            scaled *= (n - i) * (m - i) / ((i + 1.0) * (N - m - n + i + 1.0));
            i += 1.0;
        }
        while done < n {
            scaled *= 1.0 - m / (N - done);
            done += 1.0;
        }
        js_min(1.0, js_max(0.0, scaled))
    }
    #[allow(non_snake_case)]
    pub fn cdf(x: f64, N: f64, m: f64, n: f64) -> f64 {
        if x < 0.0 || x < m - (N - n) {
            return 0.0;
        }
        if x >= n || x >= m {
            return 1.0;
        }
        if m * 2.0 > N {
            if n * 2.0 > N {
                return cdf(N - m - n + x, N, N - m, N - n);
            }
            return 1.0 - cdf(n - x - 1.0, N, N - m, n);
        }
        if n * 2.0 > N {
            return 1.0 - cdf(m - x - 1.0, N, m, N - n);
        }
        if m < n {
            return cdf(x, N, n, m);
        }
        let mut scaled_cdf = 1.0;
        let mut scaled_pdf = 1.0;
        let mut done = 0.0;
        let mut i = 0.0;
        while i < x {
            while scaled_cdf > 1.0 && done < n {
                let factor = 1.0 - m / (N - done);
                scaled_pdf *= factor;
                scaled_cdf *= factor;
                done += 1.0;
            }
            scaled_pdf *= (n - i) * (m - i) / ((i + 1.0) * (N - m - n + i + 1.0));
            scaled_cdf += scaled_pdf;
            i += 1.0;
        }
        while done < n {
            scaled_cdf *= 1.0 - m / (N - done);
            done += 1.0;
        }
        js_min(1.0, js_max(0.0, scaled_cdf))
    }
}

pub mod studentt {
    use super::*;
    pub fn pdf(x: f64, dof: f64) -> f64 {
        let dof = if dof > 1e100 { 1e100 } else { dof };
        1.0 / (dof.sqrt() * betafn(0.5, dof / 2.0)) * js_pow(1.0 + x * x / dof, -((dof + 1.0) / 2.0))
    }
    pub fn cdf(x: f64, dof: f64) -> f64 {
        let dof2 = dof / 2.0;
        ibeta((x + (x * x + dof).sqrt()) / (2.0 * (x * x + dof).sqrt()), dof2, dof2)
    }
    pub fn inv(p: f64, dof: f64) -> f64 {
        let x = ibetainv(2.0 * js_min(p, 1.0 - p), 0.5 * dof, 0.5);
        let x = (dof * (1.0 - x) / x).sqrt();
        if p > 0.5 { x } else { -x }
    }
}

pub mod lognormal {
    use super::*;
    pub fn pdf(x: f64, mu: f64, sigma: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        (-x.ln() - 0.5 * (2.0 * PI).ln() - sigma.ln() - js_pow(x.ln() - mu, 2.0) / (2.0 * sigma * sigma)).exp()
    }
    pub fn cdf(x: f64, mu: f64, sigma: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        0.5 + 0.5 * erf((x.ln() - mu) / (2.0 * sigma * sigma).sqrt())
    }
    pub fn inv(p: f64, mu: f64, sigma: f64) -> f64 {
        (-1.41421356237309505 * sigma * erfcinv(2.0 * p) + mu).exp()
    }
}

pub mod negbin {
    use super::*;
    /// `None` = jStat's `false` (k not a uint32).
    pub fn pdf(k: f64, r: f64, p: f64) -> Option<f64> {
        if k != f64::from(to_uint32(k)) {
            return None;
        }
        if k < 0.0 {
            return Some(0.0);
        }
        Some(combination(k + r - 1.0, r - 1.0) * js_pow(1.0 - p, k) * js_pow(p, r))
    }
    pub fn cdf(x: f64, r: f64, p: f64) -> f64 {
        if x < 0.0 {
            return 0.0;
        }
        let mut sum = 0.0;
        let mut k = 0.0;
        while k <= x {
            sum += pdf(k, r, p).unwrap_or(0.0);
            k += 1.0;
        }
        sum
    }
}

/// `normalci(1, alpha, sd, n)[1]`.
pub fn normalci_hi(value: f64, alpha: f64, sd: f64, n: f64) -> f64 {
    let change = (normal::inv(alpha / 2.0, 0.0, 1.0) * sd / n.sqrt()).abs();
    value + change
}

/// `tci(1, alpha, sd, n)[1]`.
pub fn tci_hi(value: f64, alpha: f64, sd: f64, n: f64) -> f64 {
    let change = (studentt::inv(alpha / 2.0, n - 1.0) * sd / n.sqrt()).abs();
    value + change
}

// ------------------------------------------------------------------ bessel.js

fn horner(arr: &[f64], v: f64) -> f64 {
    let mut z = 0.0;
    for a in arr {
        z = v * z + a;
    }
    z
}

fn bessel_iter(x: f64, n: f64, f0: f64, f1: f64, sign: f64) -> f64 {
    if n == 0.0 {
        return f0;
    }
    if n == 1.0 {
        return f1;
    }
    let tdx = 2.0 / x;
    let (mut f0, mut f1, mut f2) = (f0, f1, f1);
    let mut o = 1.0;
    while o < n {
        f2 = f1 * o * tdx + sign * f0;
        f0 = f1;
        f1 = f2;
        o += 1.0;
    }
    f2
}

fn rev<const N: usize>(a: [f64; N]) -> [f64; N] {
    let mut a = a;
    a.reverse();
    a
}

const W: f64 = 0.636619772;

fn besselj0(x: f64) -> f64 {
    let a1a = rev([57568490574.0, -13362590354.0, 651619640.7, -11214424.18, 77392.33017, -184.9052456]);
    let a2a = rev([57568490411.0, 1029532985.0, 9494680.718, 59272.64853, 267.8532712, 1.0]);
    let a1b = rev([1.0, -0.1098628627e-2, 0.2734510407e-4, -0.2073370639e-5, 0.2093887211e-6]);
    let a2b = rev([-0.1562499995e-1, 0.1430488765e-3, -0.6911147651e-5, 0.7621095161e-6, -0.934935152e-7]);
    let y = x * x;
    if x < 8.0 {
        horner(&a1a, y) / horner(&a2a, y)
    } else {
        let xx = x - 0.785398164;
        let y = 64.0 / y;
        (W / x).sqrt() * (xx.cos() * horner(&a1b, y) - xx.sin() * horner(&a2b, y) * 8.0 / x)
    }
}

fn besselj1(x: f64) -> f64 {
    let a1a = rev([72362614232.0, -7895059235.0, 242396853.1, -2972611.439, 15704.48260, -30.16036606]);
    let a2a = rev([144725228442.0, 2300535178.0, 18583304.74, 99447.43394, 376.9991397, 1.0]);
    let a1b = rev([1.0, 0.183105e-2, -0.3516396496e-4, 0.2457520174e-5, -0.240337019e-6]);
    let a2b = rev([0.04687499995, -0.2002690873e-3, 0.8449199096e-5, -0.88228987e-6, 0.105787412e-6]);
    let y = x * x;
    let xx = x.abs() - 2.356194491;
    if x.abs() < 8.0 {
        x * horner(&a1a, y) / horner(&a2a, y)
    } else {
        let y = 64.0 / y;
        let a = (W / x.abs()).sqrt() * (xx.cos() * horner(&a1b, y) - xx.sin() * horner(&a2b, y) * 8.0 / x.abs());
        if x < 0.0 { -a } else { a }
    }
}

pub fn besselj(x: f64, n: f64) -> f64 {
    let n = js_round(n);
    if !x.is_finite() {
        return if x.is_nan() { x } else { 0.0 };
    }
    let odd = n % 2.0 != 0.0 && !(n % 2.0).is_nan();
    if n < 0.0 {
        return (if odd { -1.0 } else { 1.0 }) * besselj(x, -n);
    }
    if x < 0.0 {
        return (if odd { -1.0 } else { 1.0 }) * besselj(-x, n);
    }
    if n == 0.0 {
        return besselj0(x);
    }
    if n == 1.0 {
        return besselj1(x);
    }
    if x == 0.0 {
        return 0.0;
    }
    if x > n {
        return bessel_iter(x, n, besselj0(x), besselj1(x), -1.0);
    }
    let m = 2.0 * ((n + (40.0 * n).sqrt().floor()) / 2.0).floor();
    let mut jsum = false;
    let (mut bjp, mut sum, mut bj, mut ret) = (0.0, 0.0, 1.0, 0.0);
    let tox = 2.0 / x;
    let mut j = m;
    while j > 0.0 {
        let bjm = j * tox * bj - bjp;
        bjp = bj;
        bj = bjm;
        if bj.abs() > 1e10 {
            bj *= 1e-10;
            bjp *= 1e-10;
            ret *= 1e-10;
            sum *= 1e-10;
        }
        if jsum {
            sum += bj;
        }
        jsum = !jsum;
        if j == n {
            ret = bjp;
        }
        j -= 1.0;
    }
    sum = 2.0 * sum - bj;
    ret / sum
}

fn bessely0(x: f64) -> f64 {
    let a1a = rev([-2957821389.0, 7062834065.0, -512359803.6, 10879881.29, -86327.92757, 228.4622733]);
    let a2a = rev([40076544269.0, 745249964.8, 7189466.438, 47447.26470, 226.1030244, 1.0]);
    let a1b = rev([1.0, -0.1098628627e-2, 0.2734510407e-4, -0.2073370639e-5, 0.2093887211e-6]);
    let a2b = rev([-0.1562499995e-1, 0.1430488765e-3, -0.6911147651e-5, 0.7621095161e-6, -0.934945152e-7]);
    let y = x * x;
    let xx = x - 0.785398164;
    if x < 8.0 {
        horner(&a1a, y) / horner(&a2a, y) + W * besselj(x, 0.0) * x.ln()
    } else {
        let y = 64.0 / y;
        (W / x).sqrt() * (xx.sin() * horner(&a1b, y) + xx.cos() * horner(&a2b, y) * 8.0 / x)
    }
}

fn bessely1(x: f64) -> f64 {
    let a1a = rev([-0.4900604943e13, 0.1275274390e13, -0.5153438139e11, 0.7349264551e9, -0.4237922726e7, 0.8511937935e4]);
    let a2a = rev([0.2499580570e14, 0.4244419664e12, 0.3733650367e10, 0.2245904002e8, 0.1020426050e6, 0.3549632885e3, 1.0]);
    let a1b = rev([1.0, 0.183105e-2, -0.3516396496e-4, 0.2457520174e-5, -0.240337019e-6]);
    let a2b = rev([0.04687499995, -0.2002690873e-3, 0.8449199096e-5, -0.88228987e-6, 0.105787412e-6]);
    let y = x * x;
    let xx = x - 2.356194491;
    if x < 8.0 {
        x * horner(&a1a, y) / horner(&a2a, y) + W * (besselj(x, 1.0) * x.ln() - 1.0 / x)
    } else {
        let y = 64.0 / y;
        (W / x).sqrt() * (xx.sin() * horner(&a1b, y) + xx.cos() * horner(&a2b, y) * 8.0 / x)
    }
}

/// `_bessel_wrap(b0, b1, name, nonzero, sign)`.
fn bessel_wrap(x: f64, n: f64, b0: fn(f64) -> f64, b1: fn(f64) -> f64, nonzero: u8, sign: f64) -> f64 {
    if nonzero > 0 {
        if x == 0.0 {
            return if nonzero == 1 { f64::NEG_INFINITY } else { f64::INFINITY };
        } else if x < 0.0 {
            return f64::NAN;
        }
    }
    if n == 0.0 {
        return b0(x);
    }
    if n == 1.0 {
        return b1(x);
    }
    if n < 0.0 {
        return f64::NAN;
    }
    let n = f64::from(to_int32(n));
    bessel_iter(x, n, b0(x), b1(x), sign)
}

pub fn bessely(x: f64, n: f64) -> f64 {
    bessel_wrap(x, n, bessely0, bessely1, 1, -1.0)
}

fn besseli0(x: f64) -> f64 {
    let a = rev([1.0, 3.5156229, 3.0899424, 1.2067492, 0.2659732, 0.360768e-1, 0.45813e-2]);
    let b = rev([0.39894228, 0.1328592e-1, 0.225319e-2, -0.157565e-2, 0.916281e-2, -0.2057706e-1, 0.2635537e-1, -0.1647633e-1, 0.392377e-2]);
    if x <= 3.75 {
        return horner(&a, x * x / (3.75 * 3.75));
    }
    x.abs().exp() / x.abs().sqrt() * horner(&b, 3.75 / x.abs())
}

fn besseli1(x: f64) -> f64 {
    let a = rev([0.5, 0.87890594, 0.51498869, 0.15084934, 0.2658733e-1, 0.301532e-2, 0.32411e-3]);
    let b = rev([0.39894228, -0.3988024e-1, -0.362018e-2, 0.163801e-2, -0.1031555e-1, 0.2282967e-1, -0.2895312e-1, 0.1787654e-1, -0.420059e-2]);
    if x < 3.75 {
        return x * horner(&a, x * x / (3.75 * 3.75));
    }
    (if x < 0.0 { -1.0 } else { 1.0 }) * x.abs().exp() / x.abs().sqrt() * horner(&b, 3.75 / x.abs())
}

pub fn besseli(x: f64, n: f64) -> f64 {
    let n = js_round(n);
    if n == 0.0 {
        return besseli0(x);
    }
    if n == 1.0 {
        return besseli1(x);
    }
    if n < 0.0 {
        return f64::NAN;
    }
    if x.abs() == 0.0 {
        return 0.0;
    }
    if x == f64::INFINITY {
        return f64::INFINITY;
    }
    let tox = 2.0 / x.abs();
    let (mut ret, mut bip, mut bi) = (0.0, 0.0, 1.0);
    let m = 2.0 * js_round((n + js_round((40.0 * n).sqrt())) / 2.0);
    let mut j = m;
    while j > 0.0 {
        let bim = j * tox * bi + bip;
        bip = bi;
        bi = bim;
        if bi.abs() > 1e10 {
            bi *= 1e-10;
            bip *= 1e-10;
            ret *= 1e-10;
        }
        if j == n {
            ret = bip;
        }
        j -= 1.0;
    }
    ret *= besseli(x, 0.0) / bi;
    let odd = n % 2.0 != 0.0 && !(n % 2.0).is_nan();
    if x < 0.0 && odd { -ret } else { ret }
}

fn besselk0(x: f64) -> f64 {
    let a = rev([-0.57721566, 0.42278420, 0.23069756, 0.3488590e-1, 0.262698e-2, 0.10750e-3, 0.74e-5]);
    let b = rev([1.25331414, -0.7832358e-1, 0.2189568e-1, -0.1062446e-1, 0.587872e-2, -0.251540e-2, 0.53208e-3]);
    if x <= 2.0 {
        return -(x / 2.0).ln() * besseli(x, 0.0) + horner(&a, x * x / 4.0);
    }
    (-x).exp() / x.sqrt() * horner(&b, 2.0 / x)
}

fn besselk1(x: f64) -> f64 {
    let a = rev([1.0, 0.15443144, -0.67278579, -0.18156897, -0.1919402e-1, -0.110404e-2, -0.4686e-4]);
    let b = rev([1.25331414, 0.23498619, -0.3655620e-1, 0.1504268e-1, -0.780353e-2, 0.325614e-2, -0.68245e-3]);
    if x <= 2.0 {
        return (x / 2.0).ln() * besseli(x, 1.0) + 1.0 / x * horner(&a, x * x / 4.0);
    }
    (-x).exp() / x.sqrt() * horner(&b, 2.0 / x)
}

pub fn besselk(x: f64, n: f64) -> f64 {
    bessel_wrap(x, n, besselk0, besselk1, 2, 1.0)
}
