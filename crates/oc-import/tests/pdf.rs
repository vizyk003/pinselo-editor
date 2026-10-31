// SPDX-License-Identifier: Apache-2.0
//! PDF importer parity: tests/golden/import/pdf/*.json (tools/golden/import.ts, the TS importer with pdf.js and
//! poppler) against `oc_import::import_pdf`.
//!
//! Compared exactly (within 1e-6): pages, every element's geometry, text, fonts, sizes, alignment and line
//! spacing, the page pictures handed to `media` (name and PNG size). Text colours are measured from renders, and
//! hayro anti-aliases differently from cairo, so the sampled colour channels are compared within 0.05.

mod common;

use std::collections::HashSet;

use serde_json::{Value, json};

fn png_size(b: &[u8]) -> (u32, u32) {
    (u32::from_be_bytes([b[16], b[17], b[18], b[19]]), u32::from_be_bytes([b[20], b[21], b[22], b[23]]))
}

/// Paths of colours sampled from the renders (text fills).
fn sampled(path: &str) -> bool {
    path.contains(".body.defaultRun.fill.color.")
}

#[test]
fn pdf_goldens() {
    let families: HashSet<String> = common::families().into_iter().collect();
    let available = |f: &str| families.contains(f);
    let mut failures = Vec::new();
    let cases = common::cases("pdf");
    assert!(!cases.is_empty());
    for case in cases {
        let g = common::golden("pdf", &case);
        let file = g["file"].as_str().expect("file");
        let mut calls = Vec::new();
        let opts = oc_import::ImportOptions { font_available: &available, title: Some(case.clone()) };
        let doc = oc_import::import_pdf(&common::fixture(file), &opts, &mut |b: &[u8], name: &str| {
            let (w, h) = png_size(b);
            calls.push(json!({ "name": name, "w": w, "h": h }));
            Some(format!("A{:015}", calls.len()))
        });
        let doc = match doc {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{case}: {e}"));
                continue;
            }
        };
        let mut out = Vec::new();
        common::diff(&g["media"], &Value::Array(calls), &format!("{case}.media"), &|_| 1e-6, &mut out);
        let got = common::normalize_doc(&oc_schema::js::to_value(&doc).expect("doc json"));
        let want = common::normalize_doc(&g["doc"]);
        common::diff(&want, &got, &format!("{case}.doc"), &|p| if sampled(p) { 0.05 } else { 1e-6 }, &mut out);
        failures.extend(out);
    }
    assert!(failures.is_empty(), "{} mismatch(es):\n{}", failures.len(), failures.join("\n"));
}
