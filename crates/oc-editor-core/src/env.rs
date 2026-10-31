// SPDX-License-Identifier: Apache-2.0
//! Injected environment: the clock and the random source. Library code never reads the time or the OS RNG
//! directly, so the editor runs unchanged in WASM and in deterministic tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use oc_crdt::rng::{Mulberry32, OsRandom, RandomSource};

/// Wall-clock milliseconds. Used where TS reads `Date.now()` (tombstones, the undo capture window) and
/// `performance.now()` (playback anchors, animation preview) — one clock for both.
pub trait Clock: Send + Sync {
    fn now(&self) -> f64;
}

impl<F: Fn() -> f64 + Send + Sync> Clock for F {
    fn now(&self) -> f64 {
        self()
    }
}

/// A clock the host sets explicitly (tests, golden replay, or a WASM host that pushes `performance.now()`).
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(ms: f64) -> Arc<Self> {
        Arc::new(ManualClock(AtomicU64::new(ms.to_bits())))
    }

    pub fn set(&self, ms: f64) {
        self.0.store(ms.to_bits(), Ordering::SeqCst);
    }

    pub fn advance(&self, ms: f64) {
        self.set(self.now() + ms);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::SeqCst))
    }
}

/// The yrs undo-manager clock view of a [`Clock`] (integer ms, never 0 so the capture window works).
pub(crate) fn undo_clock(c: Arc<dyn Clock>) -> Arc<dyn yrs::sync::Clock> {
    Arc::new(move || {
        let n = c.now();
        if n.is_finite() && n >= 1.0 { n as u64 } else { 1 }
    })
}
