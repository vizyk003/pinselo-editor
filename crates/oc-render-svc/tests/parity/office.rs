// SPDX-License-Identifier: Apache-2.0
//! pptx.ts and email.ts parity with the renderer replayed (`tests/golden/render-svc/pptx/`, `email/`).

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use futures::future::BoxFuture;
use oc_render_svc::email::{EmailDeps, export_email};
use oc_render_svc::pptx::{AssetError, PptxDeps, pptx_package};
use oc_render_svc::renderer::RenderError;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::common::*;
use crate::replay::{ENCODED, Replay, TRANSCODED, TestHost};

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

/// Asset bytes exactly as the TS run got them.
pub struct Assets(pub HashMap<String, Option<String>>);

impl PptxDeps for Assets {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, AssetError>> {
        let r = match self
            .0
            .get(key)
            .unwrap_or_else(|| panic!("TS never asked for asset {key}"))
        {
            Some(f) => Ok(asset(f)),
            None => Err(AssetError::Missing(key.into())),
        };
        Box::pin(async move { r })
    }
}

static RID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(r:id|r:embed)="(rId\d+)""#).unwrap());
static CNVPR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<p:cNvPr id="(\d+)" name="([^"]*)""#).unwrap());
static REL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<Relationship Id="([^"]+)" Type="([^"]+)" Target="([^"]+)"( TargetMode="External")?/>"#,
    )
    .unwrap()
});
static DATES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(<dcterms:(created|modified) xsi:type="dcterms:W3CDTF">)[^<]*"#).unwrap()
});

/// Ids renumbered by first appearance: shape ids, relationship ids per slide, media names across the deck (the TS
/// hands them out in promise order inside groups). Returns the parts and the media renaming.
pub fn canonical(
    parts: &BTreeMap<String, String>,
    slides: usize,
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut out = parts.clone();
    let mut media: BTreeMap<String, String> = BTreeMap::new();
    for i in 1..=slides {
        let sk = format!("ppt/slides/slide{i}.xml");
        let rk = format!("ppt/slides/_rels/slide{i}.xml.rels");
        let (slide, rels) = (parts[&sk].clone(), parts[&rk].clone());
        let targets: HashMap<String, (String, String)> = REL
            .captures_iter(&rels)
            .map(|c| (c[1].to_string(), (c[2].to_string(), c[3].to_string())))
            .collect();
        let mut ids: HashMap<String, String> = HashMap::new();
        for c in RID.captures_iter(&slide) {
            let old = c[2].to_string();
            if ids.contains_key(&old) {
                continue;
            }
            let new = format!("rIdC{}", ids.len() + 2);
            if let Some((ty, target)) = targets.get(&old)
                && ty.ends_with("/image")
            {
                let name = target.trim_start_matches("../media/").to_string();
                let ext = name.rsplit('.').next().unwrap_or("").to_string();
                let k = media.len() + 1;
                media
                    .entry(name)
                    .or_insert_with(|| format!("media{k}.{ext}"));
            }
            ids.insert(old, new);
        }
        let slide = RID
            .replace_all(&slide, |c: &regex::Captures| {
                format!(
                    "{}=\"{}\"",
                    &c[1],
                    ids.get(&c[2]).cloned().unwrap_or_else(|| c[2].to_string())
                )
            })
            .into_owned();
        let mut k = 0;
        let slide = CNVPR.replace_all(&slide, |c: &regex::Captures| {
            k += 1;
            // Default names are `<type> <id - 1>`: renumber them with the id.
            let raw: u64 = c[1].parse().unwrap();
            let name = match c[2].rsplit_once(' ') {
                Some((ty, num)) if num.parse::<u64>().ok() == Some(raw - 1) => {
                    format!("{ty} {}", k - 1)
                }
                _ => c[2].to_string(),
            };
            format!("<p:cNvPr id=\"{k}\" name=\"{name}\"")
        });
        let mut entries: Vec<(String, String)> = REL
            .captures_iter(&rels)
            .map(|c| {
                let id = ids.get(&c[1]).cloned().unwrap_or_else(|| c[1].to_string());
                let target = match c[3].strip_prefix("../media/") {
                    Some(m) => format!(
                        "../media/{}",
                        media.get(m).cloned().unwrap_or_else(|| m.to_string())
                    ),
                    None => c[3].to_string(),
                };
                (
                    id.clone(),
                    format!(
                        "<Relationship Id=\"{id}\" Type=\"{}\" Target=\"{target}\"{}/>",
                        &c[2],
                        c.get(4).map_or("", |m| m.as_str())
                    ),
                )
            })
            .collect();
        entries.sort();
        let head = &rels[..rels
            .find("<Relationship ")
            .unwrap_or(rels.len() - "</Relationships>".len())];
        out.insert(sk, slide.into_owned());
        out.insert(
            rk,
            format!(
                "{head}{}</Relationships>",
                entries.into_iter().map(|e| e.1).collect::<String>()
            ),
        );
    }
    if let Some(core) = out.get_mut("docProps/core.xml") {
        *core = DATES.replace_all(core, "${1}DATE").into_owned();
    }
    (out, media)
}

