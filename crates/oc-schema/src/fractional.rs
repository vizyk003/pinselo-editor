// SPDX-License-Identifier: Apache-2.0
//! Exact port of the `fractional-indexing` npm package v4.0.0 (CC0, rocicorp), default alphabets only:
//! digits = base62 (`0-9A-Za-z`), integer heads = base52 (`A-Za-z`). Keys and error messages match the JS
//! implementation byte for byte.

use thiserror::Error;

pub const BASE_62_DIGITS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
pub const BASE_52_DIGITS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Error thrown by the JS implementation (same message).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct FractionalIndexError(pub String);

type R<T> = Result<T, FractionalIndexError>;

struct Alpha {
    digits: &'static [u8],
    int_digits: &'static [u8],
    lookup: [u8; 256],
    int_lookup: [u8; 256],
}

fn table(digits: &[u8]) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, d) in digits.iter().enumerate() {
        m[*d as usize] = i as u8;
    }
    m
}

impl Alpha {
    fn default_alpha() -> Self {
        Alpha {
            digits: BASE_62_DIGITS.as_bytes(),
            int_digits: BASE_52_DIGITS.as_bytes(),
            lookup: table(BASE_62_DIGITS.as_bytes()),
            int_lookup: table(BASE_52_DIGITS.as_bytes()),
        }
    }
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn err<T>(m: String) -> R<T> {
    Err(FractionalIndexError(m))
}

fn midpoint(a: &[u8], b: Option<&[u8]>, al: &Alpha) -> R<Vec<u8>> {
    let zero = al.digits[0];
    if let Some(b) = b
        && a >= b
    {
        return err(format!("{} >= {}", s(a), s(b)));
    }
    if a.last() == Some(&zero) || b.is_some_and(|b| b.last() == Some(&zero)) {
        return err("trailing zero".into());
    }
    if let Some(b) = b
        && !b.is_empty()
    {
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(zero) == b[n] {
            n += 1;
        }
        if n > 0 {
            let rest_a = if n <= a.len() { &a[n..] } else { &[][..] };
            let mut out = b[..n].to_vec();
            out.extend(midpoint(rest_a, Some(&b[n..]), al)?);
            return Ok(out);
        }
    }
    let digit_a = a.first().map_or(0usize, |c| al.lookup[*c as usize] as usize);
    // `lookup[b.charCodeAt(0)]` is undefined (NaN arithmetic) for an empty `b`.
    let digit_b: Option<usize> = match b {
        Some(b) => b.first().map(|c| al.lookup[*c as usize] as usize),
        None => Some(al.digits.len()),
    };
    if let Some(db) = digit_b
        && db as i64 - digit_a as i64 > 1
    {
        let mid = (digit_a + db).div_ceil(2); // Math.round(0.5 * (a + b))
        return Ok(vec![al.digits[mid]]);
    }
    match b {
        Some(b) if b.len() > 1 => Ok(b[..1].to_vec()),
        _ => {
            let mut out = vec![al.digits[digit_a]];
            let rest = if a.is_empty() { &[][..] } else { &a[1..] };
            out.extend(midpoint(rest, None, al)?);
            Ok(out)
        }
    }
}

fn integer_length(head: Option<u8>, al: &Alpha) -> R<usize> {
    let Some(head) = head else {
        return err("Cannot read properties of undefined (reading 'charCodeAt')".into());
    };
    let i = al.int_lookup[head as usize] as usize;
    if al.int_digits[i] == head {
        let half = al.int_digits.len() / 2;
        return Ok(if i < half { half - i + 1 } else { i - half + 2 });
    }
    err(format!("invalid order key head: {}", head as char))
}

fn validate_integer(int: &[u8], al: &Alpha) -> R<()> {
    if int.len() != integer_length(int.first().copied(), al)? {
        return err(format!("invalid integer part of order key: {}", s(int)));
    }
    Ok(())
}

fn integer_part<'a>(key: &'a [u8], al: &Alpha) -> R<&'a [u8]> {
    let len = integer_length(key.first().copied(), al)?;
    if len > key.len() {
        return err(format!("invalid order key: {}", s(key)));
    }
    Ok(&key[..len])
}

