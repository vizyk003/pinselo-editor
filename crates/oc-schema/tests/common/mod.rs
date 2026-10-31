// SPDX-License-Identifier: Apache-2.0
//! Shared golden-test helpers: loading `tests/golden/schema/*.json`, tolerant JSON comparison, case replay.
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
    let p = root().join("tests/golden/schema").join(format!("{name}.json"));
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e} (run `pnpm tsx tools/golden/schema.ts`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

pub fn fixture(file: &str) -> Value {
    let p = root().join("fixtures/templates-seed").join(file);
    serde_json::from_str(&std::fs::read_to_string(p).expect("fixture")).expect("fixture json")
}

pub fn fixture_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root().join("fixtures/templates-seed"))
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".json"))
        .collect();
    v.sort();
    v
}

/// JS-number-normalised JSON of a Rust value.
pub fn j<T: Serialize + ?Sized>(t: &T) -> Value {
    oc_schema::js::to_value(t).expect("serialise")
}

pub fn de<T: DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("deserialize {}: {e}", truncate(&v.to_string())))
}

pub fn opt<T: DeserializeOwned>(v: &Value) -> Option<T> {
    if v.is_null() || v.as_str() == Some(UNDEF) { None } else { Some(de(v)) }
}

pub fn f(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().expect("f64"),
        Value::String(s) if s.starts_with("__num__") => match &s[7..] {
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => f64::NAN,
        },
        _ => panic!("not a number: {v}"),
    }
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

pub fn num(x: f64) -> Value {
    if x.is_finite() {
        j(&x)
    } else {
        Value::String(format!("__num__{}", if x.is_nan() { "NaN" } else if x > 0.0 { "Infinity" } else { "-Infinity" }))
    }
}

pub fn undef() -> Value {
    Value::String(UNDEF.into())
}

pub fn truncate(s: &str) -> String {
    if s.len() > 600 { format!("{}…", &s[..s.char_indices().nth(600).map_or(s.len(), |(i, _)| i)]) } else { s.to_string() }
}

/// Structural comparison with floats within 1e-9 (relative for large magnitudes). Non-finite markers from the
/// TS side (`"__num__NaN"`) match `null` (serde_json writes non-finite floats as `null`).
pub fn diff(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            let tol = 1e-9 * 1f64.max(a.abs().max(b.abs()));
            if (a - b).abs() <= tol { None } else { Some(format!("{path}: {a} != {b}")) }
        }
        (Value::Null, Value::String(s)) | (Value::String(s), Value::Null) if s.starts_with("__num__") => None,
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
                return Some(format!("{path}: length {} != {} (actual {}, expected {})", a.len(), b.len(), truncate(&Value::Array(a.clone()).to_string()), truncate(&Value::Array(b.clone()).to_string())));
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

/// Outcome of replaying one case.
pub enum Out {
    Val(Value),
    Err(String),
    /// The TS throws a JS runtime error (TypeError…) here; Rust must only not panic.
    Skip,
}

impl From<Value> for Out {
    fn from(v: Value) -> Self {
        Out::Val(v)
    }
}

pub fn ok<T: Serialize>(t: T) -> Out {
    Out::Val(j(&t))
}

pub fn res<T: Serialize, E: std::fmt::Display>(r: Result<T, E>) -> Out {
    match r {
        Ok(t) => Out::Val(j(&t)),
        Err(e) => Out::Err(e.to_string()),
    }
}

pub fn opt_out<T: Serialize>(o: Option<T>, none: Value) -> Out {
    match o {
        Some(t) => Out::Val(j(&t)),
        None => Out::Val(none),
    }
}

/// Replays every `{ fn, args, out | error }` case; panics listing the mismatches. Returns the number of cases.
pub fn replay(file: &str, cases: &Value, mut run: impl FnMut(&str, &[Value]) -> Out) -> usize {
    let cases = cases.as_array().expect("cases array");
    let mut failures: Vec<String> = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let name = s(&c["fn"]);
        let args = c["args"].as_array().expect("args");
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(name, args)));
        let label = || format!("{file}#{i} {name}({})", truncate(&Value::Array(args.clone()).to_string()));
        match got {
            Err(_) => failures.push(format!("{}: panicked", label())),
            Ok(Out::Skip) => {}
            Ok(Out::Val(v)) => match (c.get("out"), c.get("error")) {
                (Some(exp), _) => {
                    if let Some(d) = diff(&v, exp, "$") {
                        failures.push(format!("{}: {d}", label()));
                    }
                }
                (None, Some(e)) => failures.push(format!("{}: expected error {e}, got {}", label(), truncate(&v.to_string()))),
                _ => failures.push(format!("{}: malformed case", label())),
            },
            Ok(Out::Err(m)) => match c.get("error") {
                Some(Value::String(e)) if *e == m || e.starts_with("Cannot ") => {}
                Some(e) => failures.push(format!("{}: error {m:?} != {e}", label())),
                None => failures.push(format!("{}: unexpected error {m:?} (expected {})", label(), truncate(&c["out"].to_string()))),
            },
        }
    }
    if !failures.is_empty() {
        let n = failures.len();
        failures.truncate(25);
        panic!("{n} of {} cases failed in {file}:\n{}", cases.len(), failures.join("\n"));
    }
    cases.len()
}

/// Replace a top-level `id` with `"<id>"` (factories generate random ids).
pub fn norm_id(mut v: Value) -> Value {
    if let Some(m) = v.as_object_mut()
        && m.get("id").is_some_and(Value::is_string)
    {
        m.insert("id".into(), Value::String("<id>".into()));
    }
    v
}
