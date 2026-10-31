// SPDX-License-Identifier: AGPL-3.0-only
//! StatisticalPlugin (distributions, ERF/GAMMA family, Bessel functions), the hypothesis tests of
//! StatisticalAggregationPlugin (Z.TEST, F.TEST, CHISQ.TEST, T.TEST), RomanPlugin, SimpleArithmetic (HF.*)
//! and VERSION.

use super::super::coerce;
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, NumType, Scalar, Val};
use super::jstat::{self, beta, binomial, central_f, chisquare, exponential, gamma, hypgeom, lognormal, negbin, normal, poisson, studentt, weibull};
use super::stats::{jmean, jstdev, jvariance, sumsqerr};
use super::{Kind, err, exact_numbers, js_pow, num, s, simple};

fn n(a: &[Arg], i: usize) -> f64 {
    a[i].num()
}

fn b(a: &[Arg], i: usize) -> bool {
    a[i].bool()
}

fn erf(_: &Call, a: &[Arg]) -> Val {
    match a[1].opt_num() {
        None => num(jstat::erf(n(a, 0))),
        Some(up) => num(jstat::erf(up) - jstat::erf(n(a, 0))),
    }
}

fn erfc(_: &Call, a: &[Arg]) -> Val {
    num(jstat::erfc(n(a, 0)))
}

fn expondist(_: &Call, a: &[Arg]) -> Val {
    num(if b(a, 2) { exponential::cdf(n(a, 0), n(a, 1)) } else { exponential::pdf(n(a, 0), n(a, 1)) })
}

fn fisher(_: &Call, a: &[Arg]) -> Val {
    let x = n(a, 0);
    num(((1.0 + x) / (1.0 - x)).ln() / 2.0)
}

fn fisherinv(_: &Call, a: &[Arg]) -> Val {
    num(1.0 - 2.0 / ((2.0 * n(a, 0)).exp() + 1.0))
}

fn gamma_fn(_: &Call, a: &[Arg]) -> Val {
    num(jstat::gammafn(n(a, 0)))
}

fn gammadist(_: &Call, a: &[Arg]) -> Val {
    num(if b(a, 3) { gamma::cdf(n(a, 0), n(a, 1), n(a, 2)) } else { gamma::pdf(n(a, 0), n(a, 1), n(a, 2)) })
}

fn gammaln(_: &Call, a: &[Arg]) -> Val {
    num(jstat::gammaln(n(a, 0)))
}

fn gammainv(_: &Call, a: &[Arg]) -> Val {
    num(gamma::inv(n(a, 0), n(a, 1), n(a, 2)))
}

fn gauss(_: &Call, a: &[Arg]) -> Val {
    num(normal::cdf(n(a, 0), 0.0, 1.0) - 0.5)
}

fn betadist(_: &Call, a: &[Arg]) -> Val {
    let (mut x, al, be, cum, lo, hi) = (n(a, 0), n(a, 1), n(a, 2), b(a, 3), n(a, 4), n(a, 5));
    if x <= lo || x >= hi {
        return err(ErrorType::Num);
    }
    x = (x - lo) / (hi - lo);
    num(if cum { beta::cdf(x, al, be) } else { beta::pdf(x, al, be) })
}

fn betainv(_: &Call, a: &[Arg]) -> Val {
    let (x, al, be, lo, hi) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4));
    if lo >= hi {
        return err(ErrorType::Num);
    }
    num(beta::inv(x, al, be) * (hi - lo) + lo)
}

fn binomdist(_: &Call, a: &[Arg]) -> Val {
    let (succ, trials, p, cum) = (n(a, 0), n(a, 1), n(a, 2), b(a, 3));
    if succ > trials {
        return err(ErrorType::Num);
    }
    let (succ, trials) = (succ.trunc(), trials.trunc());
    num(if cum { binomial::cdf(succ, trials, p) } else { binomial::pdf(succ, trials, p) })
}

