// SPDX-License-Identifier: AGPL-3.0-only
//! FinancialPlugin: PMT, IPMT, PPMT, FV, PV, NPER, RATE, CUMIPMT, CUMPRINC, DB, DDB, SLN, SYD, DOLLARDE,
//! DOLLARFR, EFFECT, NOMINAL, ISPMT, RRI, TBILLEQ/PRICE/YIELD, FVSCHEDULE, NPV, MIRR, PDURATION, XNPV, IRR, XIRR.

use super::super::datetime::number_to_simple_date;
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, NumType, Scalar, Val};
use super::{Kind, err, exact_numbers, js_pow, num, numbers_exact_ranges, simple};
use crate::jsnum::js_round;

fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

fn pmt_core(rate: f64, periods: f64, present: f64, future: f64, ty: f64) -> f64 {
    if rate == 0.0 {
        (-present - future) / periods
    } else {
        let term = js_pow(1.0 + rate, periods);
        (future * rate + present * rate * term) * (if truthy(ty) { 1.0 / (1.0 + rate) } else { 1.0 }) / (1.0 - term)
    }
}

fn fv_core(rate: f64, periods: f64, payment: f64, value: f64, ty: f64) -> f64 {
    if rate == 0.0 {
        -value - payment * periods
    } else {
        let term = js_pow(1.0 + rate, periods);
        payment * (if truthy(ty) { 1.0 + rate } else { 1.0 }) * (1.0 - term) / rate - value * term
    }
}

fn ipmt_core(rate: f64, period: f64, periods: f64, present: f64, future: f64, ty: f64) -> f64 {
    let payment = pmt_core(rate, periods, present, future, ty);
    if period == 1.0 {
        rate * (if truthy(ty) { 0.0 } else { -present })
    } else {
        rate * (if truthy(ty) {
            fv_core(rate, period - 2.0, payment, present, ty) - payment
        } else {
            fv_core(rate, period - 1.0, payment, present, ty)
        })
    }
}

fn ppmt_core(rate: f64, period: f64, periods: f64, present: f64, future: f64, ty: f64) -> f64 {
    pmt_core(rate, periods, present, future, ty) - ipmt_core(rate, period, periods, present, future, ty)
}

fn npv_core(rate: f64, args: &[f64]) -> Result<f64, ErrorType> {
    let mut acc = 0.0;
    for v in args.iter().rev() {
        acc += v;
        if rate == -1.0 {
            if acc == 0.0 {
                continue;
            }
            return Err(ErrorType::Div0);
        }
        acc /= 1.0 + rate;
    }
    Ok(acc)
}

fn n(a: &[Arg], i: usize) -> f64 {
    a[i].num()
}

fn pmt(_: &Call, a: &[Arg]) -> Val {
    num(pmt_core(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4)))
}

fn ipmt(_: &Call, a: &[Arg]) -> Val {
    num(ipmt_core(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5)))
}

fn ppmt(_: &Call, a: &[Arg]) -> Val {
    num(ppmt_core(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5)))
}

fn fv(_: &Call, a: &[Arg]) -> Val {
    num(fv_core(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4)))
}

fn cumulative(a: &[Arg], f: fn(f64, f64, f64, f64, f64, f64) -> f64) -> Val {
    let (rate, periods, value, start, end, ty) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5));
    if start > end {
        return err(ErrorType::Num);
    }
    let mut acc = 0.0;
    let mut i = start;
    while i <= end {
        acc += f(rate, i, periods, value, 0.0, ty);
        i += 1.0;
    }
    num(acc)
}

fn cumipmt(_: &Call, a: &[Arg]) -> Val {
    cumulative(a, ipmt_core)
}

fn cumprinc(_: &Call, a: &[Arg]) -> Val {
    cumulative(a, ppmt_core)
}

