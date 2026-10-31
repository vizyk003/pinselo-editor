// SPDX-License-Identifier: Apache-2.0
//! Docs → Markdown / Word against `tests/golden/render-svc/docs.json` and `docx/*.json`.

use std::collections::BTreeMap;

use futures::future::BoxFuture;
use oc_crdt::doc_body::{DOC_FIELD, PmNode, fragment_to_json};
use oc_render_svc::doc_markdown::{MarkdownOptions, doc_to_markdown};
use oc_render_svc::docx::{
    DepError, DepResult, DocxDeps, DocxError, DocxImage, DocxMeta, ImageType, doc_default_font,
    docx_package,
};
use oc_schema::doc_blocks::DocChartData;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use yrs::{Transact, WriteTxn};

use crate::common::*;

fn pm(v: &Value) -> PmNode {
    serde_json::from_value(v.clone()).expect("pm node")
}

/// Marks sorted by type (yrs keeps formatting attributes in a hash map; yjs in insertion order).
fn sort_marks(n: &mut PmNode) {
    if let Some(m) = n.marks.as_mut() {
        m.sort_by(|a, b| a.kind.cmp(&b.kind));
    }
    for c in n.content.iter_mut().flatten() {
        sort_marks(c);
    }
}

#[test]
fn markdown_matches_ts() {
    let g = golden("docs.json");
    let origin = g["origin"].as_str().unwrap().to_string();
    let mut n = 0;
    for c in g["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let body = pm(&c["pm"]);
        let md = doc_to_markdown(
            &body,
            &MarkdownOptions {
                origin: Some(origin.clone()),
            },
        );
        if let Some(d) = str_diff(&md, c["markdown"].as_str().unwrap()) {
            panic!("{name}: markdown {d}");
        }
        let md = doc_to_markdown(&body, &MarkdownOptions::default());
        if let Some(d) = str_diff(&md, c["markdownNoOrigin"].as_str().unwrap()) {
            panic!("{name}: markdown (no origin) {d}");
        }
        assert_eq!(
            doc_default_font(&body),
            c["defaultFont"].as_str().unwrap(),
            "{name}: default font"
        );
        n += 1;
    }
    assert!(n >= 10);
}

/// The service reads the doc body from the stored Yjs state with oc-crdt: same tree (marks aside, whose order
/// yrs does not keep), same Markdown.
#[test]
fn crdt_state_reads_back_the_same_body() {
    let g = golden("docs.json");
    for c in g["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let ydoc = oc_crdt::codec::new_doc();
        oc_crdt::codec::apply_update(&ydoc, &unb64(c["state"].as_str().unwrap()), None)
            .expect("apply");
        let mut body = {
            let mut txn = ydoc.transact_mut();
            let frag = txn.get_or_insert_xml_fragment(DOC_FIELD);
            fragment_to_json(&txn, &frag)
        };
        let mut want = pm(&c["pm"]);
        let md = doc_to_markdown(
            &body,
            &MarkdownOptions {
                origin: Some(g["origin"].as_str().unwrap().into()),
            },
        );
        assert_eq!(
            md,
            c["markdown"].as_str().unwrap(),
            "{name}: markdown from the stored state"
        );
        sort_marks(&mut body);
        sort_marks(&mut want);
        if let Some(d) = diff(
            &serde_json::to_value(&body).unwrap(),
            &serde_json::to_value(&want).unwrap(),
            name,
        ) {
            panic!("doc body read back differs: {d}");
        }
    }
}

/// Replays the TS deps calls in order, checking the arguments.
struct Replay {
    calls: Vec<Value>,
    at: usize,
}

impl Replay {
    fn next(&mut self, f: &str, args: Value) -> DepResult {
        let call = self
            .calls
            .get(self.at)
            .unwrap_or_else(|| panic!("unexpected extra call {f}({args})"))
            .clone();
        self.at += 1;
        assert_eq!(
            call["fn"], f,
            "call #{}: {f}({args}) but TS called {}",
            self.at, call
        );
        if let Some(d) = diff(&args, &call["args"], &format!("call #{} {f}", self.at)) {
            panic!("arguments differ: {d}");
        }
        match &call["result"] {
            Value::Null => Ok(None),
            Value::String(s) if s == "throw" => Err(DepError::Failed(format!("{f} failed"))),
            r => Ok(Some(DocxImage {
                data: asset(r["asset"].as_str().unwrap()),
                kind: ImageType::parse(r["type"].as_str().unwrap()).unwrap(),
                w: r["w"].as_f64().unwrap(),
                h: r["h"].as_f64().unwrap(),
            })),
        }
    }
}