fn binominv(_: &Call, a: &[Arg]) -> Val {
    let (trials, p, alpha) = (n(a, 0).trunc(), n(a, 1), n(a, 2));
    let mut lower = -1.0;
    let mut upper = trials;
    while upper > lower + 1.0 {
        let mid = ((lower + upper) / 2.0).trunc();
        if binomial::cdf(mid, trials, p) >= alpha {
            upper = mid;
        } else {
            lower = mid;
        }
    }
    num(upper)
}

fn besseli(_: &Call, a: &[Arg]) -> Val {
    num(jstat::besseli(n(a, 0), n(a, 1).trunc()))
}

fn besselj(_: &Call, a: &[Arg]) -> Val {
    num(jstat::besselj(n(a, 0), n(a, 1).trunc()))
}

fn besselk(_: &Call, a: &[Arg]) -> Val {
    num(jstat::besselk(n(a, 0), n(a, 1).trunc()))
}

fn bessely(_: &Call, a: &[Arg]) -> Val {
    num(jstat::bessely(n(a, 0), n(a, 1).trunc()))
}

fn chisqdist(_: &Call, a: &[Arg]) -> Val {
    let deg = n(a, 1).trunc();
    num(if b(a, 2) { chisquare::cdf(n(a, 0), deg) } else { chisquare::pdf(n(a, 0), deg) })
}

fn chisqdistrt(_: &Call, a: &[Arg]) -> Val {
    num(1.0 - chisquare::cdf(n(a, 0), n(a, 1).trunc()))
}

fn chisqinv(_: &Call, a: &[Arg]) -> Val {
    num(chisquare::inv(n(a, 0), n(a, 1).trunc()))
}

fn chisqinvrt(_: &Call, a: &[Arg]) -> Val {
    num(chisquare::inv(1.0 - n(a, 0), n(a, 1).trunc()))
}

fn fdist(_: &Call, a: &[Arg]) -> Val {
    let (d1, d2) = (n(a, 1).trunc(), n(a, 2).trunc());
    num(if b(a, 3) { central_f::cdf(n(a, 0), d1, d2) } else { central_f::pdf(n(a, 0), d1, d2) })
}

fn fdistrt(_: &Call, a: &[Arg]) -> Val {
    num(1.0 - central_f::cdf(n(a, 0), n(a, 1).trunc(), n(a, 2).trunc()))
}

fn finv(_: &Call, a: &[Arg]) -> Val {
    num(central_f::inv(n(a, 0), n(a, 1).trunc(), n(a, 2).trunc()))
}

fn finvrt(_: &Call, a: &[Arg]) -> Val {
    num(central_f::inv(1.0 - n(a, 0), n(a, 1).trunc(), n(a, 2).trunc()))
}

fn weibulldist(_: &Call, a: &[Arg]) -> Val {
    let (x, shape, scale) = (n(a, 0), n(a, 1), n(a, 2));
    num(if b(a, 3) { weibull::cdf(x, scale, shape) } else { weibull::pdf(x, scale, shape) })
}

fn poissondist(_: &Call, a: &[Arg]) -> Val {
    let x = n(a, 0).trunc();
    num(if b(a, 2) { poisson::cdf(x, n(a, 1)) } else { poisson::pdf(x, n(a, 1)) })
}

fn hypgeomdist(_: &Call, a: &[Arg]) -> Val {
    let (s, ns, ps, np) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3));
    if s > ns || s > ps || ns > np || ps > np {
        return err(ErrorType::Num);
    }
    if s + np < ps + ns {
        return err(ErrorType::Num);
    }
    let (s, ns, ps, np) = (s.trunc(), ns.trunc(), ps.trunc(), np.trunc());
    num(if b(a, 4) { hypgeom::cdf(s, np, ps, ns) } else { hypgeom::pdf(s, np, ps, ns) })
}