fn db(_: &Call, a: &[Arg]) -> Val {
    let (cost, salvage, life, period, month) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4));
    if (month == 12.0 && period > life) || period > life + 1.0 {
        return err(ErrorType::Num);
    }
    if salvage >= cost {
        return num(0.0);
    }
    let rate = js_round((1.0 - js_pow(salvage / cost, 1.0 / life)) * 1000.0) / 1000.0;
    let initial = cost * rate * month / 12.0;
    if period == 1.0 {
        return num(initial);
    }
    let mut total = initial;
    let mut i = 0.0;
    while i < period - 2.0 {
        total += (cost - total) * rate;
        i += 1.0;
    }
    if period == life + 1.0 {
        return num((cost - total) * rate * (12.0 - month) / 12.0);
    }
    num((cost - total) * rate)
}

fn ddb(_: &Call, a: &[Arg]) -> Val {
    let (cost, salvage, life, period, factor) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4));
    if period > life {
        return err(ErrorType::Num);
    }
    let mut rate = factor / life;
    let old = if rate >= 1.0 {
        rate = 1.0;
        if period == 1.0 { cost } else { 0.0 }
    } else {
        cost * js_pow(1.0 - rate, period - 1.0)
    };
    let new = cost * js_pow(1.0 - rate, period);
    num(js_max(old - js_max(salvage, new), 0.0))
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

fn dollar(a: &[Arg], de: bool) -> Val {
    let d = n(a, 0);
    let mut fraction = n(a, 1);
    if fraction < 1.0 {
        return err(ErrorType::Div0);
    }
    fraction = fraction.trunc();
    while fraction > 10.0 {
        fraction /= 10.0;
    }
    let frac = d - d.trunc();
    num(if de { d.trunc() + frac * 10.0 / fraction } else { d.trunc() + frac * fraction / 10.0 })
}

fn dollarde(_: &Call, a: &[Arg]) -> Val {
    dollar(a, true)
}

fn dollarfr(_: &Call, a: &[Arg]) -> Val {
    dollar(a, false)
}

fn effect(_: &Call, a: &[Arg]) -> Val {
    let periods = n(a, 1).trunc();
    num(js_pow(1.0 + n(a, 0) / periods, periods) - 1.0)
}

fn ispmt(_: &Call, a: &[Arg]) -> Val {
    let (rate, period, periods, value) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3));
    if periods == 0.0 {
        return err(ErrorType::Div0);
    }
    num(value * rate * (period / periods - 1.0))
}

fn nominal(_: &Call, a: &[Arg]) -> Val {
    let periods = n(a, 1).trunc();
    num((js_pow(n(a, 0) + 1.0, 1.0 / periods) - 1.0) * periods)
}

fn nper(_: &Call, a: &[Arg]) -> Val {
    let (rate, mut payment, present, future, ty) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4));
    if rate == 0.0 {
        if payment == 0.0 {
            return err(ErrorType::Div0);
        }
        return num((-present - future) / payment);
    }
    if truthy(ty) {
        payment *= 1.0 + rate;
    }
    num(((payment - future * rate) / (present * rate + payment)).ln() / (1.0 + rate).ln())
}

fn rate(_: &Call, a: &[Arg]) -> Val {
    let (periods, payment, present, future, ty, guess) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5));
    if guess <= -1.0 {
        return err(ErrorType::Value);
    }
    let eps = 1e-7;
    let mut rate = guess;
    let ty = if truthy(ty) { 1.0 } else { 0.0 };
    for _ in 0..50 {
        if rate <= -1.0 {
            return err(ErrorType::Num);
        }
        let y = if rate.abs() < eps {
            present * (1.0 + periods * rate) + payment * (1.0 + rate * ty) * periods + future
        } else {
            let f = js_pow(1.0 + rate, periods);
            present * f + payment * (1.0 / rate + ty) * (f - 1.0) + future
        };
        if y.abs() < eps {
            return num(rate);
        }
        let dy = if rate.abs() < eps {
            present * periods + payment * ty * periods
        } else {
            let f = js_pow(1.0 + rate, periods);
            let df = periods * js_pow(1.0 + rate, periods - 1.0);
            present * df + payment * (1.0 / rate + ty) * df + payment * (-1.0 / (rate * rate)) * (f - 1.0)
        };
        rate -= y / dy;
    }
    err(ErrorType::Num)
}

