// SPDX-License-Identifier: AGPL-3.0-only
//! Math plugins: Abs, Exp, Logarithm, Power, Sqrt, Modulo, MathConstants, Radians, Degrees, IsEven/IsOdd,
//! Random, Math (FACT, COMBIN, GCD, …), Rounding, Trigonometry, Bitwise/BitShift, Delta, RadixConversion.

use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, Scalar, Val};
use super::{Kind, err, js_pow, num, numbers_coerce_ranges, s, simple};
use crate::jsnum::js_round;

/// `parseFloat(Math.PI.toFixed(14))`: HyperFormula's PI() is this truncated value, not `f64::consts::PI`.
#[allow(clippy::approx_constant)]
pub const PI: f64 = 3.14159265358979;

fn f1(a: &[Arg]) -> f64 {
    a[0].num()
}

fn combin(n: f64, m: f64) -> f64 {
    let m = if 2.0 * m > n { n - m } else { m };
    let mut ret = 1.0;
    let mut i = 1.0;
    while i <= m {
        ret *= (n - m + i) / i;
        i += 1.0;
    }
    js_round(ret)
}

fn gcd2(a: f64, b: f64) -> f64 {
    let (mut a, mut b) = if a < b { (b, a) } else { (a, b) };
    while b > 0.0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

fn lcm2(a: f64, b: f64) -> f64 {
    if a == 0.0 || b == 0.0 {
        return 0.0;
    }
    a * (b / gcd2(a, b))
}

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

fn gcd(_: &Call, a: &[Arg]) -> Val {
    let vals = match numbers_coerce_ranges(a) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let mut ret = 0.0;
    for v in vals {
        if v < 0.0 {
            return err(ErrorType::Num);
        }
        ret = gcd2(ret, v.trunc());
    }
    if ret > MAX_SAFE { err(ErrorType::Num) } else { num(ret) }
}

fn lcm(_: &Call, a: &[Arg]) -> Val {
    let vals = match numbers_coerce_ranges(a) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let mut ret = 1.0;
    for v in vals {
        if v < 0.0 {
            return err(ErrorType::Num);
        }
        ret = lcm2(ret, v.trunc());
    }
    if ret > MAX_SAFE { err(ErrorType::Num) } else { num(ret) }
}

fn mround(_: &Call, a: &[Arg]) -> Val {
    let (n, d) = (a[0].num(), a[1].num());
    if d == 0.0 {
        return num(0.0);
    }
    if (n > 0.0 && d < 0.0) || (n < 0.0 && d > 0.0) {
        return err(ErrorType::Num);
    }
    num(js_round(n / d) * d)
}

fn multinomial(_: &Call, a: &[Arg]) -> Val {
    let mut n = 0.0;
    let mut ans = 1.0;
    for x in a {
        let mut v = x.num();
        if v < 0.0 {
            return err(ErrorType::Num);
        }
        v = v.trunc();
        let mut i = 1.0;
        while i <= v {
            ans *= (n + i) / i;
            i += 1.0;
        }
        n += v;
    }
    num(js_round(ans))
}

fn seriessum(_: &Call, a: &[Arg]) -> Val {
    let (x, n, m) = (a[0].num(), a[1].num(), a[2].num());
    let r = a[3].to_range();
    let mut coefs = Vec::new();
    for v in r.values() {
        match v {
            Scalar::Err(e) => return err(*e),
            Scalar::Num(c, _) => coefs.push(*c),
            Scalar::Empty => {}
            _ => return err(ErrorType::Value),
        }
    }
    coefs.reverse();
    let mut ret = 0.0;
    for c in coefs {
        ret *= js_pow(x, m);
        ret += c;
    }
    num(ret * js_pow(x, n))
}

fn sumx(a: &[Arg], f: fn(f64, f64) -> f64) -> Val {
    let x: Vec<Scalar> = a[0].to_range().values().cloned().collect();
    let y: Vec<Scalar> = a[1].to_range().values().cloned().collect();
    if x.len() != y.len() {
        return err(ErrorType::Na);
    }
    let mut ret = 0.0;
    for (vx, vy) in x.iter().zip(&y) {
        if let Scalar::Err(e) = vx {
            return err(*e);
        }
        if let Scalar::Err(e) = vy {
            return err(*e);
        }
        if let (Scalar::Num(p, super::super::value::NumType::Raw), Scalar::Num(q, super::super::value::NumType::Raw)) = (vx, vy) {
            ret += f(*p, *q);
        }
    }
    num(ret)
}

fn round_with(a: &[Arg], f: fn(f64) -> f64) -> Val {
    let (v, places) = (a[0].num(), a[1].num());
    let m = js_pow(10.0, places);
    if v < 0.0 { num(-f(-v * m) / m) } else { num(f(v * m) / m) }
}

fn next_odd(x: f64) -> f64 {
    let c = x.ceil();
    if c % 2.0 == 1.0 { c } else { c + 1.0 }
}

fn next_even(x: f64) -> f64 {
    let c = x.ceil();
    if c % 2.0 == 0.0 { c } else { c + 1.0 }
}

const MAX_48: f64 = 281_474_976_710_655.0;

fn shift_left(v: f64, p: f64) -> Val {
    if p < 0.0 {
        return shift_right(v, -p);
    }
    let r = v * js_pow(2.0, p);
    if r > MAX_48 { err(ErrorType::Num) } else { num(r) }
}

fn shift_right(v: f64, p: f64) -> Val {
    if p < 0.0 {
        return shift_left(v, -p);
    }
    let r = (v / js_pow(2.0, p)).floor();
    if r > MAX_48 { err(ErrorType::Num) } else { num(r) }
}

/// JS `ToInt32`.
fn to_i32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc().rem_euclid(4_294_967_296.0);
    m as u32 as i32
}

