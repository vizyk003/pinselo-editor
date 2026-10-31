// SPDX-License-Identifier: Apache-2.0
//! Ids and fractional ordering keys.

use std::cmp::Ordering;

pub use crate::fractional::FractionalIndexError;
use crate::fractional::{generate_key_between, generate_n_keys_between};
use crate::js;

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    // The OS / Web Crypto RNG never fails in practice; a failure is unrecoverable for id generation.
    getrandom::fill(&mut out).expect("secure random source unavailable");
    out
}

/// 16-character base62 id, generated client-side. Always contains a digit (redrawn otherwise, ~6 % of draws), so an id
/// never reads as a 16-letter word: the API parity harness tells generated ids from words by that digit.
pub fn new_id() -> String {
    loop {
        let id: String = random_bytes::<16>().iter().map(|b| BASE62[(*b % 62) as usize] as char).collect();
        if id.bytes().any(|b| b.is_ascii_digit()) {
            return id;
        }
    }
}

/// `/^[0-9A-Za-z]{16}$/`.
pub fn is_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Fractional index between `a` and `b` (`None` = open end).
/// Jittered: picks one of six evenly spaced candidate keys at random so concurrent inserts rarely collide.
pub fn key_between(a: Option<&str>, b: Option<&str>, jitter: bool) -> Result<String, FractionalIndexError> {
    if !jitter {
        return generate_key_between(a, b);
    }
    let keys = generate_n_keys_between(a, b, 6)?;
    let pick = random_bytes::<1>()[0] as usize % keys.len();
    Ok(keys[pick].clone())
}

pub fn keys_between(a: Option<&str>, b: Option<&str>, n: usize) -> Result<Vec<String>, FractionalIndexError> {
    generate_n_keys_between(a, b, n)
}

/// Order by `(key, id)` with JS string comparison.
pub fn compare_order(a_key: &str, a_id: &str, b_key: &str, b_id: &str) -> Ordering {
    js::str_cmp(a_key, b_key).then_with(|| js::str_cmp(a_id, b_id))
}

/// The first key of an empty list (`keyBetween(null, null, false)`, always `"a0"`).
pub fn first_key() -> String {
    "a0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_base62_with_a_digit() {
        for _ in 0..10_000 {
            let id = new_id();
            assert!(is_id(&id), "{id}");
            assert!(id.bytes().any(|b| b.is_ascii_digit()), "{id}");
        }
    }
}