fn is_smallest_integer(key: &[u8], al: &Alpha) -> bool {
    let n = al.int_digits.len() / 2;
    key.len() == n + 1 && key[0] == al.int_digits[0] && key[1..].iter().all(|c| *c == al.digits[0])
}

fn validate_order_key(key: &[u8], al: &Alpha) -> R<()> {
    if is_smallest_integer(key, al) {
        return err(format!("invalid order key: {}", s(key)));
    }
    let i = integer_part(key, al)?;
    let f = &key[i.len()..];
    if f.last() == Some(&al.digits[0]) {
        return err(format!("invalid order key: {}", s(key)));
    }
    Ok(())
}

fn increment_integer(x: &[u8], al: &Alpha) -> R<Option<Vec<u8>>> {
    validate_integer(x, al)?;
    let head = x[0];
    let zero = al.digits[0];
    let mut trailing: Vec<u8> = Vec::new();
    for i in (1..x.len()).rev() {
        let d = al.lookup[x[i] as usize] as usize + 1;
        if d == al.digits.len() {
            trailing.insert(0, zero);
        } else {
            let mut out = vec![head];
            out.extend_from_slice(&x[1..i]);
            out.push(al.digits[d]);
            out.extend(trailing);
            return Ok(Some(out));
        }
    }
    let head_index = al.int_lookup[head as usize] as usize;
    if head_index == al.int_digits.len() - 1 {
        return Ok(None);
    }
    let h = al.int_digits[head_index + 1];
    let delta = integer_length(Some(h), al)? as i64 - integer_length(Some(head), al)? as i64;
    let mut out = vec![h];
    if delta > 0 {
        out.extend(trailing);
        out.push(zero);
    } else if delta < 0 {
        out.extend(trailing.into_iter().skip(1));
    } else {
        out.extend(trailing);
    }
    Ok(Some(out))
}

fn decrement_integer(x: &[u8], al: &Alpha) -> R<Option<Vec<u8>>> {
    validate_integer(x, al)?;
    let head = x[0];
    let last = *al.digits.last().expect("digits");
    let mut trailing: Vec<u8> = Vec::new();
    for i in (1..x.len()).rev() {
        let d = al.lookup[x[i] as usize] as i64 - 1;
        if d == -1 {
            trailing.insert(0, last);
        } else {
            let mut out = vec![head];
            out.extend_from_slice(&x[1..i]);
            out.push(al.digits[d as usize]);
            out.extend(trailing);
            return Ok(Some(out));
        }
    }
    let head_index = al.int_lookup[head as usize] as usize;
    if head_index == 0 {
        return Ok(None);
    }
    let h = al.int_digits[head_index - 1];
    let delta = integer_length(Some(h), al)? as i64 - integer_length(Some(head), al)? as i64;
    let mut out = vec![h];
    if delta > 0 {
        out.extend(trailing);
        out.push(last);
    } else if delta < 0 {
        out.extend(trailing.into_iter().skip(1));
    } else {
        out.extend(trailing);
    }
    Ok(Some(out))
}

fn check_bytes(k: &str) -> R<&[u8]> {
    if !k.is_ascii() {
        return err(format!("invalid order key: {k}"));
    }
    Ok(k.as_bytes())
}