// ------------------------------------------------------------------ radix

const ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const MAX_LENGTH: i32 = 10;

/// V8 `DoubleToRadixCString` (`Number#toString(radix)`).
pub fn to_radix_string(value: f64, radix: u32) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if radix == 10 {
        return crate::jsnum::num_to_string(value);
    }
    let chars: Vec<u8> = b"0123456789abcdefghijklmnopqrstuvwxyz".to_vec();
    let negative = value < 0.0;
    let value = value.abs();
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let next = f64::from_bits(value.to_bits() + 1);
    let mut delta = (0.5 * (next - value)).max(f64::from_bits(1));
    let r = radix as f64;
    let mut frac_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= r;
            delta *= r;
            let digit = fraction as usize;
            frac_digits.push(chars[digit]);
            fraction -= digit as f64;
            if (fraction > 0.5 || (fraction == 0.5 && digit & 1 == 1)) && fraction + delta > 1.0 {
                loop {
                    match frac_digits.pop() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(c) => {
                            let d = if c > b'9' { (c - b'a' + 10) as u32 } else { (c - b'0') as u32 };
                            if d + 1 < radix {
                                frac_digits.push(chars[(d + 1) as usize]);
                                break;
                            }
                        }
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut int_digits: Vec<u8> = Vec::new();
    // Fill unrepresented digits with zero (while integer / radix >= 2^53).
    while integer / r >= 9_007_199_254_740_992.0 {
        integer /= r;
        int_digits.push(b'0');
    }
    loop {
        let rem = integer % r;
        int_digits.push(chars[rem as usize]);
        integer = (integer - rem) / r;
        if integer <= 0.0 {
            break;
        }
    }
    int_digits.reverse();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&int_digits).unwrap_or("0"));
    if !frac_digits.is_empty() {
        out.push('.');
        out.push_str(std::str::from_utf8(&frac_digits).unwrap_or(""));
    }
    out
}

fn coerce_to_base(v: &str, base: u32, max_len: usize) -> Option<String> {
    let alpha = &ALPHABET[..base as usize];
    if super::js_len(v) > max_len || v.is_empty() || !v.chars().all(|c| alpha.contains(c)) {
        return None;
    }
    Some(v.to_string())
}

/// JS `parseInt(str, base)` for strings already validated against the alphabet.
fn parse_int(v: &str, base: u32) -> f64 {
    let mut n = 0.0;
    for c in v.chars() {
        let d = ALPHABET.find(c.to_ascii_uppercase()).unwrap_or(0) as f64;
        n = n * base as f64 + d;
    }
    n
}

