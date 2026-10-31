// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for the golden / cross-language tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

use base64::Engine;
use oc_crdt::value::canonical;
use serde_json::{Value, json};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

pub fn golden_dir() -> PathBuf {
    root().join("tests/golden/crdt")
}

pub fn load(name: &str) -> Value {
    let p = golden_dir().join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e} (run `pnpm tsx tools/golden/crdt.ts`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

pub fn unb64(s: &Value) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s.as_str().expect("base64 string")).expect("base64")
}

pub fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Order-insensitive equality with a readable failure message.
#[track_caller]
pub fn assert_same(actual: &Value, expected: &Value, what: &str) {
    let a = canonical(actual);
    let e = canonical(expected);
    if a != e {
        let a_s = serde_json::to_string_pretty(&a).unwrap();
        let e_s = serde_json::to_string_pretty(&e).unwrap();
        let line = a_s.lines().zip(e_s.lines()).position(|(x, y)| x != y).unwrap_or(0);
        let ctx = |s: &str| s.lines().skip(line.saturating_sub(8)).take(20).collect::<Vec<_>>().join("\n");
        panic!("{what}: mismatch near line {line}\n--- actual ---\n{}\n--- expected ---\n{}", ctx(&a_s), ctx(&e_s));
    }
}

/// Decode updates with **yjs** (the TS verify mode) and return what TS reads back per case:
/// `{ design, board: { timer, vote, votes }, docBody, plain }`. `None` when the TS side is unavailable (TS package
/// deleted after cutover, no pnpm, or `OC_SKIP_TS_VERIFY` set) — the committed goldens still cover the TS → Rust
/// direction in that case.
pub fn ts_verify(cases: Vec<(Vec<Vec<u8>>, Vec<String>)>) -> Option<Vec<Value>> {
    if std::env::var_os("OC_SKIP_TS_VERIFY").is_some() {
        eprintln!("OC_SKIP_TS_VERIFY set: skipping the yjs side of the cross-check");
        return None;
    }
    let root = root();
    if !root.join("tools/golden/crdt.ts").exists() || !root.join("packages/crdt/src/index.ts").exists() {
        eprintln!("TS @oc/crdt not present: skipping the yjs side of the cross-check");
        return None;
    }
    let input = json!({
        "cases": cases.iter().map(|(u, texts)| json!({ "updates": u.iter().map(|b| b64(b)).collect::<Vec<_>>(), "texts": texts })).collect::<Vec<_>>()
    });
    let dir = std::env::temp_dir();
    let path = dir.join(format!("oc-crdt-verify-{}-{}.json", std::process::id(), rand_suffix()));
    std::fs::write(&path, serde_json::to_vec(&input).unwrap()).expect("write verify input");
    let out_path = path.with_extension("out.json");
    let out = Command::new("pnpm")
        .args(["tsx", "tools/golden/crdt.ts", "--verify"])
        .arg(&path)
        .arg("--out")
        .arg(&out_path)
        .current_dir(&root)
        .output();
    let _ = std::fs::remove_file(&path);
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cannot run pnpm ({e}): skipping the yjs side of the cross-check");
            return None;
        }
    };
    assert!(out.status.success(), "TS verify failed:\n{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&out_path).expect("verify output file");
    let _ = std::fs::remove_file(&out_path);
    let v: Value = serde_json::from_slice(&bytes).expect("verify output json");
    Some(v["results"].as_array().expect("results").clone())
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u8(0);
    h.finish()
}
