// SPDX-License-Identifier: Apache-2.0
//! Whiteboard facilitation state shared through the design's Y.Doc (WB-030..): the countdown timer and vote
//! sessions. It lives in its own root maps outside the element tree, so it syncs in real time to every
//! collaborator, persists with the document, and is not part of undo history.
//!
//! ```text
//! board: Y.Map { timer: TimerState, vote: VoteSession }
//! votes: Y.Map<"{session}|{voter}|{element}", VoteCell>
//! ```
//!
//! Each voter only writes keys that carry their own id, so concurrent votes from different people never
//! conflict: merging two replicas keeps every voter's dots.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yrs::{Doc, Map as _, MapRef, Out, Transact};

use crate::origins::BOARD_ORIGIN;
use crate::value::{json_to_any, out_to_json};
use oc_schema::Extra;

pub const BOARD: &str = "board";
pub const VOTES: &str = "votes";

/// The countdown timer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerState {
    /// Configured length (reset target).
    pub duration_ms: f64,
    /// Running: wall-clock end time (ms since epoch).
    pub ends_at: Option<f64>,
    /// Paused / idle: time left.
    pub remaining_ms: f64,
    /// Play a chime for everyone when it reaches zero.
    pub sound: bool,
    /// Who last changed it (display only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Who started a vote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Starter {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A dot-voting session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteSession {
    pub id: String,
    /// Dots each person may place.
    pub per_person: f64,
    pub anonymous: bool,
    /// Elements that can receive votes.
    pub scope: Vec<String>,
    /// `'open' | 'ended'`.
    pub status: String,
    pub started_by: Starter,
    pub started_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<f64>,
    /// "Highlight top 3": elements marked for everyone after the vote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One voter's dots on one element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoteEntry {
    pub session: String,
    pub voter: String,
    pub element: String,
    pub n: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A voter (`{ id, name? }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voter {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn board_map(ydoc: &Doc) -> MapRef {
    ydoc.get_or_insert_map(BOARD)
}

fn votes_map(ydoc: &Doc) -> MapRef {
    ydoc.get_or_insert_map(VOTES)
}

fn read_json(ydoc: &Doc, map: &MapRef, key: &str) -> Option<Value> {
    let txn = ydoc.transact();
    map.get(&txn, key).map(|v| out_to_json(&txn, &v)).filter(|v| !v.is_null())
}

fn write_json(ydoc: &Doc, map: &MapRef, key: &str, v: Option<Value>) {
    let mut txn = ydoc.transact_mut_with(BOARD_ORIGIN.origin());
    match v {
        Some(v) => {
            map.insert(&mut txn, key, json_to_any(&v));
        }
        None => {
            map.remove(&mut txn, key);
        }
    }
}

/// The timer as JSON (`null` → `None`).
pub fn read_timer_value(ydoc: &Doc) -> Option<Value> {
    read_json(ydoc, &board_map(ydoc), "timer")
}

pub fn read_timer(ydoc: &Doc) -> Option<TimerState> {
    read_timer_value(ydoc).and_then(|v| serde_json::from_value(v).ok())
}

pub fn write_timer(ydoc: &Doc, timer: Option<&TimerState>) {
    write_json(ydoc, &board_map(ydoc), "timer", timer.and_then(|t| oc_schema::js::to_value(t).ok()));
}

pub fn read_vote_session_value(ydoc: &Doc) -> Option<Value> {
    read_json(ydoc, &board_map(ydoc), "vote")
}

pub fn read_vote_session(ydoc: &Doc) -> Option<VoteSession> {
    read_vote_session_value(ydoc).and_then(|v| serde_json::from_value(v).ok())
}

pub fn write_vote_session(ydoc: &Doc, session: Option<&VoteSession>) {
    write_json(ydoc, &board_map(ydoc), "vote", session.and_then(|s| oc_schema::js::to_value(s).ok()));
}

fn vote_key(session: &str, voter: &str, element: &str) -> String {
    format!("{session}|{voter}|{element}")
}

fn cell_n(v: &Value) -> Option<f64> {
    v.get("n").and_then(Value::as_f64)
}

/// All dots of a session (every voter, every element), sorted by key (yrs does not keep insertion order).
pub fn read_votes(ydoc: &Doc, session: &str) -> Vec<VoteEntry> {
    let m = votes_map(ydoc);
    let txn = ydoc.transact();
    let mut entries: Vec<(String, Out)> = m.iter(&txn).map(|(k, v)| (k.to_string(), v)).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (k, v) in entries {
        let v = out_to_json(&txn, &v);
        let mut parts = k.split('|');
        let (Some(s), Some(voter), Some(element)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some(n) = cell_n(&v) else { continue };
        if s != session || voter.is_empty() || element.is_empty() || n.is_nan() || n <= 0.0 {
            continue;
        }
        let name = v.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        out.push(VoteEntry { session: s.into(), voter: voter.into(), element: element.into(), n, name });
    }
    out
}

/// Add (`delta` > 0) or remove (< 0) a voter's dots on one element. Returns the voter's new count there.
/// The per-person budget is enforced by the caller (see `voteAllowance` in editor-core).
pub fn cast_vote(ydoc: &Doc, session: &str, voter: &Voter, element: &str, delta: f64) -> f64 {
    let m = votes_map(ydoc);
    let key = vote_key(session, &voter.id, element);
    let cur = read_json(ydoc, &m, &key).as_ref().and_then(cell_n).unwrap_or(0.0);
    let n = (cur + delta).max(0.0);
    let mut txn = ydoc.transact_mut_with(BOARD_ORIGIN.origin());
    if n == 0.0 {
        m.remove(&mut txn, &key);
    } else {
        let mut cell = Map::new();
        cell.insert("n".into(), crate::value::f64_to_json(n));
        if let Some(name) = voter.name.as_ref().filter(|s| !s.is_empty()) {
            cell.insert("name".into(), Value::String(name.clone()));
        }
        m.insert(&mut txn, key, json_to_any(&Value::Object(cell)));
    }
    n
}

/// Drop every dot of a session (or of all sessions) after the results are no longer needed.
pub fn clear_votes(ydoc: &Doc, session: Option<&str>) {
    let m = votes_map(ydoc);
    let mut txn = ydoc.transact_mut_with(BOARD_ORIGIN.origin());
    let keys: Vec<String> = m.keys(&txn).map(str::to_string).collect();
    for k in keys {
        if session.is_none_or(|s| k.starts_with(&format!("{s}|"))) {
            m.remove(&mut txn, &k);
        }
    }
}

/// Subscribe to timer / vote changes (local and remote) under `key`; [`unobserve_board`] removes it.
pub fn observe_board<F: Fn() + Clone + Send + Sync + 'static>(ydoc: &Doc, key: &str, f: F) {
    use yrs::Observable;
    let g = f.clone();
    board_map(ydoc).observe(key, move |_, _| f());
    votes_map(ydoc).observe(key, move |_, _| g());
}

/// Remove a subscription made with [`observe_board`].
pub fn unobserve_board(ydoc: &Doc, key: &str) {
    use yrs::Observable;
    board_map(ydoc).unobserve(key);
    votes_map(ydoc).unobserve(key);
}
