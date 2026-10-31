// SPDX-License-Identifier: Apache-2.0
//! Shared golden-test helpers: loading `tests/golden/render-svc/`, designs, tolerant JSON comparison, XML
//! normalisation for OOXML parts, and the TS cross-check (`tools/golden/render-svc.ts --mammoth`).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use base64::Engine as _;
use quick_xml::events::Event;
use serde_json::Value;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The bundled fonts only, as the golden generators use them (`createNodeRenderer({ bundledOnly: true })`):
/// fixtures/fonts/manifest.json without the Google catalogue, so a Google family is unknown and falls back to
/// Inter on both sides, whatever a machine's database has pinned (ADR-0001: no network, no database).
pub fn bundled_fonts_dir() -> PathBuf {
    root().join("fixtures/fonts")
}

pub fn bundled_manifest() -> oc_gfonts::manifest::FontManifest {
    serde_json::from_str(
        &std::fs::read_to_string(bundled_fonts_dir().join("manifest.json")).unwrap(),
    )
    .unwrap()
}

/// A bundled font file (`@oc/gfonts` `bundledFontFileLoader`): Google faces (`gf/…`) and uploads (`custom/…`)
/// are missing, never fetched or looked up.
pub fn bundled_font_file(file: &str) -> Result<Vec<u8>, String> {
    if file.starts_with("gf/") || file.starts_with("custom/") {
        return Err(format!("font {file} is not bundled"));
    }
    std::fs::read(bundled_fonts_dir().join("files").join(file)).map_err(|e| e.to_string())
}

pub fn golden_path(rel: &str) -> PathBuf {
    root().join("tests/golden/render-svc").join(rel)
}

pub fn golden(rel: &str) -> Value {
    let p = golden_path(rel);
    let s = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e} (run `pnpm golden render-svc`)", p.display()));
    serde_json::from_str(&s).expect("golden json")
}

/// Every golden file in a directory, sorted by name.
pub fn golden_dir(dir: &str) -> Vec<(String, Value)> {
    let p = golden_path(dir);
    let mut names: Vec<String> = std::fs::read_dir(&p)
        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| {
            let v = golden(&format!("{dir}/{n}"));
            (n, v)
        })
        .collect()
}

/// A design by golden name: `fixture:<file>` from fixtures/templates-seed, others from designs.json.
pub fn design_json(name: &str) -> Value {
    if let Some(f) = name.strip_prefix("fixture:") {
        let p = root()
            .join("fixtures/templates-seed")
            .join(format!("{f}.json"));
        return serde_json::from_str(&std::fs::read_to_string(p).expect("fixture"))
            .expect("fixture json");
    }
    static DESIGNS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    DESIGNS
        .get_or_init(|| golden("designs.json"))
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("no design {name} in designs.json"))
}

pub fn design(name: &str) -> oc_schema::DesignDoc {
    serde_json::from_value(design_json(name)).unwrap_or_else(|e| panic!("parse design {name}: {e}"))
}

/// Test asset files (base64 in assets.json) by name.
pub fn asset(name: &str) -> Vec<u8> {
    static ASSETS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let a = ASSETS.get_or_init(|| golden("assets.json"));
    base64::engine::general_purpose::STANDARD
        .decode(
            a[name]
                .as_str()
                .unwrap_or_else(|| panic!("no asset {name}")),
        )
        .expect("b64")
}

pub fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

pub fn unb64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .expect("b64")
}

pub fn truncate(s: &str) -> String {
    if s.len() > 800 {
        format!(
            "{}…",
            &s[..s.char_indices().nth(800).map_or(s.len(), |(i, _)| i)]
        )
    } else {
        s.to_string()
    }
}

/// Structural comparison with floats within 1e-9 (relative for large magnitudes); strings exact.
pub fn diff(actual: &Value, expected: &Value, path: &str) -> Option<String> {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (
                a.as_f64().unwrap_or(f64::NAN),
                b.as_f64().unwrap_or(f64::NAN),
            );
            let tol = 1e-9 * 1f64.max(a.abs().max(b.abs()));
            if (a - b).abs() <= tol {
                None
            } else {
                Some(format!("{path}: {a} != {b}"))
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for k in b.keys() {
                if !a.contains_key(k) {
                    return Some(format!(
                        "{path}.{k}: missing in actual (expected {})",
                        truncate(&b[k].to_string())
                    ));
                }
            }
            for k in a.keys() {
                if !b.contains_key(k) {
                    return Some(format!(
                        "{path}.{k}: unexpected in actual ({})",
                        truncate(&a[k].to_string())
                    ));
                }
            }
            a.iter()
                .find_map(|(k, v)| diff(v, &b[k], &format!("{path}.{k}")))
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
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(i, (x, y))| diff(x, y, &format!("{path}[{i}]")))
        }
        _ => {
            if actual == expected {
                None
            } else {
                Some(format!(
                    "{path}: {} != {}",
                    truncate(&actual.to_string()),
                    truncate(&expected.to_string())
                ))
            }
        }
    }
}