fn tdist(_: &Call, a: &[Arg]) -> Val {
    let deg = n(a, 1).trunc();
    num(if b(a, 2) { studentt::cdf(n(a, 0), deg) } else { studentt::pdf(n(a, 0), deg) })
}

fn tdist2t(_: &Call, a: &[Arg]) -> Val {
    num((1.0 - studentt::cdf(n(a, 0), n(a, 1).trunc())) * 2.0)
}

fn tdistrt(_: &Call, a: &[Arg]) -> Val {
    num(1.0 - studentt::cdf(n(a, 0), n(a, 1).trunc()))
}

fn tdistold(_: &Call, a: &[Arg]) -> Val {
    num(n(a, 2) * (1.0 - studentt::cdf(n(a, 0), n(a, 1).trunc())))
}

fn tinv(_: &Call, a: &[Arg]) -> Val {
    num(studentt::inv(n(a, 0), n(a, 1).trunc()))
}

fn tinv2t(_: &Call, a: &[Arg]) -> Val {
    num(studentt::inv(1.0 - n(a, 0) / 2.0, n(a, 1).trunc()))
}

fn lognormdist(_: &Call, a: &[Arg]) -> Val {
    num(if b(a, 3) { lognormal::cdf(n(a, 0), n(a, 1), n(a, 2)) } else { lognormal::pdf(n(a, 0), n(a, 1), n(a, 2)) })
}

fn lognorminv(_: &Call, a: &[Arg]) -> Val {
    num(lognormal::inv(n(a, 0), n(a, 1), n(a, 2)))
}

fn normdist(_: &Call, a: &[Arg]) -> Val {
    num(if b(a, 3) { normal::cdf(n(a, 0), n(a, 1), n(a, 2)) } else { normal::pdf(n(a, 0), n(a, 1), n(a, 2)) })
}

fn norminv(_: &Call, a: &[Arg]) -> Val {
    num(normal::inv(n(a, 0), n(a, 1), n(a, 2)))
}

fn normsdist(_: &Call, a: &[Arg]) -> Val {
    num(if b(a, 1) { normal::cdf(n(a, 0), 0.0, 1.0) } else { normal::pdf(n(a, 0), 0.0, 1.0) })
}

fn normsinv(_: &Call, a: &[Arg]) -> Val {
    num(normal::inv(n(a, 0), 0.0, 1.0))
}

fn phi(_: &Call, a: &[Arg]) -> Val {
    num(normal::pdf(n(a, 0), 0.0, 1.0))
}

fn negbinomdist(_: &Call, a: &[Arg]) -> Val {
    let (nf, ns, p) = (n(a, 0).trunc(), n(a, 1).trunc(), n(a, 2));
    if b(a, 3) {
        return num(negbin::cdf(nf, ns, p));
    }
    match negbin::pdf(nf, ns, p) {
        Some(v) => num(v),
        // jStat returns `false` for a non-uint32 count.
        None => Val::bool(false),
    }
}

fn confidencenorm(_: &Call, a: &[Arg]) -> Val {
    num(jstat::normalci_hi(1.0, n(a, 0), n(a, 1), n(a, 2).trunc()) - 1.0)
}

fn confidencet(_: &Call, a: &[Arg]) -> Val {
    let size = n(a, 2).trunc();
    if size == 1.0 {
        return err(ErrorType::Div0);
    }
    num(jstat::tci_hi(1.0, n(a, 0), n(a, 1), size) - 1.0)
}

fn standardize(_: &Call, a: &[Arg]) -> Val {
    num((n(a, 0) - n(a, 1)) / n(a, 2))
}

// ------------------------------------------------------------------ hypothesis tests

fn exact(a: &Arg) -> Result<Vec<f64>, ErrorType> {
    exact_numbers(a.to_range().values())
}