fn norm_calls(calls: &[Value]) -> Vec<String> {
    let mut v: Vec<String> = calls
        .iter()
        .map(|c| {
            let mut c = c.clone();
            oc_schema::js::normalize_numbers(&mut c);
            c.to_string()
        })
        .collect();
    v.sort();
    v
}

#[test]
fn decks_match_ts() {
    let mut decks = 0;
    for (file, g) in golden_dir("pptx") {
        let name = g["name"].as_str().unwrap();
        let doc = design(name);
        let mut r = Replay::default();
        for (d, c) in g["paths"].as_object().unwrap() {
            r.paths.insert(
                d.clone(),
                c.as_array()
                    .map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()),
            );
        }
        for (sha, ok) in g["decoded"].as_object().unwrap() {
            r.decoded.insert(sha.clone(), ok.as_bool().unwrap());
        }
        let assets = Assets(
            g["assets"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().map(str::to_string)))
                .collect(),
        );
        let pages: Vec<String> = g["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let pkg = block_on(pptx_package(
            &r,
            &TestHost::new(),
            &doc,
            &pages,
            &assets,
            chrono::Utc::now(),
        ))
        .unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut ours: BTreeMap<String, String> = BTreeMap::new();
        let mut our_media: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (part, bytes) in &pkg.parts {
            if let Some(m) = part.strip_prefix("ppt/media/") {
                our_media.insert(m.to_string(), bytes.clone());
            } else {
                ours.insert(part.clone(), String::from_utf8(bytes.clone()).unwrap());
            }
        }
        let theirs: BTreeMap<String, String> = g["parts"]
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
        let (a, am) = canonical(&ours, pages.len());
        let (b, bm) = canonical(&theirs, pages.len());
        for k in b.keys() {
            if let Some(d) = str_diff(&a[k], &b[k]) {
                panic!("{file} {k}: {d}");
            }
        }
        // Media: same count and names after renumbering; bytes copied from assets are identical.
        let their_media: BTreeMap<String, String> = g["media"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["name"]
                        .as_str()
                        .unwrap()
                        .trim_start_matches("ppt/media/")
                        .to_string(),
                    m["sha256"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(our_media.len(), their_media.len(), "{file}: media count");
        let replayed = [ENCODED.to_vec(), TRANSCODED.to_vec()];
        let mut by_canon: BTreeMap<String, String> = BTreeMap::new();
        for (raw, canon) in &bm {
            by_canon.insert(canon.clone(), their_media[raw].clone());
        }
        for (raw, canon) in &am {
            let bytes = &our_media[raw];
            if replayed.contains(bytes) {
                continue; // a picture the painter drew or transcoded
            }
            assert_eq!(
                hex::encode(Sha256::digest(bytes)),
                by_canon[canon],
                "{file}: media {canon} bytes"
            );
        }
        let calls = r.raster_calls.lock().unwrap().clone();
        assert_eq!(
            norm_calls(&calls),
            norm_calls(g["renderCalls"].as_array().unwrap()),
            "{file}: element pictures requested"
        );
        decks += 1;
    }
    assert!(decks >= 25, "{decks} decks");
}

struct Host {
    n: Mutex<usize>,
}

impl EmailDeps for Host {
    fn host<'a>(&'a self, _png: Vec<u8>) -> BoxFuture<'a, Result<String, RenderError>> {
        let k = {
            let mut n = self.n.lock().unwrap();
            *n += 1;
            *n
        };
        Box::pin(async move { Ok(format!("https://img.example/e/{k}.png")) })
    }
}

#[test]
fn emails_match_ts() {
    let mut n = 0;
    for (file, g) in golden_dir("email") {
        let doc = design(g["name"].as_str().unwrap());
        let r = Replay::default();
        let host = Host { n: Mutex::new(0) };
        let pages: Vec<String> = g["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let out = block_on(export_email(&r, &TestHost::new(), &doc, &pages, &host)).unwrap();
        if let Some(d) = str_diff(&out.html, g["html"].as_str().unwrap()) {
            panic!("{file}: html {d}");
        }
        assert_eq!(out.text, g["text"].as_str().unwrap(), "{file}: text");
        assert_eq!(
            *host.n.lock().unwrap() as u64,
            g["slices"].as_u64().unwrap(),
            "{file}: slices"
        );
        let calls = r.raster_calls.lock().unwrap().clone();
        let mut want = g["renderCalls"].as_array().unwrap().clone();
        for c in want.iter_mut() {
            oc_schema::js::normalize_numbers(c);
        }
        let mut got = calls;
        for c in got.iter_mut() {
            oc_schema::js::normalize_numbers(c);
        }
        assert_eq!(got, want, "{file}: page rasters");
        n += 1;
    }
    assert!(n >= 6);
}
