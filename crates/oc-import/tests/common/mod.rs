// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for the golden tests: load goldens and fixtures, normalise generated ids, compare JSON.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn golden(kind: &str, name: &str) -> Value {
    let p = root().join("tests/golden/import").join(kind).join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("golden json")
}

/// Golden case names of a kind (file stems, sorted).
pub fn cases(kind: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root().join("tests/golden/import").join(kind))
        .expect("golden dir")
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
        .collect();
    v.sort();
    v
}

pub fn fixture(file: &str) -> Vec<u8> {
    std::fs::read(root().join("fixtures/import").join(file)).expect("fixture")
}

pub fn families() -> Vec<String> {
    serde_json::from_str(&std::fs::read_to_string(root().join("tests/golden/import/families.json")).expect("families")).expect("json")
}

/// Replace generated ids: pages → `P<order>`, elements → `E<page order>/<z>` (z keys are deterministic and unique
/// per parent), everywhere they occur (keys and string values).
pub fn normalize_doc(doc: &Value) -> Value {
    let mut ids: HashMap<String, String> = HashMap::new();
    if let Some(pages) = doc["pages"].as_object() {
        for (id, p) in pages {
            ids.insert(id.clone(), format!("P{}", p["order"].as_str().unwrap_or("?")));
        }
    }
    if let Some(els) = doc["elements"].as_object() {
        for (id, e) in els {
            let parent = e["parentId"].as_str().unwrap_or("");
            let pk = ids.get(parent).cloned().unwrap_or_else(|| parent.to_owned());
            ids.insert(id.clone(), format!("E{pk}/{}", e["z"].as_str().unwrap_or("?")));
        }
    }
    fn walk(v: &Value, ids: &HashMap<String, String>) -> Value {
        match v {
            Value::String(s) => Value::String(ids.get(s).cloned().unwrap_or_else(|| s.clone())),
            Value::Array(a) => Value::Array(a.iter().map(|x| walk(x, ids)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (ids.get(k).cloned().unwrap_or_else(|| k.clone()), walk(x, ids))).collect()),
            other => other.clone(),
        }
    }
    walk(doc, &ids)
}

/// Structural comparison (object key order ignored), numbers within `tol(path)` (default 1e-6, relative above 1).
pub fn diff(want: &Value, got: &Value, path: &str, tol: &dyn Fn(&str) -> f64, out: &mut Vec<String>) {
    if out.len() > 40 {
        return;
    }
    match (want, got) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
            let t = tol(path) * a.abs().max(1.0);
            if (a - b).abs() > t {
                out.push(format!("{path}: want {a}, got {b}"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for (k, va) in a {
                match b.get(k) {
                    Some(vb) => diff(va, vb, &format!("{path}.{k}"), tol, out),
                    None => out.push(format!("{path}.{k}: missing (want {})", short(va))),
                }
            }
            for (k, vb) in b {
                if !a.contains_key(k) {
                    out.push(format!("{path}.{k}: unexpected {}", short(vb)));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!("{path}: want {} items, got {}\n  want {}\n  got  {}", a.len(), b.len(), short(want), short(got)));
                return;
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                diff(x, y, &format!("{path}[{i}]"), tol, out);
            }
        }
        _ => {
            if want != got {
                out.push(format!("{path}: want {}, got {}", short(want), short(got)));
            }
        }
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 400 { format!("{}…", &s[..s.char_indices().nth(400).map_or(s.len(), |(i, _)| i)]) } else { s }
}