fn ztest(_: &Call, a: &[Arg]) -> Val {
    let vals = match exact(&a[0]) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let x = n(a, 1);
    let cnt = vals.len();
    let sigma = match a[2].opt_num() {
        Some(s) => s,
        None => {
            if cnt < 2 {
                return err(ErrorType::Div0);
            }
            jstdev(&vals, true)
        }
    };
    if cnt < 1 {
        return err(ErrorType::Na);
    }
    if sigma == 0.0 {
        return err(ErrorType::Div0);
    }
    num(1.0 - normal::cdf((jmean(&vals) - x) / (sigma / (cnt as f64).sqrt()), 0.0, 1.0))
}

fn ftest(_: &Call, a: &[Arg]) -> Val {
    let x = match exact(&a[0]) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let y = match exact(&a[1]) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if x.len() <= 1 || y.len() <= 1 {
        return err(ErrorType::Div0);
    }
    let (vx, vy) = (jvariance(&x, true), jvariance(&y, true));
    if vx == 0.0 || vy == 0.0 {
        return err(ErrorType::Div0);
    }
    let v = central_f::cdf(vx / vy, x.len() as f64 - 1.0, y.len() as f64 - 1.0);
    num(2.0 * v.min(1.0 - v))
}

/// `parseTwoArrays`: pairs where both values are numbers (errors propagate).
fn two(a: &[Arg]) -> Result<(Vec<f64>, Vec<f64>), ErrorType> {
    let (x, y) = (a[0].to_range(), a[1].to_range());
    let (mut ax, mut ay) = (Vec::new(), Vec::new());
    for (vx, vy) in x.values().zip(y.values()) {
        if let Scalar::Err(e) = vx {
            return Err(*e);
        }
        if let Scalar::Err(e) = vy {
            return Err(*e);
        }
        if let (Scalar::Num(p, _), Scalar::Num(q, _)) = (vx, vy) {
            ax.push(*p);
            ay.push(*q);
        }
    }
    Ok((ax, ay))
}

fn chisqtest(_: &Call, a: &[Arg]) -> Val {
    let (x, y) = (a[0].to_range(), a[1].to_range());
    let (r, c) = (x.height(), x.width());
    if y.height() != r || y.width() != c {
        return err(ErrorType::Na);
    }
    let (ax, ay) = match two(a) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if ax.len() <= 1 {
        return err(ErrorType::Div0);
    }
    let mut sum = 0.0;
    for i in 0..ax.len() {
        if ay[i] == 0.0 {
            return err(ErrorType::Div0);
        }
        sum += js_pow(ax[i] - ay[i], 2.0) / ay[i];
    }
    if sum < 0.0 {
        return err(ErrorType::Num);
    }
    let dof = if r > 1 && c > 1 { (r - 1) * (c - 1) } else { r * c - 1 };
    num(1.0 - chisquare::cdf(sum, dof as f64))
}

fn ttest(_: &Call, a: &[Arg]) -> Val {
    let x = match exact(&a[0]) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let y = match exact(&a[1]) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let (tails, ty) = (n(a, 2), n(a, 3));
    let (nx, my) = (x.len() as f64, y.len() as f64);
    if ty == 1.0 {
        if x.len() != y.len() {
            return err(ErrorType::Na);
        }
        if nx <= 1.0 {
            return err(ErrorType::Div0);
        }
        let sub: Vec<f64> = x.iter().zip(&y).map(|(p, q)| p - q).collect();
        let s = jstdev(&sub, true);
        if s == 0.0 {
            return err(ErrorType::Div0);
        }
        let t = (nx.sqrt() * jmean(&sub) / s).abs();
        num(tails * (1.0 - studentt::cdf(t, nx - 1.0)))
    } else if ty == 2.0 {
        if nx <= 1.0 || my <= 1.0 {
            return err(ErrorType::Div0);
        }
        let s = (sumsqerr(&x) + sumsqerr(&y)) / (nx + my - 2.0);
        if s == 0.0 {
            return err(ErrorType::Div0);
        }
        let t = ((jmean(&x) - jmean(&y)) / (s * (1.0 / nx + 1.0 / my)).sqrt()).abs();
        num(tails * (1.0 - studentt::cdf(t, nx + my - 2.0)))
    } else {
        if nx <= 1.0 || my <= 1.0 {
            return err(ErrorType::Div0);
        }
        let (sx, sy) = (jvariance(&x, true), jvariance(&y, true));
        if sx == 0.0 && sy == 0.0 {
            return err(ErrorType::Div0);
        }
        let t = ((jmean(&x) - jmean(&y)) / (sx / nx + sy / my).sqrt()).abs();
        let v = js_pow(sx / nx + sy / my, 2.0) / (js_pow(sx / nx, 2.0) / (nx - 1.0) + js_pow(sy / my, 2.0) / (my - 1.0));
        num(tails * (1.0 - studentt::cdf(t, v)))
    }
}

