// SPDX-License-Identifier: Apache-2.0
//! PowerPoint import parity: every tests/golden/import/pptx/*.json (tools/golden/import.ts, from the TS importer)
//! against `oc_import::import_pptx` on the same fixture, ids normalised.

mod common;

use std::collections::HashSet;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn pptx_goldens() {
    let fams: HashSet<String> = common::families().into_iter().collect();
    let available = |f: &str| fams.contains(f);
    let cases = common::cases("pptx");
    assert!(!cases.is_empty(), "no pptx goldens");
    let mut failures = Vec::new();
    for case in cases {
        let g = common::golden("pptx", &case);
        let file = g["file"].as_str().expect("file");
        let bytes = common::fixture(file);
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s).to_owned();
        let mut media: Vec<Value> = Vec::new();
        let opts = oc_import::ImportOptions { font_available: &available, title: Some(stem) };
        let mut cb = |b: &[u8], name: &str| -> Option<String> {
            media.push(json!({ "name": name, "sha256": hex::encode(Sha256::digest(b)), "bytes": b.len() }));
            Some(format!("A{:015}", media.len()))
        };
        let doc = match oc_import::import_pptx(&bytes, &opts, &mut cb) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{case}: import failed: {e}"));
                continue;
            }
        };
        let got = common::normalize_doc(&oc_schema::js::to_value(&doc).expect("doc json"));
        let want = common::normalize_doc(&g["doc"]);
        let mut out = Vec::new();
        common::diff(&g["media"], &Value::Array(media), "media", &|_| 0.0, &mut out);
        common::diff(&want, &got, "doc", &|_| 1e-6, &mut out);
        if !out.is_empty() {
            failures.push(format!("{case}:\n  {}", out.join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "{} case(s) differ:\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn not_a_presentation() {
    let fams = |_: &str| false;
    let opts = oc_import::ImportOptions { font_available: &fams, title: None };
    let bytes = common::fixture("word-features.docx");
    let mut cb = |_: &[u8], _: &str| -> Option<String> { None };
    assert!(matches!(oc_import::import_pptx(&bytes, &opts, &mut cb), Err(oc_import::ImportError::NotPresentation)));
    assert!(oc_import::import_pptx(b"not a zip", &opts, &mut cb).is_err());
}
