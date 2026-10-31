// SPDX-License-Identifier: AGPL-3.0-only
//! TextPlugin, CharPlugin, CodePlugin, the Pinselo CONCAT plugin, and TEXT (HyperFormula's `format`).

use super::super::coerce::{self, Pattern};
use super::super::datetime::{SimpleDate, SimpleTime, number_to_simple_date_time, number_to_simple_time};
use super::super::func::{Arg, Call};
use super::super::value::{ErrorType, NumType, Scalar, Val};
use super::{Kind, err, from_u16, js_index_of, js_len, js_slice, js_substring, num, s, simple, to_int, u16s};
use crate::jsnum::{num_to_string, parse_num, to_fixed};

fn concatenate(_: &Call, a: &[Arg]) -> Val {
    let mut out = String::new();
    for x in a {
        out.push_str(x.str());
    }
    s(out)
}

fn split(_: &Call, a: &[Arg]) -> Val {
    let parts: Vec<&str> = a[0].str().split(' ').collect();
    let i = a[1].num();
    if i >= parts.len() as f64 || i < 0.0 {
        return err(ErrorType::Value);
    }
    s(parts[i as usize])
}

fn trim(_: &Call, a: &[Arg]) -> Val {
    let t = a[0].str().trim_matches(' ');
    let mut out = String::with_capacity(t.len());
    let mut prev_space = false;
    for c in t.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    s(out)
}

