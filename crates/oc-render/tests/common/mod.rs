// SPDX-License-Identifier: Apache-2.0
//! Shared test helpers: repo paths, golden loading, tolerant JSON comparison, fixtures.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const UNDEF: &str = "__undefined__";

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

pub fn golden(name: &str) -> Value {
    let p = root().join("tests/golden/render").join(format!("{name}.json"));
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e} (run `pnpm tsx tools/golden/render.ts`)", p.display()));
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
    match v {
        Value::String(s) if s.starts_with("__num__") => match &s[7..] {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            x => panic!("bad number marker {x}"),
        },
        _ => v.as_f64().unwrap_or_else(|| panic!("not a number: {v}")),
    }
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))
}

pub fn is_undef(v: &Value) -> bool {
    v.is_null() || v.as_str() == Some(UNDEF)
}

/// A number the way the generator wrote it (non-finite values as `__num__` markers).
pub fn num(x: f64) -> Value {
    if x.is_finite() { j(&x) } else { Value::String(format!("__num__{}", oc_schema::js::num_to_string(x))) }
}

pub fn truncate(s: &str) -> String {
    if s.len() > 600 { format!("{}…", &s[..s.char_indices().nth(600).map_or(s.len(), |(i, _)| i)]) } else { s.to_string() }
}

/// Structural comparison with floats within 1e-9 (relative for large magnitudes); strings exact, except SVG path
/// data (see [`path_data_eq`]).
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
        (Value::String(a), Value::String(b)) if a != b && path_data_eq(a, b) => None,
        _ => {
            if actual == expected {
                None
            } else {
                Some(format!("{path}: {} != {}", truncate(&actual.to_string()), truncate(&expected.to_string())))
            }
        }
    }
}

/// SVG path data that differs only in its numbers, each within 1e-6. tools/golden/render.ts writes path strings with
/// coordinates rounded to 1e-6 (`canonPath`, so the goldens do not depend on the Node/V8 version's last-digit
/// `Math` results); the Rust port prints them unrounded.
pub fn path_data_eq(a: &str, b: &str) -> bool {
    fn is_path(s: &str) -> bool {
        s.trim_start().starts_with(['M', 'm']) && s.chars().all(|c| c.is_ascii_digit() || c.is_ascii_whitespace() || "MmLlHhVvCcSsQqTtAaZz.eE+-,".contains(c))
    }
    /// Splits into text and number tokens (a number: optional `-`, digits with an optional `.`, optional exponent).
    fn tokens(s: &str) -> Vec<Result<f64, &str>> {
        let b = s.as_bytes();
        let (mut out, mut i, mut text) = (Vec::new(), 0, 0);
        while i < b.len() {
            let neg = b[i] == b'-';
            let j = i + usize::from(neg);
            let starts = j < b.len() && (b[j].is_ascii_digit() || (b[j] == b'.' && b.get(j + 1).is_some_and(u8::is_ascii_digit)));
            if !starts {
                i += 1;
                continue;
            }
            let mut k = j;
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            if k < b.len() && b[k] == b'.' {
                k += 1;
                while k < b.len() && b[k].is_ascii_digit() {
                    k += 1;
                }
            }
            if k < b.len() && (b[k] == b'e' || b[k] == b'E') {
                let mut e = k + 1;
                if e < b.len() && (b[e] == b'+' || b[e] == b'-') {
                    e += 1;
                }
                if e < b.len() && b[e].is_ascii_digit() {
                    while e < b.len() && b[e].is_ascii_digit() {
                        e += 1;
                    }
                    k = e;
                }
            }
            out.push(Err(&s[text..i]));
            out.push(Ok(s[i..k].parse().unwrap_or(f64::NAN)));
            (i, text) = (k, k);
        }
        out.push(Err(&s[text..]));
        out
    }
    if !is_path(a) || !is_path(b) {
        return false;
    }
    let (ta, tb) = (tokens(a), tokens(b));
    ta.len() == tb.len()
        && ta.iter().zip(&tb).all(|(x, y)| match (x, y) {
            (Ok(x), Ok(y)) => (x - y).abs() <= 1e-6 + 1e-9 * x.abs().max(y.abs()),
            (Err(x), Err(y)) => x == y,
            _ => false,
        })
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

/// Shallow merge of `patch`'s keys into the JSON object `target` (`{ ...target, ...patch }`).
pub fn merge(target: &mut Value, patch: &Value) {
    if let (Some(t), Some(p)) = (target.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            t.insert(k.clone(), v.clone());
        }
    }
}

// ------------------------------------------------------------------ rendering helpers

pub fn fonts_dir() -> PathBuf {
    root().join("fixtures/fonts")
}

/// A renderer over the pinned font manifest (faces load on demand from fixtures/fonts/files).
pub fn renderer() -> oc_render::Renderer {
    let manifest: oc_text::FontManifest = serde_json::from_slice(&std::fs::read(fonts_dir().join("manifest.json")).expect("font manifest")).expect("manifest json");
    let mut r = oc_render::Renderer::new(oc_text::TextEngine::new(oc_text::FontRegistry::new(manifest)));
    let inter = oc_text::FaceRequest::new("Inter", 400.0, false);
    for file in r.text.fonts.missing([&inter]) {
        load_face(&mut r, &file);
    }
    r
}

pub fn load_face(r: &mut oc_render::Renderer, file: &str) {
    let p = fonts_dir().join("files").join(file);
    if let Ok(bytes) = std::fs::read(&p) {
        let _ = r.text.fonts.insert_face(file, bytes);
    }
}

/// Load every font a display list needs.
pub fn ensure_fonts(r: &mut oc_render::Renderer, dl: &oc_render::DisplayList) {
    for f in r.missing_fonts(dl) {
        load_face(r, &f);
    }
}

/// The committed test assets of the hand-made scenes (crates/oc-render/tests/assets), keyed like the scenes use them.
pub fn scene_assets() -> std::collections::HashMap<String, Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets");
    let mut m = std::collections::HashMap::new();
    for (k, f) in [("photo", "photo.jpg"), ("tile", "tile.png"), ("anim", "anim.gif"), ("pic", "pic.webp"), ("icon", "icon.svg"), ("clip/poster", "poster.png")] {
        m.insert(k.to_owned(), std::fs::read(dir.join(f)).expect("test asset"));
    }
    m
}
