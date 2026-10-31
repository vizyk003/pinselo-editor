// SPDX-License-Identifier: Apache-2.0
//! Word → HTML parity with mammoth as `POST /api/docs/import-docx` runs it (tests/golden/import/docx/, written by
//! tools/golden/import.ts).

mod common;

use oc_import::docx::{DocxImage, docx_to_html};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn docx_goldens() {
    let cases = common::cases("docx");
    assert!(cases.len() >= 7, "expected the docx goldens, found {cases:?}");
    let mut failures = Vec::new();
    for name in cases {
        let g = common::golden("docx", &name);
        let file = g["file"].as_str().expect("file");
        let bytes = common::fixture(file);
        let mut images: Vec<Value> = Vec::new();
        let result = docx_to_html(&bytes, &mut |img: &DocxImage<'_>| {
            images.push(json!({ "contentType": img.content_type, "sha256": hex::encode(Sha256::digest(img.bytes)), "bytes": img.bytes.len() }));
            format!("/api/assets/A{:015}", images.len())
        });
        if g.get("error").is_some() {
            if result.is_ok() {
                failures.push(format!("{name}: expected the conversion to fail"));
            }
            continue;
        }
        let r = match result {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: conversion failed: {e}"));
                continue;
            }
        };
        let got = json!({ "file": file, "html": r.html, "warnings": r.messages.len(), "messages": r.messages, "images": images });
        let mut diffs = Vec::new();
        common::diff(&g, &got, &name, &|_| 0.0, &mut diffs);
        if !diffs.is_empty() {
            failures.push(diffs.join("\n"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
