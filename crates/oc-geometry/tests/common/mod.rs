// SPDX-License-Identifier: Apache-2.0
//! Shared golden-test helpers: loading `tests/golden/geometry/*.json`, tolerant JSON comparison, case replay.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const UNDEF: &str = "__undefined__";

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn golden(name: &str) -> Value {
    let p = root().join("tests/golden/geometry").join(format!("{name}.json"));
    let s = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e} (run `pnpm tsx tools/golden/geometry.ts`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

pub fn fixture(file: &str) -> Value {
    let p = root().join("fixtures/templates-seed").join(file);
    serde_json::from_str(&std::fs::read_to_string(p).expect("fixture")).expect("fixture json")
}

/// JS-number-normalised JSON of a Rust value.
pub fn j<T: Serialize + ?Sized>(t: &T) -> Value {
    oc_schema::js::to_value(t).expect("serialise")
}

pub fn de<T: DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("deserialize {}: {e}", truncate(&v.to_string())))
}

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

pub fn is_undef(v: &Value) -> bool {
    v.is_null() || v.as_str() == Some(UNDEF)
}

pub fn truncate(s: &str) -> String {
    if s.len() > 600 { format!("{}…", &s[..s.char_indices().nth(600).map_or(s.len(), |(i, _)| i)]) } else { s.to_string() }
}

/// Structural comparison with floats within 1e-9 (relative for large magnitudes); strings exact.
pub fn diff(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            let tol = 1e-9 * 1f64.max(a.abs().max(b.abs()));
            if (a - b).abs() <= tol { None } else { Some(format!("{path}: {a} != {b}")) }
        }
        (Value::Object(a), Value::Object(b)) => {
            for k in b.keys() {
                if !a.contains_key(k) {
                    return Some(format!("{path}.{k}: missing in actual (expected {})", truncate(&b[k].to_string())));
                }
            }
            for k in a.keys() {
                if !b.contains_key(k) {
                    return Some(format!("{path}.{k}: unexpected in actual ({})", truncate(&a[k].to_string())));
                }
            }
            a.iter().find_map(|(k, v)| diff(v, &b[k], &format!("{path}.{k}")))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!(
                    "{path}: length {} != {} (actual {}, expected {})",
                    a.len(),
                    b.len(),
                    truncate(&Value::Array(a.clone()).to_string()),
                    truncate(&Value::Array(b.clone()).to_string())
                ));
            }
            a.iter().zip(b).enumerate().find_map(|(i, (x, y))| diff(x, y, &format!("{path}[{i}]")))
        }
        _ => {
            if actual == expected {
                None
            } else {
                Some(format!("{path}: {} != {}", truncate(&actual.to_string()), truncate(&expected.to_string())))
            }
        }
    }
}

/// Collects mismatches and panics with the first few at the end.
#[derive(Default)]
pub struct Failures(pub Vec<String>, pub usize);

impl Failures {
    pub fn check(&mut self, label: impl FnOnce() -> String, actual: &Value, expected: &Value) {
        self.1 += 1;
        if let Some(d) = diff(actual, expected, "$") {
            self.0.push(format!("{}: {d}", label()));
        }
    }
    pub fn finish(mut self, what: &str) -> usize {
        if !self.0.is_empty() {
            let n = self.0.len();
            self.0.truncate(25);
            panic!("{n} of {} checks failed in {what}:\n{}", self.1, self.0.join("\n"));
        }
        self.1
    }
}

/// Replays every `{ fn, args, out }` case; panics listing the mismatches. Returns the number of cases.
pub fn replay(file: &str, run: impl Fn(&str, &[Value]) -> Value) -> usize {
    let cases = golden(file);
    let cases = cases.as_array().expect("cases array");
    let mut fails = Failures::default();
    for (i, c) in cases.iter().enumerate() {
        let name = s(&c["fn"]);
        let args = c["args"].as_array().expect("args");
        let label = || format!("{file}#{i} {name}({})", truncate(&Value::Array(args.clone()).to_string()));
        let exp = c.get("out").unwrap_or_else(|| panic!("{}: TS threw {}", label(), c["error"]));
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(name, args))) {
            Ok(v) => fails.check(label, &v, exp),
            Err(_) => {
                fails.1 += 1;
                fails.0.push(format!("{}: panicked", label()));
            }
        }
    }
    let n = fails.finish(file);
    eprintln!("{file}: {n} golden cases ok");
    n
}