fn key_between(a: Option<&[u8]>, b: Option<&[u8]>, al: &Alpha) -> R<Vec<u8>> {
    if let Some(a) = a {
        validate_order_key(a, al)?;
    }
    if let Some(b) = b {
        validate_order_key(b, al)?;
    }
    let (a, b) = match (a, b) {
        (Some(x), Some(y)) if x > y => (Some(y), Some(x)),
        other => other,
    };
    match (a, b) {
        (None, None) => Ok(vec![al.int_digits[al.int_digits.len() / 2], al.digits[0]]),
        (None, Some(b)) => {
            let ib = integer_part(b, al)?;
            let fb = &b[ib.len()..];
            if is_smallest_integer(ib, al) {
                let mut out = ib.to_vec();
                out.extend(midpoint(&[], Some(fb), al)?);
                return Ok(out);
            }
            if ib < b {
                return Ok(ib.to_vec());
            }
            match decrement_integer(ib, al)? {
                Some(r) => Ok(r),
                None => err("cannot decrement any more".into()),
            }
        }
        (Some(a), None) => {
            let ia = integer_part(a, al)?;
            let fa = &a[ia.len()..];
            match increment_integer(ia, al)? {
                Some(i) => Ok(i),
                None => {
                    let mut out = ia.to_vec();
                    out.extend(midpoint(fa, None, al)?);
                    Ok(out)
                }
            }
        }
        (Some(a), Some(b)) => {
            let ia = integer_part(a, al)?;
            let fa = &a[ia.len()..];
            let ib = integer_part(b, al)?;
            let fb = &b[ib.len()..];
            if ia == ib {
                let mut out = ia.to_vec();
                out.extend(midpoint(fa, Some(fb), al)?);
                return Ok(out);
            }
            let Some(i) = increment_integer(ia, al)? else {
                return err("cannot increment any more".into());
            };
            if i.as_slice() < b {
                return Ok(i);
            }
            let mut out = ia.to_vec();
            out.extend(midpoint(fa, None, al)?);
            Ok(out)
        }
    }
}

/// `generateKeyBetween(a, b)`: a key sorting between `a` and `b` (`None` = open end). Bounds may be swapped.
pub fn generate_key_between(a: Option<&str>, b: Option<&str>) -> Result<String, FractionalIndexError> {
    let al = Alpha::default_alpha();
    let a = a.map(check_bytes).transpose()?;
    let b = b.map(check_bytes).transpose()?;
    key_between(a, b, &al).map(|v| s(&v))
}

fn n_keys(a: Option<&[u8]>, b: Option<&[u8]>, n: usize, al: &Alpha) -> R<Vec<Vec<u8>>> {
    if n == 0 {
        return Ok(vec![]);
    }
    if n == 1 {
        return Ok(vec![key_between(a, b, al)?]);
    }
    if b.is_none() {
        let mut c = key_between(a, b, al)?;
        let mut out = vec![c.clone()];
        for _ in 0..n - 1 {
            c = key_between(Some(&c), b, al)?;
            out.push(c.clone());
        }
        return Ok(out);
    }
    if a.is_none() {
        let mut c = key_between(a, b, al)?;
        let mut out = vec![c.clone()];
        for _ in 0..n - 1 {
            c = key_between(a, Some(&c), al)?;
            out.push(c.clone());
        }
        out.reverse();
        return Ok(out);
    }
    let mid = n / 2;
    let c = key_between(a, b, al)?;
    let mut out = n_keys(a, Some(&c), mid, al)?;
    out.push(c.clone());
    out.extend(n_keys(Some(&c), b, n - mid - 1, al)?);
    Ok(out)
}

/// `generateNKeysBetween(a, b, n)`: `n` distinct sorted keys between `a` and `b`.
pub fn generate_n_keys_between(a: Option<&str>, b: Option<&str>, n: usize) -> Result<Vec<String>, FractionalIndexError> {
    let al = Alpha::default_alpha();
    let a = a.map(check_bytes).transpose()?;
    let b = b.map(check_bytes).transpose()?;
    Ok(n_keys(a, b, n, &al)?.iter().map(|k| s(k)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(generate_key_between(None, None).unwrap(), "a0");
        assert_eq!(generate_key_between(Some("a0"), None).unwrap(), "a1");
        assert_eq!(generate_key_between(None, Some("a0")).unwrap(), "Zz");
        assert_eq!(generate_key_between(Some("a0"), Some("a1")).unwrap(), "a0V");
        assert_eq!(generate_key_between(Some("a1"), Some("a0")).unwrap(), "a0V");
        assert_eq!(generate_key_between(Some("az"), None).unwrap(), "b00");
        assert_eq!(generate_n_keys_between(None, None, 3).unwrap(), vec!["a0", "a1", "a2"]);
    }
}