fn pv(_: &Call, a: &[Arg]) -> Val {
    let (rate, periods, payment, future, ty) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4));
    let ty = if truthy(ty) { 1.0 } else { 0.0 };
    if rate == -1.0 {
        return if periods == 0.0 { err(ErrorType::Num) } else { err(ErrorType::Div0) };
    }
    if rate == 0.0 {
        num(-payment * periods - future)
    } else {
        num(((1.0 - js_pow(1.0 + rate, periods)) * payment * (1.0 + rate * ty) / rate - future) / js_pow(1.0 + rate, periods))
    }
}

fn rri(_: &Call, a: &[Arg]) -> Val {
    let (periods, present, future) = (n(a, 0), n(a, 1), n(a, 2));
    if present == 0.0 || (future < 0.0 && present > 0.0) || (future > 0.0 && present < 0.0) {
        return err(ErrorType::Num);
    }
    num(js_pow(future / present, 1.0 / periods) - 1.0)
}

fn sln(_: &Call, a: &[Arg]) -> Val {
    let (cost, salvage, life) = (n(a, 0), n(a, 1), n(a, 2));
    if life == 0.0 {
        return err(ErrorType::Div0);
    }
    num((cost - salvage) / life)
}

fn syd(_: &Call, a: &[Arg]) -> Val {
    let (cost, salvage, life, period) = (n(a, 0), n(a, 1), n(a, 2), n(a, 3));
    if period > life {
        return err(ErrorType::Num);
    }
    num((cost - salvage) * (life - period + 1.0) * 2.0 / (life * (life + 1.0)))
}

/// Shared TBILL validation: (settlement, maturity) rounded, `None` when the bill is invalid.
fn tbill_dates(a: &[Arg]) -> Option<(f64, f64)> {
    let settlement = js_round(n(a, 0));
    let maturity = js_round(n(a, 1));
    if settlement >= maturity {
        return None;
    }
    let s = number_to_simple_date(settlement);
    let e = number_to_simple_date(maturity);
    if e.year > s.year + 1.0 || (e.year == s.year + 1.0 && (e.month > s.month || (e.month == s.month && e.day > s.day))) {
        return None;
    }
    Some((settlement, maturity))
}

fn tbilleq(_: &Call, a: &[Arg]) -> Val {
    let Some((s, m)) = tbill_dates(a) else { return err(ErrorType::Num) };
    let discount = n(a, 2);
    let denom = 360.0 - discount * (m - s);
    if denom == 0.0 {
        return num(0.0);
    }
    if denom < 0.0 {
        return err(ErrorType::Num);
    }
    num(365.0 * discount / denom)
}

fn tbillprice(_: &Call, a: &[Arg]) -> Val {
    let Some((s, m)) = tbill_dates(a) else { return err(ErrorType::Num) };
    let discount = n(a, 2);
    let denom = 360.0 - discount * (m - s);
    if denom == 0.0 {
        return num(0.0);
    }
    if denom < 0.0 {
        return err(ErrorType::Num);
    }
    num(100.0 * (1.0 - discount * (m - s) / 360.0))
}

fn tbillyield(_: &Call, a: &[Arg]) -> Val {
    let Some((s, m)) = tbill_dates(a) else { return err(ErrorType::Num) };
    let price = n(a, 2);
    num((100.0 - price) * 360.0 / (price * (m - s)))
}

fn fvschedule(_: &Call, a: &[Arg]) -> Val {
    let mut value = n(a, 0);
    let r = a[1].to_range();
    if let Some(Scalar::Err(e)) = r.values().find(|v| matches!(v, Scalar::Err(_))) {
        return err(*e);
    }
    for v in r.values() {
        match v {
            Scalar::Num(x, _) => value *= 1.0 + x,
            Scalar::Empty => {}
            _ => return err(ErrorType::Value),
        }
    }
    num(value)
}

