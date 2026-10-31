// SPDX-License-Identifier: Apache-2.0
//! Randomness for ids and jittered ordering keys.
//!
//! `@oc/schema`'s `newId` (16 bytes) and `keyBetween` (1 byte) draw from `crypto.getRandomValues`. The ops here
//! draw the same bytes, in the same order, from a [`RandomSource`], so with a deterministic source (the golden
//! generator replaces `crypto.getRandomValues` with [`Mulberry32`]) TS and Rust produce identical ids and keys.

use oc_schema::fractional::{FractionalIndexError, generate_n_keys_between};

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// A source of random bytes (`crypto.getRandomValues`).
pub trait RandomSource {
    fn fill(&mut self, buf: &mut [u8]);
}

/// The OS / Web Crypto RNG (what `@oc/schema` uses).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, buf: &mut [u8]) {
        // Same policy as oc-schema: the OS / Web Crypto RNG never fails in practice, and a failure is
        // unrecoverable for id generation.
        getrandom::fill(buf).expect("secure random source unavailable");
    }
}

/// Deterministic generator (mulberry32), one byte per draw (`next() & 0xff`). The golden generator installs
/// the identical function as `crypto.getRandomValues`.
#[derive(Debug, Clone)]
pub struct Mulberry32(u32);

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Mulberry32(seed)
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        t ^ (t >> 14)
    }
}

impl RandomSource for Mulberry32 {
    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = (self.next_u32() & 0xff) as u8;
        }
    }
}

/// `newId()`.
pub fn new_id(rng: &mut dyn RandomSource) -> String {
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    bytes.iter().map(|b| BASE62[(*b % 62) as usize] as char).collect()
}

/// `keyBetween(a, b)` (jittered: one of six evenly spaced candidates).
pub fn key_between(rng: &mut dyn RandomSource, a: Option<&str>, b: Option<&str>) -> Result<String, FractionalIndexError> {
    let keys = generate_n_keys_between(a, b, 6)?;
    let mut pick = [0u8; 1];
    rng.fill(&mut pick);
    Ok(keys[pick[0] as usize % keys.len()].clone())
}