/// First differing position of two strings, with context.
pub fn str_diff(actual: &str, expected: &str) -> Option<String> {
    if actual == expected {
        return None;
    }
    let i = actual
        .char_indices()
        .zip(expected.chars())
        .find(|((_, a), b)| a != b)
        .map_or_else(|| actual.len().min(expected.len()), |((i, _), _)| i);
    let lo = actual[..i]
        .char_indices()
        .rev()
        .nth(120)
        .map_or(0, |(k, _)| k);
    let ctx = |s: &str| {
        let start = lo.min(s.len());
        let start = (0..=start)
            .rev()
            .find(|k| s.is_char_boundary(*k))
            .unwrap_or(0);
        let end = (i + 200).min(s.len());
        let end = (end..=s.len())
            .find(|k| s.is_char_boundary(*k))
            .unwrap_or(s.len());
        s[start..end].to_string()
    };
    Some(format!(
        "differs at byte {i} (lengths {} vs {}):\n  actual:   …{}\n  expected: …{}",
        actual.len(),
        expected.len(),
        ctx(actual),
        ctx(expected)
    ))
}

// ------------------------------------------------------------------ XML normalisation

/// A parsed XML node.
#[derive(Debug, Clone, PartialEq)]
pub enum XNode {
    El {
        name: String,
        attrs: BTreeMap<String, String>,
        kids: Vec<XNode>,
    },
    Text(String),
}

