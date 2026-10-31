//! Server-side awareness (port of y-protocols/awareness as used by a Hocuspocus `Document`): the server
//! holds no local state and relays remote states. States are kept as `JSON.stringify(JSON.parse(raw))`
//! text, so re-encoding reproduces exactly what yjs would send.

use std::collections::HashMap;

use indexmap::IndexMap;
use oc_js::JsValue;

use crate::protocol::{DecodeError, Decoder, Encoder};

/// `outdatedTimeout` of y-protocols: remote states not renewed within 30 s are removed.
pub const OUTDATED_TIMEOUT_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy)]
struct Meta {
    clock: u64,
    last_updated: i64,
}

/// Added / updated / removed client ids of one awareness change (y-protocols `'update'` event payload).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AwarenessChange {
    pub added: Vec<u64>,
    pub updated: Vec<u64>,
    pub removed: Vec<u64>,
}

impl AwarenessChange {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
    /// `added.concat(updated, removed)`.
    pub fn changed(&self) -> Vec<u64> {
        self.added.iter().chain(&self.updated).chain(&self.removed).copied().collect()
    }
}

#[derive(Debug, Default)]
pub struct Awareness {
    /// Insertion-ordered like the JS `Map` (`Array.from(states.keys())`).
    states: IndexMap<u64, String>,
    meta: HashMap<u64, Meta>,
}

#[derive(Debug)]
pub enum AwarenessError {
    Decode(DecodeError),
    Json(String),
}

impl std::fmt::Display for AwarenessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AwarenessError::Decode(e) => write!(f, "{e}"),
            AwarenessError::Json(e) => write!(f, "{e}"),
        }
    }
}

impl From<DecodeError> for AwarenessError {
    fn from(e: DecodeError) -> Self {
        AwarenessError::Decode(e)
    }
}

impl Awareness {
    pub fn has_states(&self) -> bool {
        !self.states.is_empty()
    }

    pub fn client_ids(&self) -> Vec<u64> {
        self.states.keys().copied().collect()
    }

    pub fn state(&self, client: u64) -> Option<&str> {
        self.states.get(&client).map(String::as_str)
    }

    /// `encodeAwarenessUpdate(awareness, clients)`.
    pub fn encode(&self, clients: &[u64]) -> Vec<u8> {
        let mut e = Encoder::new();
        e.var_uint(clients.len() as u64);
        for c in clients {
            let clock = self.meta.get(c).map_or(0, |m| m.clock);
            // `states.get(clientID) || null`: falsy states (0, false, "") go out as null too.
            let state = self.states.get(c).map(String::as_str).filter(|s| !matches!(*s, "0" | "false" | "\"\"")).unwrap_or("null");
            e.var_uint(*c).var_uint(clock).var_string(state);
        }
        e.finish()
    }

    /// `applyAwarenessUpdate(awareness, update, origin)`; `now` is ms since the epoch. Like the JS code,
    /// entries before a malformed one stay applied (and no event is reported) when decoding fails.
    pub fn apply(&mut self, update: &[u8], now: i64) -> Result<AwarenessChange, AwarenessError> {
        let mut d = Decoder::new(update);
        let mut ch = AwarenessChange::default();
        let len = d.var_uint()?;
        for _ in 0..len {
            let client = d.var_uint()?;
            let clock = d.var_uint()?;
            let raw = d.var_string()?;
            let parsed = JsValue::parse(&raw).map_err(|e| AwarenessError::Json(e.to_string()))?;
            let state = match parsed {
                JsValue::Null => None,
                v => Some(v.stringify()),
            };
            let meta = self.meta.get(&client).copied();
            let curr = meta.map_or(0, |m| m.clock);
            if curr < clock || (curr == clock && state.is_none() && self.states.contains_key(&client)) {
                let is_null = state.is_none();
                match state {
                    None => {
                        self.states.shift_remove(&client);
                    }
                    Some(s) => {
                        self.states.insert(client, s);
                    }
                }
                self.meta.insert(client, Meta { clock, last_updated: now });
                if meta.is_none() && !is_null {
                    ch.added.push(client);
                } else if meta.is_some() && is_null {
                    ch.removed.push(client);
                } else if !is_null {
                    ch.updated.push(client);
                }
            }
        }
        Ok(ch)
    }

    /// `removeAwarenessStates(awareness, clients, origin)`: drop the states (clocks are kept).
    pub fn remove(&mut self, clients: &[u64]) -> AwarenessChange {
        let mut ch = AwarenessChange::default();
        for c in clients {
            if self.states.shift_remove(c).is_some() {
                ch.removed.push(*c);
            }
        }
        ch
    }

    /// Remote states not renewed for [`OUTDATED_TIMEOUT_MS`] (the y-protocols check interval).
    pub fn outdated(&self, now: i64) -> Vec<u64> {
        self.meta
            .iter()
            .filter(|(c, m)| OUTDATED_TIMEOUT_MS <= now - m.last_updated && self.states.contains_key(*c))
            .map(|(c, _)| *c)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upd(entries: &[(u64, u64, &str)]) -> Vec<u8> {
        let mut e = Encoder::new();
        e.var_uint(entries.len() as u64);
        for (c, k, s) in entries {
            e.var_uint(*c).var_uint(*k).var_string(s);
        }
        e.finish()
    }

    #[test]
    fn add_update_remove() {
        let mut a = Awareness::default();
        let ch = a.apply(&upd(&[(7, 1, r#"{"user": {"name":"A"}}"#)]), 0).unwrap();
        assert_eq!(ch.added, vec![7]);
        assert_eq!(a.state(7), Some(r#"{"user":{"name":"A"}}"#));
        // Older clock: ignored.
        assert!(a.apply(&upd(&[(7, 0, "{}")]), 0).unwrap().is_empty());
        // Same clock renewal: ignored unless null.
        assert!(a.apply(&upd(&[(7, 1, "{}")]), 0).unwrap().is_empty());
        assert_eq!(a.apply(&upd(&[(7, 2, "{}")]), 0).unwrap().updated, vec![7]);
        assert_eq!(a.apply(&upd(&[(7, 2, "null")]), 0).unwrap().removed, vec![7]);
        assert!(!a.has_states());
        // Encoding a removed client: its last clock and null.
        assert_eq!(a.encode(&[7]), upd(&[(7, 2, "null")]));
    }

    #[test]
    fn timeout_and_remove() {
        let mut a = Awareness::default();
        a.apply(&upd(&[(1, 1, "{}"), (2, 1, "{}")]), 0).unwrap();
        a.apply(&upd(&[(2, 2, "{}")]), 20_000).unwrap();
        assert_eq!(a.outdated(30_000), vec![1]);
        assert_eq!(a.remove(&[1, 9]).removed, vec![1]);
        assert_eq!(a.client_ids(), vec![2]);
        assert!(a.apply(&[1, 1], 0).is_err());
    }

    #[test]
    fn falsy_states_encode_as_null() {
        let mut a = Awareness::default();
        a.apply(&upd(&[(1, 1, "0"), (2, 1, "false"), (3, 1, "\"\""), (4, 1, "-0"), (5, 1, "1")]), 0).unwrap();
        assert_eq!(a.client_ids(), vec![1, 2, 3, 4, 5]);
        assert_eq!(a.encode(&[1, 2, 3, 4, 5]), upd(&[(1, 1, "null"), (2, 1, "null"), (3, 1, "null"), (4, 1, "null"), (5, 1, "1")]));
    }
}