// ------------------------------------------------------------------ RomanPlugin

fn roman_mode(input: f64, mode: f64) -> String {
    let mut val = input % 1000.0;
    let mut acc = "M".repeat((input / 1000.0).floor().max(0.0) as usize);
    let absorb = |val: &mut f64, acc: &mut String, token: &str, lower: f64, upper: f64| {
        if *val >= lower && *val < upper {
            *val -= lower;
            acc.push_str(token);
        }
    };
    if mode == 4.0 {
        absorb(&mut val, &mut acc, "IM", 999.0, 1000.0);
        absorb(&mut val, &mut acc, "ID", 499.0, 500.0);
    }
    if mode >= 3.0 {
        absorb(&mut val, &mut acc, "VM", 995.0, 1000.0);
        absorb(&mut val, &mut acc, "VD", 495.0, 500.0);
    }
    if mode >= 2.0 {
        absorb(&mut val, &mut acc, "XM", 990.0, 1000.0);
        absorb(&mut val, &mut acc, "XD", 490.0, 500.0);
    }
    if mode >= 1.0 {
        absorb(&mut val, &mut acc, "LM", 950.0, 1000.0);
        absorb(&mut val, &mut acc, "LD", 450.0, 500.0);
    }
    absorb(&mut val, &mut acc, "CM", 900.0, 1000.0);
    absorb(&mut val, &mut acc, "CD", 400.0, 500.0);
    absorb(&mut val, &mut acc, "D", 500.0, 900.0);
    acc.push_str(&"C".repeat((val / 100.0).floor().max(0.0) as usize));
    val %= 100.0;
    if mode >= 2.0 {
        absorb(&mut val, &mut acc, "IC", 99.0, 100.0);
        absorb(&mut val, &mut acc, "IL", 49.0, 50.0);
    }
    if mode >= 1.0 {
        absorb(&mut val, &mut acc, "VC", 95.0, 100.0);
        absorb(&mut val, &mut acc, "VL", 45.0, 50.0);
    }
    absorb(&mut val, &mut acc, "XC", 90.0, 100.0);
    absorb(&mut val, &mut acc, "XL", 40.0, 50.0);
    absorb(&mut val, &mut acc, "L", 50.0, 90.0);
    acc.push_str(&"X".repeat((val / 10.0).floor().max(0.0) as usize));
    val %= 10.0;
    absorb(&mut val, &mut acc, "IX", 9.0, 10.0);
    absorb(&mut val, &mut acc, "IV", 4.0, 5.0);
    absorb(&mut val, &mut acc, "V", 5.0, 9.0);
    acc.push_str(&"I".repeat(val.max(0.0) as usize));
    acc
}