fn npv(_: &Call, a: &[Arg]) -> Val {
    match numbers_exact_ranges(&a[1..]) {
        Ok(v) => match npv_core(n(a, 0), &v) {
            Ok(x) => num(x),
            Err(e) => err(e),
        },
        Err(e) => err(e),
    }
}

fn mirr(_: &Call, a: &[Arg]) -> Val {
    let r = a[0].to_range();
    let (frate, rrate) = (n(a, 1), n(a, 2));
    let vals = match exact_numbers(r.values()) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let (mut pos_flag, mut neg_flag) = (false, false);
    let mut pos = Vec::new();
    let mut neg = Vec::new();
    for v in &vals {
        if *v > 0.0 {
            pos_flag = true;
            pos.push(*v);
            neg.push(0.0);
        } else if *v < 0.0 {
            neg_flag = true;
            neg.push(*v);
            pos.push(0.0);
        } else {
            neg.push(0.0);
            pos.push(0.0);
        }
    }
    if !pos_flag || !neg_flag {
        return err(ErrorType::Div0);
    }
    let cnt = vals.len() as f64;
    let nom = match npv_core(rrate, &pos) {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    let denom = match npv_core(frate, &neg) {
        Ok(x) => x,
        Err(e) => return err(e),
    };
    num(js_pow(-nom * js_pow(1.0 + rrate, cnt) / denom / (1.0 + frate), 1.0 / (cnt - 1.0)) - 1.0)
}

fn pduration(_: &Call, a: &[Arg]) -> Val {
    num((n(a, 2).ln() - n(a, 1).ln()) / (1.0 + n(a, 0)).ln())
}

/// `typeof v === 'number'` (typed numbers are objects in HyperFormula and fail).
fn plain_numbers(r: &super::super::value::RangeVal) -> Option<Vec<f64>> {
    r.values()
        .map(|v| match v {
            Scalar::Num(x, NumType::Raw) => Some(*x),
            _ => None,
        })
        .collect()
}

fn xnpv(_: &Call, a: &[Arg]) -> Val {
    let rate = n(a, 0);
    let Some(vals) = plain_numbers(&a[1].to_range()) else { return err(ErrorType::Value) };
    let Some(mut dates) = plain_numbers(&a[2].to_range()) else { return err(ErrorType::Value) };
    if dates.len() != vals.len() {
        return err(ErrorType::Num);
    }
    let mut ret = 0.0;
    if dates[0] < 0.0 {
        return err(ErrorType::Num);
    }
    for i in 0..dates.len() {
        dates[i] = dates[i].floor();
        if dates[i] < dates[0] {
            return err(ErrorType::Num);
        }
        ret += vals[i] / js_pow(1.0 + rate, (dates[i] - dates[0]) / 365.0);
    }
    num(ret)
}

fn irr_core(values: &[f64], guess: f64) -> Val {
    let eps = 1e-10;
    let mut rate = guess;
    for _ in 0..50 {
        let mut npv = 0.0;
        let mut dnpv = 0.0;
        for (i, v) in values.iter().enumerate() {
            let factor = js_pow(1.0 + rate, i as f64);
            if !factor.is_finite() || factor == 0.0 {
                return err(ErrorType::Num);
            }
            npv += v / factor;
            if i > 0 {
                dnpv -= i as f64 * v / (factor * (1.0 + rate));
            }
        }
        if npv.abs() < eps {
            return num(rate);
        }
        if dnpv.abs() < eps {
            return err(ErrorType::Num);
        }
        let mut new_rate = rate - npv / dnpv;
        if !new_rate.is_finite() {
            return err(ErrorType::Num);
        }
        if new_rate <= -1.0 {
            new_rate = (rate - 1.0) / 2.0;
        }
        if (new_rate - rate).abs() < eps {
            return num(new_rate);
        }
        rate = new_rate;
    }
    err(ErrorType::Num)
}

fn irr(_: &Call, a: &[Arg]) -> Val {
    let guess = n(a, 1);
    if guess <= -1.0 {
        return err(ErrorType::Value);
    }
    let vals = match exact_numbers(a[0].to_range().values()) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if !vals.iter().any(|v| *v > 0.0) || !vals.iter().any(|v| *v < 0.0) {
        return err(ErrorType::Num);
    }
    irr_core(&vals, guess)
}

fn sanitize_xirr(r: &super::super::value::RangeVal) -> Result<Vec<f64>, ErrorType> {
    let mut out = Vec::new();
    for v in r.values() {
        match v {
            Scalar::Err(e) => return Err(*e),
            Scalar::Empty => out.push(0.0),
            Scalar::Num(x, _) => out.push(*x),
            _ => return Err(ErrorType::Value),
        }
    }
    Ok(out)
}

fn xirr(_: &Call, a: &[Arg]) -> Val {
    let guess = n(a, 2);
    if guess <= -1.0 {
        return err(ErrorType::Num);
    }
    let flows = match sanitize_xirr(&a[0].to_range()) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let dates = match sanitize_xirr(&a[1].to_range()) {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if flows.len() != dates.len() {
        return err(ErrorType::Num);
    }
    if flows.len() < 2 {
        return err(ErrorType::Na);
    }
    if !flows.iter().any(|v| *v > 0.0) || !flows.iter().any(|v| *v < 0.0) {
        return err(ErrorType::Num);
    }
    let eps = 1e-10;
    let start = dates[0].floor();
    if start < 0.0 {
        return err(ErrorType::Num);
    }
    let mut offsets = Vec::with_capacity(dates.len());
    for d in &dates {
        let t = d.floor();
        if t < start {
            return err(ErrorType::Num);
        }
        offsets.push(t - start);
    }
    let mut rate = guess;
    for _ in 0..50 {
        let mut npv = 0.0;
        let mut dnpv = 0.0;
        for (i, v) in flows.iter().enumerate() {
            let exponent = offsets[i] / 365.0;
            let base = 1.0 + rate;
            let factor = js_pow(base, exponent);
            if !factor.is_finite() || factor == 0.0 {
                return err(ErrorType::Num);
            }
            npv += v / factor;
            dnpv -= exponent * v / (factor * base);
        }
        if !npv.is_finite() || !dnpv.is_finite() {
            return err(ErrorType::Num);
        }
        if npv.abs() < eps {
            return num(rate);
        }
        if dnpv.abs() < eps {
            return err(ErrorType::Num);
        }
        let mut new_rate = rate - npv / dnpv;
        if !new_rate.is_finite() {
            return err(ErrorType::Num);
        }
        if new_rate <= -1.0 {
            new_rate = (rate - 1.0) / 2.0;
        }
        if (new_rate - rate).abs() < eps {
            return num(new_rate);
        }
        rate = new_rate;
    }
    err(ErrorType::Num)
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("PMT", pmt),
        ("IPMT", ipmt),
        ("PPMT", ppmt),
        ("FV", fv),
        ("CUMIPMT", cumipmt),
        ("CUMPRINC", cumprinc),
        ("DB", db),
        ("DDB", ddb),
        ("DOLLARDE", dollarde),
        ("DOLLARFR", dollarfr),
        ("EFFECT", effect),
        ("ISPMT", ispmt),
        ("NOMINAL", nominal),
        ("NPER", nper),
        ("RATE", rate),
        ("PV", pv),
        ("RRI", rri),
        ("SLN", sln),
        ("SYD", syd),
        ("TBILLEQ", tbilleq),
        ("TBILLPRICE", tbillprice),
        ("TBILLYIELD", tbillyield),
        ("FVSCHEDULE", fvschedule),
        ("NPV", npv),
        ("MIRR", mirr),
        ("PDURATION", pduration),
        ("XNPV", xnpv),
        ("IRR", irr),
        ("XIRR", xirr),
    ])
}