fn radix_complement(v: f64, base: u32) -> String {
    let offset = if v < 0.0 { js_pow(base as f64, MAX_LENGTH as f64) } else { 0.0 };
    to_radix_string(v + offset, base).to_uppercase()
}

fn two_complement(v: &str, base: u32) -> f64 {
    let parsed = parse_int(v, base);
    let offset = js_pow(base as f64, MAX_LENGTH as f64);
    if parsed >= offset / 2.0 { parsed - offset } else { parsed }
}

fn pad_left(v: &str, size: f64) -> String {
    let len = super::js_len(v) as f64;
    if len >= size { v.to_string() } else { format!("{}{v}", "0".repeat((size - len) as usize)) }
}

fn to_base_exact(v: f64, base: u32, places: Option<f64>) -> Val {
    let min = -js_pow(base as f64, MAX_LENGTH as f64) / 2.0;
    let max = -min - 1.0;
    if v > max || v < min {
        return err(ErrorType::Num);
    }
    let r = radix_complement(v, base);
    match places {
        None => s(r),
        Some(_) if v < 0.0 => s(r),
        Some(p) if super::js_len(&r) as f64 > p => err(ErrorType::Num),
        Some(p) => s(pad_left(&r, p)),
    }
}

fn from_base(a: &[Arg], base: u32) -> Result<f64, ErrorType> {
    coerce_to_base(a[0].str(), base, MAX_LENGTH as usize).map(|v| two_complement(&v, base)).ok_or(ErrorType::Num)
}

fn conv(a: &[Arg], from: u32, to: u32) -> Val {
    match from_base(a, from) {
        Ok(v) => to_base_exact(v, to, a.get(1).and_then(Arg::opt_num)),
        Err(e) => err(e),
    }
}

fn base_fn(_: &Call, a: &[Arg]) -> Val {
    let (v, base) = (a[0].num(), a[1].num() as u32);
    let r = radix_complement(v, base);
    match a[2].opt_num() {
        Some(p) if p > super::js_len(&r) as f64 => s(pad_left(&r, p)),
        _ => s(r),
    }
}