#[allow(deprecated)]
pub fn parse_xml(s: &str) -> XNode {
    let mut r = quick_xml::Reader::from_str(s);
    let mut stack: Vec<XNode> = vec![XNode::El {
        name: "#doc".into(),
        attrs: BTreeMap::new(),
        kids: vec![],
    }];
    // quick-xml 0.41 dropped `Attribute::unescape_value`; this is its equivalent (unescape, no value normalisation).
    let decoder = r.decoder();
    let attrs_of = |e: &quick_xml::events::BytesStart| -> BTreeMap<String, String> {
        e.attributes()
            .map(|a| {
                let a = a.expect("attr");
                (
                    String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                    a.decode_and_unescape_value(decoder).expect("attr value").into_owned(),
                )
            })
            .collect()
    };
    let push = |stack: &mut Vec<XNode>, n: XNode| {
        if let Some(XNode::El { kids, .. }) = stack.last_mut() {
            if let (XNode::Text(t), Some(XNode::Text(prev))) = (&n, kids.last_mut()) {
                prev.push_str(t);
                return;
            }
            kids.push(n);
        }
    };
    loop {
        match r
            .read_event()
            .unwrap_or_else(|e| panic!("xml parse: {e} in {}", truncate(s)))
        {
            Event::Start(e) => stack.push(XNode::El {
                name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                attrs: attrs_of(&e),
                kids: vec![],
            }),
            Event::Empty(e) => push(
                &mut stack,
                XNode::El {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    attrs: attrs_of(&e),
                    kids: vec![],
                },
            ),
            Event::End(_) => {
                let n = stack.pop().expect("balanced");
                push(&mut stack, n);
            }
            Event::Text(t) => {
                let s = t.decode().expect("text").into_owned();
                if !s.is_empty() {
                    push(&mut stack, XNode::Text(s));
                }
            }
            Event::CData(t) => push(
                &mut stack,
                XNode::Text(String::from_utf8_lossy(&t).into_owned()),
            ),
            Event::GeneralRef(e) => {
                let name = String::from_utf8_lossy(&e).into_owned();
                let ch = match name.as_str() {
                    "amp" => "&".to_string(),
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "quot" => "\"".to_string(),
                    "apos" => "'".to_string(),
                    n if n.starts_with("#x") => {
                        char::from_u32(u32::from_str_radix(&n[2..], 16).expect("hex ref"))
                            .expect("char")
                            .to_string()
                    }
                    n if n.starts_with('#') => char::from_u32(n[1..].parse().expect("dec ref"))
                        .expect("char")
                        .to_string(),
                    n => panic!("unknown entity {n}"),
                };
                push(&mut stack, XNode::Text(ch));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    stack.pop().expect("doc")
}

/// Canonical form: attributes sorted, text escaped minimally, empty elements self-closed.
pub fn canon(n: &XNode, out: &mut String) {
    match n {
        XNode::Text(t) => out.push_str(
            &t.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        ),
        XNode::El { name, attrs, kids } => {
            let doc = name == "#doc";
            if !doc {
                out.push('<');
                out.push_str(name);
                for (k, v) in attrs {
                    out.push_str(&format!(
                        " {k}=\"{}\"",
                        v.replace('&', "&amp;")
                            .replace('"', "&quot;")
                            .replace('<', "&lt;")
                    ));
                }
                if kids.is_empty() {
                    out.push_str("/>");
                    return;
                }
                out.push('>');
            }
            for k in kids {
                canon(k, out);
            }
            if !doc {
                out.push_str(&format!("</{name}>"));
            }
        }
    }
}

/// Rename attribute values (relationship ids) everywhere in a tree.
pub fn rename_values(n: &mut XNode, map: &BTreeMap<String, String>) {
    if let XNode::El { attrs, kids, .. } = n {
        for v in attrs.values_mut() {
            if let Some(new) = map.get(v) {
                *v = new.clone();
            }
        }
        for k in kids {
            rename_values(k, map);
        }
    }
}

/// Replace the text of named elements (timestamps).
pub fn mask_text(n: &mut XNode, names: &[&str], with: &str) {
    if let XNode::El { name, kids, .. } = n {
        if names.contains(&name.as_str()) {
            *kids = vec![XNode::Text(with.into())];
            return;
        }
        for k in kids {
            mask_text(k, names, with);
        }
    }
}

/// Relationships of a .rels tree whose `Type` ends with `suffix`, in document order.
pub fn rel_ids(rels: &XNode, suffix: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let XNode::El { kids, .. } = rels {
        for k in kids {
            if let XNode::El {
                name,
                attrs,
                kids: inner,
            } = k
            {
                if name == "Relationship" && attrs.get("Type").is_some_and(|t| t.ends_with(suffix))
                {
                    out.push(attrs["Id"].clone());
                }
                if name != "Relationship" {
                    out.extend(rel_ids(k, suffix));
                }
                let _ = inner;
            }
        }
    }
    out
}

// ------------------------------------------------------------------ TS cross-check

/// Read .docx files with mammoth on the TS side (`tools/golden/render-svc.ts --mammoth`). `None` when the TS side
/// is not available (no pnpm/tsx, or `OC_SKIP_TS_VERIFY` set).
pub fn ts_mammoth(files: &[Vec<u8>]) -> Option<Value> {
    if std::env::var_os("OC_SKIP_TS_VERIFY").is_some() {
        eprintln!("OC_SKIP_TS_VERIFY set: skipping the mammoth cross-check");
        return None;
    }
    let root = root();
    if !root.join("tools/golden/render-svc.ts").exists()
        || !root.join("services/api/node_modules/mammoth").exists()
    {
        eprintln!("mammoth / the generator is not present: skipping the TS cross-check");
        return None;
    }
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let input = std::env::temp_dir().join(format!("oc-render-svc-mammoth-{tag}.json"));
    let output = std::env::temp_dir().join(format!("oc-render-svc-mammoth-{tag}.out.json"));
    std::fs::write(
        &input,
        serde_json::to_vec(
            &serde_json::json!({ "files": files.iter().map(|f| b64(f)).collect::<Vec<_>>() }),
        )
        .expect("json"),
    )
    .expect("write input");
    let out = Command::new("pnpm")
        .args(["tsx", "tools/golden/render-svc.ts", "--mammoth"])
        .arg(&input)
        .arg("--out")
        .arg(&output)
        .current_dir(&root)
        .output();
    let _ = std::fs::remove_file(&input);
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cannot run pnpm ({e}): skipping the mammoth cross-check");
            return None;
        }
    };
    assert!(
        out.status.success(),
        "mammoth read failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&output).expect("mammoth output");
    let _ = std::fs::remove_file(&output);
    Some(serde_json::from_slice(&bytes).expect("mammoth json"))
}