fn roman(_: &Call, a: &[Arg]) -> Val {
    let val = n(a, 0).trunc();
    let mode = match a[1].scalar() {
        Scalar::Bool(false) => Scalar::num(4.0),
        Scalar::Bool(true) => Scalar::num(0.0),
        o => o,
    };
    let mode = match coerce::to_number_or_error(&mode) {
        Scalar::Num(v, _) => v.trunc(),
        Scalar::Err(e) => return err(e),
        _ => return err(ErrorType::Value),
    };
    if mode < 0.0 || mode > 4.0 {
        return err(ErrorType::Value);
    }
    s(roman_mode(val, mode))
}

fn arabic(_: &Call, a: &[Arg]) -> Val {
    let mut input = oc_schema::js::trim(a[0].str()).to_uppercase();
    let mut minus = false;
    if let Some(rest) = input.strip_prefix('-') {
        input = rest.to_string();
        minus = true;
        if input.is_empty() {
            return err(ErrorType::Value);
        }
    }
    let mut acc = 0.0;
    let groups: [&[(&str, f64)]; 10] = [
        &[("MMM", 3000.0), ("MM", 2000.0), ("M", 1000.0)],
        &[("IM", 999.0), ("VM", 995.0), ("XM", 990.0), ("LM", 950.0), ("CM", 900.0)],
        &[("D", 500.0), ("ID", 499.0), ("VD", 495.0), ("XD", 490.0), ("LD", 450.0), ("CD", 400.0)],
        &[("CCC", 300.0), ("CC", 200.0), ("C", 100.0)],
        &[("IC", 99.0), ("VC", 95.0), ("XC", 90.0)],
        &[("L", 50.0), ("IL", 49.0), ("VL", 45.0), ("XL", 40.0)],
        &[("XXX", 30.0), ("XX", 20.0), ("X", 10.0)],
        &[("IX", 9.0)],
        &[("V", 5.0), ("IV", 4.0)],
        &[("III", 3.0), ("II", 2.0), ("I", 1.0)],
    ];
    for g in groups {
        for (tok, v) in g {
            if let Some(rest) = input.strip_prefix(tok) {
                input = rest.to_string();
                acc += v;
                break;
            }
        }
    }
    if !input.is_empty() {
        return err(ErrorType::Value);
    }
    num(if minus { -acc } else { acc })
}

// ------------------------------------------------------------------ HF.* operators, VERSION

fn typed2(a: &[Arg]) -> (f64, NumType, f64, NumType) {
    let g = |x: &Arg| match x.scalar() {
        Scalar::Num(v, t) => (v, t),
        _ => (f64::NAN, NumType::Raw),
    };
    let (l, lt) = g(&a[0]);
    let (r, rt) = g(&a[1]);
    (l, lt, r, rt)
}

fn hf_add(_: &Call, a: &[Arg]) -> Val {
    let (l, lt, r, rt) = typed2(a);
    Val::S(coerce::add(l, lt, r, rt))
}

fn hf_minus(_: &Call, a: &[Arg]) -> Val {
    let (l, lt, r, rt) = typed2(a);
    Val::S(coerce::subtract(l, lt, r, rt))
}

fn hf_multiply(_: &Call, a: &[Arg]) -> Val {
    let (l, lt, r, rt) = typed2(a);
    Val::S(coerce::multiply(l, lt, r, rt))
}

fn hf_divide(_: &Call, a: &[Arg]) -> Val {
    let (l, lt, r, rt) = typed2(a);
    Val::S(coerce::divide(l, lt, r, rt))
}

fn hf_pow(_: &Call, a: &[Arg]) -> Val {
    num(js_pow(n(a, 0), n(a, 1)))
}

fn hf_concat(_: &Call, a: &[Arg]) -> Val {
    s(format!("{}{}", a[0].str(), a[1].str()))
}

fn cmp(a: &[Arg]) -> i32 {
    coerce::compare(&a[0].scalar(), &a[1].scalar())
}

fn hf_eq(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) == 0)
}

fn hf_ne(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) != 0)
}

fn hf_lt(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) < 0)
}

fn hf_lte(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) <= 0)
}