fn decimal(_: &Call, a: &[Arg]) -> Val {
    let base = a[1].num() as u32;
    match coerce_to_base(a[0].str(), base, 255) {
        Some(v) => num(parse_int(&v, base)),
        None => err(ErrorType::Num),
    }
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("ABS", |_, a| num(f1(a).abs())),
        ("EXP", |_, a| num(f1(a).exp())),
        ("LOG10", |_, a| num(f1(a).log10())),
        ("LOG", |_, a| num(a[0].num().ln() / a[1].num().ln())),
        ("LN", |_, a| num(f1(a).ln())),
        ("POWER", |_, a| num(js_pow(a[0].num(), a[1].num()))),
        ("SQRT", |_, a| num(f1(a).sqrt())),
        ("MOD", |_, a| if a[1].num() == 0.0 { err(ErrorType::Div0) } else { num(a[0].num() % a[1].num()) }),
        ("PI", |_, _| num(PI)),
        ("SQRTPI", |_, a| num((PI * f1(a)).sqrt())),
        ("RADIANS", |_, a| num(f1(a) * (std::f64::consts::PI / 180.0))),
        ("DEGREES", |_, a| num(f1(a) * (180.0 / std::f64::consts::PI))),
        ("ISEVEN", |_, a| Val::bool(f1(a) % 2.0 == 0.0)),
        ("ISODD", |_, a| Val::bool(f1(a) % 2.0 == 1.0)),
        ("RAND", |c, _| num(c.it.env.random())),
        ("RANDBETWEEN", |c, a| {
            let (lo, hi) = (a[0].num(), a[1].num());
            if hi < lo {
                return err(ErrorType::Num);
            }
            let lo = lo.ceil();
            let mut hi = hi.floor() + 1.0;
            if lo == hi {
                hi += 1.0;
            }
            num(lo + (c.it.env.random() * (hi - lo)).floor())
        }),
        ("FACT", |_, a| {
            let n = f1(a).trunc();
            let mut r = 1.0;
            let mut i = 1.0;
            while i <= n {
                r *= i;
                i += 1.0;
            }
            num(r)
        }),
        ("FACTDOUBLE", |_, a| {
            let mut i = f1(a).trunc();
            let mut r = 1.0;
            while i >= 1.0 {
                r *= i;
                i -= 2.0;
            }
            num(r)
        }),
        ("COMBIN", |_, a| {
            let (n, m) = (a[0].num(), a[1].num());
            if m > n {
                return err(ErrorType::Num);
            }
            num(combin(n.trunc(), m.trunc()))
        }),
        ("COMBINA", |_, a| {
            let (n, m) = (a[0].num().trunc(), a[1].num().trunc());
            if n + m - 1.0 >= 1030.0 {
                return err(ErrorType::Num);
            }
            if n == 0.0 && m == 0.0 {
                return num(1.0);
            }
            num(combin(n + m - 1.0, m))
        }),
        ("GCD", gcd),
        ("LCM", lcm),
        ("MROUND", mround),
        ("MULTINOMIAL", multinomial),
        ("QUOTIENT", |_, a| if a[1].num() == 0.0 { err(ErrorType::Div0) } else { num((a[0].num() / a[1].num()).trunc()) }),
        ("SERIESSUM", seriessum),
        ("SIGN", |_, a| {
            let v = f1(a);
            num(if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            })
        }),
        ("SUMX2MY2", |_, a| sumx(a, |x, y| js_pow(x, 2.0) - js_pow(y, 2.0))),
        ("SUMX2PY2", |_, a| sumx(a, |x, y| js_pow(x, 2.0) + js_pow(y, 2.0))),
        ("SUMXMY2", |_, a| sumx(a, |x, y| js_pow(x - y, 2.0))),
        ("ROUNDUP", |_, a| round_with(a, f64::ceil)),
        ("ROUNDDOWN", |_, a| round_with(a, f64::floor)),
        ("ROUND", |_, a| round_with(a, js_round)),
        ("INT", |_, a| {
            let v = f1(a);
            num(if v < 0.0 { -(-v).floor() } else { v.floor() })
        }),
        ("EVEN", |_, a| {
            let v = f1(a);
            num(if v < 0.0 { -next_even(-v) } else { next_even(v) })
        }),
        ("ODD", |_, a| {
            let v = f1(a);
            num(if v < 0.0 { -next_odd(-v) } else { next_odd(v) })
        }),
        ("CEILING.MATH", |_, a| {
            let (v, mut sig, mode) = (a[0].num(), a[1].num(), a[2].num());
            if sig == 0.0 || v == 0.0 {
                return num(0.0);
            }
            sig = sig.abs();
            if mode == 1.0 && v < 0.0 {
                sig = -sig;
            }
            num((v / sig).ceil() * sig)
        }),
        ("CEILING", |_, a| {
            let (v, sig) = (a[0].num(), a[1].num());
            if v == 0.0 {
                return num(0.0);
            }
            if sig == 0.0 {
                return err(ErrorType::Div0);
            }
            if v > 0.0 && sig < 0.0 {
                return err(ErrorType::Num);
            }
            num((v / sig).ceil() * sig)
        }),
        ("CEILING.PRECISE", |_, a| {
            let (v, sig) = (a[0].num(), a[1].num());
            if sig == 0.0 || v == 0.0 {
                return num(0.0);
            }
            let sig = sig.abs();
            num((v / sig).ceil() * sig)
        }),
        ("FLOOR.MATH", |_, a| {
            let (v, mut sig, mode) = (a[0].num(), a[1].num(), a[2].num());
            if sig == 0.0 || v == 0.0 {
                return num(0.0);
            }
            sig = sig.abs();
            if mode == 1.0 && v < 0.0 {
                sig = -sig;
            }
            num((v / sig).floor() * sig)
        }),
        ("FLOOR", |_, a| {
            let (v, sig) = (a[0].num(), a[1].num());
            if v == 0.0 {
                return num(0.0);
            }
            if sig == 0.0 {
                return err(ErrorType::Div0);
            }
            if v > 0.0 && sig < 0.0 {
                return err(ErrorType::Num);
            }
            num((v / sig).floor() * sig)
        }),
        ("FLOOR.PRECISE", |_, a| {
            let (v, sig) = (a[0].num(), a[1].num());
            if sig == 0.0 || v == 0.0 {
                return num(0.0);
            }
            let sig = sig.abs();
            num((v / sig).floor() * sig)
        }),
        ("ACOS", |_, a| num(f1(a).acos())),
        ("ASIN", |_, a| num(f1(a).asin())),
        ("COS", |_, a| num(f1(a).cos())),
        ("SIN", |_, a| num(f1(a).sin())),
        ("TAN", |_, a| num(f1(a).tan())),
        ("ATAN", |_, a| num(f1(a).atan())),
        ("ATAN2", |_, a| {
            let (x, y) = (a[0].num(), a[1].num());
            if x == 0.0 && y == 0.0 {
                return err(ErrorType::Div0);
            }
            num(y.atan2(x))
        }),
        ("COT", |_, a| if f1(a) == 0.0 { err(ErrorType::Div0) } else { num(1.0 / f1(a).tan()) }),
        ("ACOT", |_, a| if f1(a) == 0.0 { num(PI / 2.0) } else { num((1.0 / f1(a)).atan()) }),
        ("SEC", |_, a| num(1.0 / f1(a).cos())),
        ("CSC", |_, a| if f1(a) == 0.0 { err(ErrorType::Div0) } else { num(1.0 / f1(a).sin()) }),
        ("SINH", |_, a| num(f1(a).sinh())),
        ("ASINH", |_, a| num(f1(a).asinh())),
        ("COSH", |_, a| num(f1(a).cosh())),
        ("ACOSH", |_, a| num(f1(a).acosh())),
        ("TANH", |_, a| num(f1(a).tanh())),
        ("ATANH", |_, a| num(f1(a).atanh())),
        ("COTH", |_, a| if f1(a) == 0.0 { err(ErrorType::Div0) } else { num(1.0 / f1(a).tanh()) }),
        ("ACOTH", |_, a| if f1(a) == 0.0 { err(ErrorType::Num) } else { num((1.0 / f1(a)).atanh()) }),
        ("SECH", |_, a| num(1.0 / f1(a).cosh())),
        ("CSCH", |_, a| if f1(a) == 0.0 { err(ErrorType::Div0) } else { num(1.0 / f1(a).sinh()) }),
        ("BITLSHIFT", |_, a| shift_left(a[0].num(), a[1].num())),
        ("BITRSHIFT", |_, a| shift_right(a[0].num(), a[1].num())),
        ("BITAND", |_, a| num((to_i32(a[0].num()) & to_i32(a[1].num())) as f64)),
        ("BITOR", |_, a| num((to_i32(a[0].num()) | to_i32(a[1].num())) as f64)),
        ("BITXOR", |_, a| num((to_i32(a[0].num()) ^ to_i32(a[1].num())) as f64)),
        ("DELTA", |_, a| num(if a[0].num() == a[1].num() { 1.0 } else { 0.0 })),
        ("DEC2BIN", |_, a| to_base_exact(a[0].num(), 2, a[1].opt_num())),
        ("DEC2OCT", |_, a| to_base_exact(a[0].num(), 8, a[1].opt_num())),
        ("DEC2HEX", |_, a| to_base_exact(a[0].num(), 16, a[1].opt_num())),
        ("BIN2DEC", |_, a| from_base(a, 2).map_or_else(err, num)),
        ("OCT2DEC", |_, a| from_base(a, 8).map_or_else(err, num)),
        ("HEX2DEC", |_, a| from_base(a, 16).map_or_else(err, num)),
        ("BIN2OCT", |_, a| conv(a, 2, 8)),
        ("BIN2HEX", |_, a| conv(a, 2, 16)),
        ("OCT2BIN", |_, a| conv(a, 8, 2)),
        ("OCT2HEX", |_, a| conv(a, 8, 16)),
        ("HEX2BIN", |_, a| conv(a, 16, 2)),
        ("HEX2OCT", |_, a| conv(a, 16, 8)),
        ("BASE", base_fn),
        ("DECIMAL", decimal),
    ])
}