fn proper(_: &Call, a: &[Arg]) -> Val {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |w: &mut String, out: &mut String| {
        if !w.is_empty() {
            let mut cs = w.chars();
            if let Some(f) = cs.next() {
                out.extend(f.to_uppercase());
                out.push_str(&cs.as_str().to_lowercase());
            }
            w.clear();
        }
    };
    for c in a[0].str().chars() {
        if c.is_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    s(out)
}

fn rept(_: &Call, a: &[Arg]) -> Val {
    let n = a[1].num();
    if n < 0.0 {
        return err(ErrorType::Value);
    }
    let n = to_int(n);
    if n.is_infinite() || (n * a[0].str().len() as f64) > 1e8 {
        return err(ErrorType::Value);
    }
    s(a[0].str().repeat(n as usize))
}

fn right(_: &Call, a: &[Arg]) -> Val {
    let n = a[1].num();
    if n < 0.0 {
        return err(ErrorType::Value);
    }
    if n == 0.0 {
        return s("");
    }
    s(js_slice(a[0].str(), -n, None))
}

fn left(_: &Call, a: &[Arg]) -> Val {
    let n = a[1].num();
    if n < 0.0 {
        return err(ErrorType::Value);
    }
    s(js_slice(a[0].str(), 0.0, Some(n)))
}

fn mid(_: &Call, a: &[Arg]) -> Val {
    let (st, n) = (a[1].num(), a[2].num());
    if st < 1.0 {
        return err(ErrorType::Value);
    }
    if n < 0.0 {
        return err(ErrorType::Value);
    }
    s(js_substring(a[0].str(), st - 1.0, Some(st + n - 1.0)))
}

fn replace(_: &Call, a: &[Arg]) -> Val {
    let (st, n) = (a[1].num(), a[2].num());
    if st < 1.0 {
        return err(ErrorType::Value);
    }
    if n < 0.0 {
        return err(ErrorType::Value);
    }
    let t = a[0].str();
    s(format!("{}{}{}", js_substring(t, 0.0, Some(st - 1.0)), a[3].str(), js_substring(t, st + n - 1.0, None)))
}

fn search(_: &Call, a: &[Arg]) -> Val {
    let (pattern, text, start) = (a[0].str(), a[1].str(), a[2].num());
    if start < 1.0 || start > js_len(text) as f64 {
        return err(ErrorType::Value);
    }
    let np = pattern.to_lowercase();
    let nt = js_substring(text, start - 1.0, None).to_lowercase();
    let index = if coerce::requires_regex(&np) {
        match Pattern::new(&np, false).search(&nt) {
            Some(ci) => nt.chars().take(ci).map(char::len_utf16).sum::<usize>() as i64,
            None => -1,
        }
    } else {
        js_index_of(&nt, &np)
    };
    if index > -1 { num(index as f64 + start) } else { err(ErrorType::Value) }
}

/// JS `String#replace` replacement-pattern expansion (`$$`, `$&`, `` $` ``, `$'`).
fn expand_replacement(rep: &str, m: &str, before: &str, after: &str) -> String {
    let mut out = String::new();
    let mut it = rep.chars().peekable();
    while let Some(c) = it.next() {
        if c == '$' {
            match it.peek() {
                Some('$') => {
                    out.push('$');
                    it.next();
                }
                Some('&') => {
                    out.push_str(m);
                    it.next();
                }
                Some('`') => {
                    out.push_str(before);
                    it.next();
                }
                Some('\'') => {
                    out.push_str(after);
                    it.next();
                }
                _ => out.push('$'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn substitute(_: &Call, a: &[Arg]) -> Val {
    let (text, search, rep) = (a[0].str(), a[1].str(), a[2].str());
    let t = u16s(text);
    let sv = u16s(search);
    let mut matches: Vec<usize> = Vec::new();
    if sv.is_empty() {
        matches.extend(0..=t.len());
    } else {
        let mut i = 0;
        while i + sv.len() <= t.len() {
            if t[i..i + sv.len()] == sv[..] {
                matches.push(i);
                i += sv.len();
            } else {
                i += 1;
            }
        }
    }
    match a[3].opt_num() {
        None => {
            let mut out: Vec<u16> = Vec::new();
            let mut last = 0;
            for &m in &matches {
                out.extend_from_slice(&t[last..m]);
                let r = expand_replacement(rep, search, &from_u16(&t[..m]), &from_u16(&t[m + sv.len()..]));
                out.extend(r.encode_utf16());
                last = m + sv.len();
            }
            out.extend_from_slice(&t[last.min(t.len())..]);
            s(from_u16(&out))
        }
        Some(n) => {
            if n < 1.0 {
                return err(ErrorType::Value);
            }
            for (i, &m) in matches.iter().enumerate() {
                if n == (i + 1) as f64 {
                    return s(format!("{}{}{}", from_u16(&t[..m]), rep, from_u16(&t[m + sv.len()..])));
                }
            }
            s(text)
        }
    }
}

fn find(_: &Call, a: &[Arg]) -> Val {
    let (pattern, text, start) = (a[0].str(), a[1].str(), a[2].num());
    if start < 1.0 || start > js_len(text) as f64 {
        return err(ErrorType::Value);
    }
    let shifted = js_substring(text, start - 1.0, None);
    let index = js_index_of(&shifted, pattern) as f64 + start;
    if index > 0.0 { num(index) } else { err(ErrorType::Value) }
}

fn t(_: &Call, a: &[Arg]) -> Val {
    match &a[0] {
        Arg::S(Scalar::Err(e)) => err(*e),
        Arg::S(Scalar::Str(x)) => s(x.clone()),
        _ => s(""),
    }
}

fn n(_: &Call, a: &[Arg]) -> Val {
    let v = match &a[0] {
        Arg::R(r) => r.data.first().and_then(|x| x.first()).cloned().unwrap_or(Scalar::Empty),
        Arg::S(x) => x.clone(),
        Arg::Missing => Scalar::Empty,
        c @ Arg::Complex(..) => c.scalar(),
    };
    match v {
        Scalar::Err(e) => err(e),
        Scalar::Num(x, NumType::Raw) => num(x),
        Scalar::Bool(b) => num(if b { 1.0 } else { 0.0 }),
        _ => num(0.0),
    }
}

fn parse_string_to_number(input: &str) -> Option<Scalar> {
    let t = oc_schema::js::trim(input);
    if t.is_empty() {
        return None;
    }
    coerce::to_maybe_number(&Scalar::str(t))
}

fn value(_: &Call, a: &[Arg]) -> Val {
    let arg = a[0].scalar();
    match &arg {
        Scalar::Err(e) => return err(*e),
        Scalar::Num(..) => return Val::S(arg),
        Scalar::Str(_) => {}
        _ => return err(ErrorType::Value),
    }
    let Scalar::Str(text) = arg else { unreachable!() };
    let trimmed = oc_schema::js::trim(&text);
    if let Some(inner) = trimmed.strip_prefix('(').and_then(|x| x.strip_suffix(')'))
        && !inner.is_empty()
        && !inner.contains(['(', ')'])
        && let Some(v) = parse_string_to_number(inner)
    {
        return match v {
            Scalar::Num(x, NumType::Raw) => num(-x),
            // `-richNumber` is NaN in JS.
            _ => num(f64::NAN),
        };
    }
    match parse_string_to_number(trimmed) {
        Some(v) => Val::S(v),
        None => err(ErrorType::Value),
    }
}

fn flatten_strings(a: &Arg) -> Result<Vec<String>, ErrorType> {
    let vals: Vec<Scalar> = match a {
        Arg::R(r) => r.values().cloned().collect(),
        Arg::S(x) => vec![x.clone()],
        Arg::Missing => vec![Scalar::Empty],
        c @ Arg::Complex(..) => vec![c.scalar()],
    };
    let mut out = Vec::new();
    for v in vals {
        match coerce::to_string(&v) {
            Scalar::Err(e) => return Err(e),
            Scalar::Str(x) => out.push(x.to_string()),
            _ => out.push(String::new()),
        }
    }
    Ok(out)
}

fn textjoin(_: &Call, a: &[Arg]) -> Val {
    let delims = match flatten_strings(&a[0]) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    let ignore = a[1].bool();
    let mut texts = Vec::new();
    for x in &a[2..] {
        match flatten_strings(x) {
            Ok(v) => texts.extend(v),
            Err(e) => return err(e),
        }
    }
    let parts: Vec<String> = if ignore { texts.into_iter().filter(|t| !t.is_empty()).collect() } else { texts };
    if parts.is_empty() {
        return s("");
    }
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 && !delims.is_empty() {
            out.push_str(&delims[(i - 1) % delims.len()]);
        } else if i > 0 {
            out.push_str("undefined");
        }
        out.push_str(p);
    }
    if js_len(&out) > 32767 {
        return err(ErrorType::Value);
    }
    s(out)
}

fn char_fn(_: &Call, a: &[Arg]) -> Val {
    let v = a[0].num();
    if !(1.0..256.0).contains(&v) {
        return err(ErrorType::Value);
    }
    s(char::from_u32(v.trunc() as u32).map(String::from).unwrap_or_default())
}

fn unichar(_: &Call, a: &[Arg]) -> Val {
    let v = a[0].num();
    if !(1.0..1_114_112.0).contains(&v) {
        return err(ErrorType::Value);
    }
    let cp = v.trunc() as u32;
    s(char::from_u32(cp).map_or_else(|| String::from('\u{FFFD}'), String::from))
}

fn code(_: &Call, a: &[Arg]) -> Val {
    match a[0].str().encode_utf16().next() {
        None => err(ErrorType::Value),
        Some(u) => num(u as f64),
    }
}

fn unicode(_: &Call, a: &[Arg]) -> Val {
    match a[0].str().chars().next() {
        None => err(ErrorType::Value),
        Some(c) => num(c as u32 as f64),
    }
}

// ------------------------------------------------------------------ TEXT(number, format)

#[derive(Debug, Clone, PartialEq)]
enum FmtTok {
    Format(String),
    Free(String),
}

const DATE_ALTS: [&str; 22] = [
    "dd", "DD", "d", "D", "mm", "MM", "m", "M", "YYYY", "YY", "yyyy", "yy", "HH", "hh", "H", "h", "ss", "s", "AM/PM", "am/pm", "A/P", "a/p",
];

/// One `dateFormatRegex` match at char index `i` (length in chars).
fn date_token_at(c: &[char], i: usize) -> Option<usize> {
    if c[i] == '\\' {
        return (i + 1 < c.len() && !matches!(c[i + 1], '\n' | '\r' | '\u{2028}' | '\u{2029}')).then_some(2);
    }
    let rest: String = c[i..].iter().take(6).collect();
    for alt in DATE_ALTS {
        if rest.starts_with(alt) {
            if alt == "ss" {
                // ss(\.(0+|s+))?
                let mut j = i + 2;
                if c.get(j) == Some(&'.') {
                    let k0 = j + 1;
                    let ch = c.get(k0).copied();
                    if ch == Some('0') || ch == Some('s') {
                        let mut k = k0;
                        while c.get(k) == ch.as_ref() {
                            k += 1;
                        }
                        j = k;
                    }
                }
                return Some(j - i);
            }
            return Some(alt.chars().count());
        }
    }
    for alt in ["[mm]", "[MM]", "[hh]", "[HH]"] {
        if rest.starts_with(alt) {
            return Some(4);
        }
    }
    None
}

fn tokens_from(c: &[char], matches: &[(usize, usize)]) -> Vec<FmtTok> {
    let mut out = Vec::new();
    let mut start = 0;
    for &(i, len) in matches {
        if i != start {
            out.push(FmtTok::Free(c[start..i].iter().collect()));
        }
        let v: String = c[i..i + len].iter().collect();
        if v.starts_with('\\') {
            out.push(FmtTok::Free(v));
        } else {
            out.push(FmtTok::Format(v));
        }
        start = i + len;
    }
    if let Some(&(i, len)) = matches.last()
        && i + len < c.len()
    {
        out.push(FmtTok::Free(c[i + len..].iter().collect()));
    }
    out
}

fn parse_date_format(fmt: &str) -> Option<Vec<FmtTok>> {
    let c: Vec<char> = fmt.chars().collect();
    let mut matches = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match date_token_at(&c, i) {
            Some(len) => {
                matches.push((i, len));
                i += len;
            }
            None => i += 1,
        }
    }
    if matches.iter().all(|(i, _)| c[*i] == '\\') {
        return None;
    }
    Some(tokens_from(&c, &matches))
}

fn parse_number_format(fmt: &str) -> Option<Vec<FmtTok>> {
    let c: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && i + 1 < c.len() && !matches!(c[i + 1], '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            return None; // the first match is an escape → every(isEscape)
        }
        if c[i] == '#' || c[i] == '0' {
            let mut j = i;
            while j < c.len() && (c[j] == '#' || c[j] == '0') {
                j += 1;
            }
            if c.get(j) == Some(&'.') {
                j += 1;
                while j < c.len() && (c[j] == '#' || c[j] == '0') {
                    j += 1;
                }
            }
            return Some(tokens_from(&c, &[(i, j - i)]));
        }
        i += 1;
    }
    None
}

fn pad_left(v: &str, size: usize) -> String {
    let len = js_len(v);
    if len >= size { v.to_string() } else { format!("{}{v}", "0".repeat(size - len)) }
}

fn is_seconds_token(v: &str) -> bool {
    let Some(rest) = v.strip_prefix("ss") else { return false };
    if rest.is_empty() {
        return true;
    }
    let Some(r) = rest.strip_prefix('.') else { return false };
    !r.is_empty() && (r.chars().all(|c| c == 's') || r.chars().all(|c| c == '0'))
}

fn fraction_seconds(seconds: f64, precision: usize) -> String {
    let p = 10f64.powi(precision as i32);
    format!("{}{}", if seconds < 10.0 { "0" } else { "" }, num_to_string((seconds * p).floor() / p))
}

fn lcid_currency_tag(f: &str) -> bool {
    // /\[\$[^\-\]]+-/
    let c: Vec<char> = f.chars().collect();
    for i in 0..c.len() {
        if c[i] == '[' && c.get(i + 1) == Some(&'$') {
            let mut j = i + 2;
            while j < c.len() && c[j] != '-' && c[j] != ']' {
                j += 1;
            }
            if j > i + 2 && c.get(j) == Some(&'-') {
                return true;
            }
        }
    }
    false
}

fn stringify_date_time(d: &SimpleDate, t: &SimpleTime, fmt: &str) -> Option<String> {
    if lcid_currency_tag(fmt) {
        return None;
    }
    let tokens = parse_date_format(fmt)?;
    let ampm = tokens.iter().any(|x| matches!(x, FmtTok::Format(v) if v == "a/p" || v == "A/P" || v == "am/pm" || v == "AM/PM"));
    let mut out = String::new();
    let mut minutes = false;
    for (i, tok) in tokens.iter().enumerate() {
        let v = match tok {
            FmtTok::Free(x) => {
                out.push_str(x);
                continue;
            }
            FmtTok::Format(v) => v,
        };
        let len = js_len(v);
        match v.to_lowercase().as_str() {
            "h" | "hh" => {
                minutes = true;
                let h = if ampm { (t.hours + 11.0) % 12.0 + 1.0 } else { t.hours };
                out.push_str(&pad_left(&num_to_string(h), len));
            }
            "d" | "dd" => out.push_str(&pad_left(&num_to_string(d.day), len)),
            "s" | "ss" => out.push_str(&pad_left(&num_to_string(t.seconds.floor()), len)),
            "m" | "mm" => {
                let next_colon = tokens.get(i + 1).is_some_and(|n| match n {
                    FmtTok::Free(x) | FmtTok::Format(x) => x.starts_with(':'),
                });
                if next_colon {
                    minutes = true;
                }
                if minutes {
                    out.push_str(&pad_left(&num_to_string(t.minutes), len));
                } else {
                    out.push_str(&pad_left(&num_to_string(d.month), len));
                }
                minutes = true;
            }
            "yy" => out.push_str(&pad_left(&num_to_string(d.year % 100.0), len)),
            "yyyy" => out.push_str(&num_to_string(d.year)),
            "am/pm" | "a/p" => {
                let (am, pm) = v.split_once('/').unwrap_or(("", ""));
                out.push_str(if t.hours < 12.0 { am } else { pm });
            }
            _ => {
                if is_seconds_token(v) {
                    out.push_str(&fraction_seconds(t.seconds, len - 3));
                    continue;
                }
                return None;
            }
        }
    }
    Some(out)
}

fn stringify_duration(mut t: SimpleTime, fmt: &str) -> Option<String> {
    if lcid_currency_tag(fmt) {
        return None;
    }
    let tokens = parse_date_format(fmt)?;
    let mut out = String::new();
    for tok in &tokens {
        let v = match tok {
            FmtTok::Free(x) => {
                out.push_str(x);
                continue;
            }
            FmtTok::Format(v) => v,
        };
        let len = js_len(v);
        match v.to_lowercase().as_str() {
            "h" | "hh" => {
                out.push_str(&pad_left(&num_to_string(t.hours), len));
                t.hours = 0.0;
            }
            "[hh]" => {
                out.push_str(&pad_left(&num_to_string(t.hours), len - 2));
                t.hours = 0.0;
            }
            "m" | "mm" => {
                out.push_str(&pad_left(&num_to_string(t.minutes), len));
                t.minutes = 0.0;
            }
            "[mm]" => {
                out.push_str(&pad_left(&num_to_string(t.minutes + 60.0 * t.hours), len - 2));
                t.minutes = 0.0;
                t.hours = 0.0;
            }
            "s" | "ss" => out.push_str(&pad_left(&num_to_string(t.seconds.floor()), len)),
            _ => {
                if is_seconds_token(v) {
                    out.push_str(&fraction_seconds(t.seconds, len.saturating_sub(3)));
                    continue;
                }
                return None;
            }
        }
    }
    Some(out)
}

fn number_format(tokens: &[FmtTok], value: f64) -> String {
    let mut out = String::new();
    for tok in tokens {
        let v = match tok {
            FmtTok::Free(x) => {
                out.push_str(x);
                continue;
            }
            FmtTok::Format(v) => v,
        };
        let (int_fmt, dec_fmt) = match v.split_once('.') {
            Some((a, b)) => (a, b),
            None => (v.as_str(), ""),
        };
        let separator = if dec_fmt.is_empty() { "" } else { "." };
        let fixed = num_to_string(parse_num(&to_fixed(value, dec_fmt.len())));
        let (mut ip, mut dp) = match fixed.split_once('.') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (fixed.clone(), String::new()),
        };
        if int_fmt.len() > ip.len() {
            let pad = int_fmt[..int_fmt.len() - ip.len()].matches('0').count();
            ip = pad_left(&ip, pad + ip.len());
        }
        let pad_dec = dec_fmt.get(dp.len()..).map_or(0, |x| x.matches('0').count());
        dp.push_str(&"0".repeat(pad_dec));
        out.push_str(&ip);
        out.push_str(separator);
        out.push_str(&dp);
    }
    out
}

/// HyperFormula's `format(value, formatArg)` (TEXT).
pub fn text_format(value: f64, fmt: &str) -> String {
    let (d, t) = number_to_simple_date_time(value);
    if let Some(r) = stringify_date_time(&d, &t, fmt) {
        return r;
    }
    if let Some(r) = stringify_duration(number_to_simple_time(value), fmt) {
        return r;
    }
    if let Some(tokens) = parse_number_format(fmt) {
        return number_format(&tokens, value);
    }
    fmt.to_string()
}

fn text(_: &Call, a: &[Arg]) -> Val {
    s(text_format(a[0].num(), a[1].str()))
}

pub fn functions() -> Vec<(&'static str, Kind)> {
    simple(&[
        ("CONCATENATE", concatenate),
        ("CONCAT", concatenate),
        ("SPLIT", split),
        ("LEN", |_, a| num(js_len(a[0].str()) as f64)),
        ("LOWER", |_, a| s(a[0].str().to_lowercase())),
        ("UPPER", |_, a| s(a[0].str().to_uppercase())),
        ("TRIM", trim),
        ("PROPER", proper),
        ("CLEAN", |_, a| s(a[0].str().chars().filter(|c| *c as u32 > 0x1f).collect::<String>())),
        ("EXACT", |_, a| Val::bool(a[0].str() == a[1].str())),
        ("REPT", rept),
        ("RIGHT", right),
        ("LEFT", left),
        ("MID", mid),
        ("REPLACE", replace),
        ("SEARCH", search),
        ("SUBSTITUTE", substitute),
        ("FIND", find),
        ("T", t),
        ("N", n),
        ("VALUE", value),
        ("TEXTJOIN", textjoin),
        ("CHAR", char_fn),
        ("UNICHAR", unichar),
        ("CODE", code),
        ("UNICODE", unicode),
        ("TEXT", text),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(text_format(45306.5, "yyyy-mm-dd"), "2024-01-15");
        assert_eq!(text_format(45306.5, "hh:mm"), "12:00");
        assert_eq!(text_format(1234.567, "0.00"), "1234.57");
        assert_eq!(text_format(5.0, "000"), "005");
    }
}