impl DocxDeps for Replay {
    fn image<'a>(&'a mut self, src: &'a str) -> BoxFuture<'a, DepResult> {
        let r = self.next("image", json!([src]));
        Box::pin(async move { r })
    }
    fn embed<'a>(
        &'a mut self,
        design_id: &'a str,
        page_id: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, DepResult> {
        let r = self.next("embed", json!([design_id, page_id, width_px]));
        Box::pin(async move { r })
    }
    fn chart<'a>(&'a mut self, data: &'a DocChartData, w: f64, h: f64) -> BoxFuture<'a, DepResult> {
        let r = self.next(
            "chart",
            json!([oc_schema::js::to_value(data).unwrap(), w, h]),
        );
        Box::pin(async move { r })
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// Normalised XML of every part: attributes sorted, hyperlink relationship ids renamed by order of appearance,
/// core.xml timestamps masked.
fn normalise(parts: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let rels = parse_xml(&parts["word/_rels/document.xml.rels"]);
    let mut map = BTreeMap::new();
    for (i, id) in rel_ids(&rels, "/hyperlink").into_iter().enumerate() {
        map.insert(id, format!("rIdLINK{}", i + 1));
    }
    parts
        .iter()
        .map(|(k, v)| {
            let mut t = parse_xml(v);
            rename_values(&mut t, &map);
            mask_text(&mut t, &["dcterms:created", "dcterms:modified"], "DATE");
            let mut s = String::new();
            canon(&t, &mut s);
            (k.clone(), s)
        })
        .collect()
}

fn rust_docx(c: &Value) -> Result<oc_render_svc::docx::DocxPackage, DocxError> {
    let body = pm(&golden("docs.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == c["name"])
        .unwrap()["pm"]);
    let mut deps = Replay {
        calls: c["calls"].as_array().unwrap().clone(),
        at: 0,
    };
    let r = block_on(docx_package(
        &body,
        &DocxMeta {
            title: c["title"].as_str().unwrap().into(),
            font: None,
        },
        &mut deps,
        chrono::Utc::now(),
    ));
    assert_eq!(
        deps.at,
        deps.calls.len(),
        "{}: TS made {} deps calls, Rust {}",
        c["name"],
        deps.calls.len(),
        deps.at
    );
    r
}

#[test]
fn docx_parts_match_ts() {
    let mut n = 0;
    for (file, c) in golden_dir("docx") {
        let name = c["name"].as_str().unwrap();
        let res = rust_docx(&c);
        if let Some(err) = c.get("error") {
            let e = res
                .err()
                .unwrap_or_else(|| panic!("{name}: TS threw ({err}), Rust exported"));
            assert_eq!(
                e.to_string(),
                err.as_str().unwrap(),
                "{name}: error message"
            );
            continue;
        }
        let pkg = res.unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut ours: BTreeMap<String, String> = BTreeMap::new();
        let mut media: BTreeMap<String, String> = BTreeMap::new();
        for (part, bytes) in &pkg.parts {
            if part.starts_with("word/media/") {
                media.insert(part.clone(), hex::encode(Sha256::digest(bytes)));
            } else {
                ours.insert(part.clone(), String::from_utf8(bytes.clone()).unwrap());
            }
        }
        let theirs: BTreeMap<String, String> = c["parts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            theirs.keys().collect::<Vec<_>>(),
            "{file}: part names"
        );
        let (a, b) = (normalise(&ours), normalise(&theirs));
        for k in b.keys() {
            if let Some(d) = str_diff(&a[k], &b[k]) {
                panic!("{file} {k}: {d}");
            }
        }
        let want: BTreeMap<String, String> = c["media"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        assert_eq!(media, want, "{file}: media");
        n += 1;
    }
    assert!(n >= 10);
}

/// The Rust-written .docx opens on the TS side: mammoth reads the same HTML, text and messages from it as from
/// the TS-written one.
#[test]
fn docx_reads_back_with_mammoth() {
    let cases: Vec<(String, Value)> = golden_dir("docx")
        .into_iter()
        .filter(|(_, c)| c.get("error").is_none())
        .collect();
    let files: Vec<Vec<u8>> = cases
        .iter()
        .map(|(_, c)| rust_docx(c).unwrap().to_zip().unwrap())
        .collect();
    let Some(read) = ts_mammoth(&files) else {
        return;
    };
    for ((file, c), got) in cases.iter().zip(read.as_array().unwrap()) {
        if let Some(d) = diff(got, &c["mammoth"], file) {
            panic!("mammoth reads the Rust .docx differently: {d}");
        }
    }
}

/// docLayout.ts `blocksOf` (the pure half of Docs pagination).
#[test]
fn layout_blocks_match_ts() {
    let g = golden("docs.json");
    for c in g["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let got = oc_render_svc::doc_layout::blocks_of(&pm(&c["pm"]));
        match (&c["blocks"].get("ok"), got) {
            (Some(want), Ok(blocks)) => {
                let got = oc_schema::js::to_value(&blocks).unwrap();
                if let Some(d) = diff(&got, want, &format!("{name}.blocks")) {
                    panic!("{d}");
                }
            }
            (None, Err(e)) => assert_eq!(
                e.to_string(),
                c["blocks"]["error"].as_str().unwrap(),
                "{name}: error"
            ),
            (want, got) => panic!(
                "{name}: TS {:?}, Rust {:?}",
                want.is_some(),
                got.map(|_| ())
            ),
        }
        let hs: Vec<Value> = oc_render_svc::doc_layout::doc_headings(&pm(&c["pm"]))
            .into_iter()
            .map(|(l, t)| json!({ "level": l, "text": t }))
            .collect();
        if let Some(d) = diff(
            &Value::Array(hs),
            &c["headings"],
            &format!("{name}.headings"),
        ) {
            panic!("{d}");
        }
    }
}