fn hf_gt(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) > 0)
}

fn hf_gte(_: &Call, a: &[Arg]) -> Val {
    Val::bool(cmp(a) >= 0)
}

fn hf_uminus(_: &Call, a: &[Arg]) -> Val {
    match a[0].scalar() {
        Scalar::Num(v, t) => Val::S(Scalar::Num(-v, t)),
        o => Val::S(o),
    }
}

fn hf_uplus(_: &Call, a: &[Arg]) -> Val {
    Val::S(a[0].scalar())
}

fn hf_upercent(_: &Call, a: &[Arg]) -> Val {
    Val::S(Scalar::Num(n(a, 0) / 100.0, NumType::Percent))
}

/// `VERSION()` with the engine's `licenseKey: 'gpl-v3'` (status 1).
fn version(_: &Call, _: &[Arg]) -> Val {
    s("HyperFormula v3.4.0, 1")
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("ERF", erf),
        ("ERFC", erfc),
        ("EXPON.DIST", expondist),
        ("FISHER", fisher),
        ("FISHERINV", fisherinv),
        ("GAMMA", gamma_fn),
        ("GAMMA.DIST", gammadist),
        ("GAMMALN", gammaln),
        ("GAMMA.INV", gammainv),
        ("GAUSS", gauss),
        ("BETA.DIST", betadist),
        ("BETA.INV", betainv),
        ("BINOM.DIST", binomdist),
        ("BINOM.INV", binominv),
        ("BESSELI", besseli),
        ("BESSELJ", besselj),
        ("BESSELK", besselk),
        ("BESSELY", bessely),
        ("CHISQ.DIST", chisqdist),
        ("CHISQ.DIST.RT", chisqdistrt),
        ("CHISQ.INV", chisqinv),
        ("CHISQ.INV.RT", chisqinvrt),
        ("F.DIST", fdist),
        ("F.DIST.RT", fdistrt),
        ("F.INV", finv),
        ("F.INV.RT", finvrt),
        ("WEIBULL.DIST", weibulldist),
        ("POISSON.DIST", poissondist),
        ("HYPGEOM.DIST", hypgeomdist),
        ("T.DIST", tdist),
        ("T.DIST.2T", tdist2t),
        ("T.DIST.RT", tdistrt),
        ("TDIST", tdistold),
        ("T.INV", tinv),
        ("T.INV.2T", tinv2t),
        ("LOGNORM.DIST", lognormdist),
        ("LOGNORM.INV", lognorminv),
        ("NORM.DIST", normdist),
        ("NORM.INV", norminv),
        ("NORM.S.DIST", normsdist),
        ("NORM.S.INV", normsinv),
        ("PHI", phi),
        ("NEGBINOM.DIST", negbinomdist),
        ("CONFIDENCE.NORM", confidencenorm),
        ("CONFIDENCE.T", confidencet),
        ("STANDARDIZE", standardize),
        ("Z.TEST", ztest),
        ("F.TEST", ftest),
        ("CHISQ.TEST", chisqtest),
        ("T.TEST", ttest),
        ("ROMAN", roman),
        ("ARABIC", arabic),
        ("HF.ADD", hf_add),
        ("HF.MINUS", hf_minus),
        ("HF.MULTIPLY", hf_multiply),
        ("HF.DIVIDE", hf_divide),
        ("HF.POW", hf_pow),
        ("HF.CONCAT", hf_concat),
        ("HF.EQ", hf_eq),
        ("HF.NE", hf_ne),
        ("HF.LT", hf_lt),
        ("HF.LTE", hf_lte),
        ("HF.GT", hf_gt),
        ("HF.GTE", hf_gte),
        ("HF.UMINUS", hf_uminus),
        ("HF.UPLUS", hf_uplus),
        ("HF.UNARY_PERCENT", hf_upercent),
        ("VERSION", version),
    ])
}
