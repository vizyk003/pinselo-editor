// SPDX-License-Identifier: Apache-2.0
//! Print options of PDF exports (`print` on `/internal/export` and the `export` job), colour profiles and the
//! print routes:
//!
//! | route | |
//! |---|---|
//! | `POST /internal/icc/inspect` | raw ICC bytes → `{ ok, colorSpace, channels, class, description, version }` or 422 |
//! | `POST /internal/print/gamut` | `{ profile, intent, colors }` → `{ results: [{ inGamut, deltaE, cmyk }] }` |
//!
//! `profile` is a bundled id (`packages/schema/src/icc-profiles.json`, files in `fixtures/icc` from `pnpm icc:sync`)
//! or an uploaded profile the API resolved: `{ asset: <sha256 in the object store>, name?, conditionId? }`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use oc_color::{Intent, OutputProfile};
use oc_schema::primitives::ColorValue;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::renderer::{PrintSetup, ResolvedProfile};
use crate::service::{Service, SvcError};

/// A profile reference: bundled id or a stored upload.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ProfileArg {
    Bundled(String),
    Asset {
        asset: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default, rename = "conditionId")]
        condition_id: Option<String>,
    },
}

/// The `print` request field.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PrintBody {
    #[serde(default = "three")]
    pub bleed_mm: f64,
    #[serde(default = "yes")]
    pub crop_marks: bool,
    #[serde(default)]
    pub registration_marks: bool,
    #[serde(default)]
    pub color_bars: bool,
    #[serde(default)]
    pub slug_mm: Option<f64>,
    #[serde(default)]
    pub standard: Option<String>,
    #[serde(default)]
    pub flatten: bool,
    #[serde(default)]
    pub profile: Option<ProfileArg>,
    #[serde(default)]
    pub intent: Option<String>,
}

fn three() -> f64 {
    3.0
}
fn yes() -> bool {
    true
}

fn bad(msg: impl Into<String>) -> SvcError {
    SvcError::Other(msg.into())
}

/// Validate a `print` body and load its profile.
pub async fn resolve(svc: &Service, b: &PrintBody) -> Result<PrintSetup, SvcError> {
    if !(0.0..=10.0).contains(&b.bleed_mm) {
        return Err(bad("print.bleedMm must be 0–10"));
    }
    let standard = b.standard.clone().unwrap_or_else(|| "none".into());
    if !matches!(standard.as_str(), "none" | "pdfx4" | "pdfx1a") {
        return Err(bad("print.standard must be none, pdfx4 or pdfx1a"));
    }
    // PDF/X and CMYK flattening need a profile: the default print profile unless one was chosen.
    let profile = match (&b.profile, standard.as_str()) {
        (Some(p), _) => Some(p.clone()),
        (None, "pdfx4" | "pdfx1a") => Some(ProfileArg::Bundled(oc_color::registry::default_id().into())),
        (None, _) => None,
    };
    let profile = match profile {
        Some(p) => Some(load_profile(svc, &p).await?),
        None => None,
    };
    if standard == "pdfx1a" && profile.as_ref().is_some_and(|p| !output_profile(p).map(|o| o.is_cmyk()).unwrap_or(false)) {
        return Err(bad("PDF/X-1a needs a CMYK profile"));
    }
    Ok(PrintSetup {
        bleed_mm: b.bleed_mm,
        crop_marks: b.crop_marks,
        registration_marks: b.registration_marks,
        color_bars: b.color_bars,
        slug_mm: b.slug_mm.map(|s| s.clamp(0.0, 20.0)),
        standard,
        flatten: b.flatten,
        intent: b.intent.clone().unwrap_or_else(|| "relative".into()),
        profile,
    })
}

/// Bytes and output-intent metadata of a profile reference.
pub async fn load_profile(svc: &Service, p: &ProfileArg) -> Result<ResolvedProfile, SvcError> {
    match p {
        ProfileArg::Bundled(id) => {
            let meta = oc_color::registry::find(id).ok_or_else(|| bad(format!("unknown colour profile `{id}`")))?;
            let bytes = oc_color::registry::read_bundled(id, &svc.settings.icc_dir).map_err(|e| bad(e.to_string()))?;
            Ok(ResolvedProfile {
                key: format!("bundled:{id}"),
                bytes: Arc::new(bytes),
                condition_id: meta.condition_id.clone(),
                condition: Some(meta.condition.clone()),
                registry: Some(oc_color::registry::registry_name().to_string()),
                info: Some(meta.name.clone()),
            })
        }
        ProfileArg::Asset { asset, name, condition_id } => {
            if !(asset.len() == 64 && asset.bytes().all(|c| c.is_ascii_hexdigit())) {
                return Err(bad("profile.asset must be a sha256"));
            }
            let bytes = svc.objects.get(asset).await.map_err(|e| SvcError::Storage(e.to_string()))?;
            let info = oc_color::inspect(&bytes).map_err(|e| bad(e.to_string()))?;
            let label = name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| info.description.clone());
            // A custom condition is identified by its name (no registry entry).
            let cond = condition_id.clone().filter(|c| !c.trim().is_empty()).unwrap_or_else(|| if label.is_empty() { "Custom".into() } else { label.clone() });
            Ok(ResolvedProfile { key: format!("asset:{asset}"), bytes: Arc::new(bytes), condition_id: cond, condition: Some(label.clone()), registry: None, info: Some(label) })
        }
    }
}

/// The parsed profile (cached per key: building transforms and image LUTs is the expensive part).
pub fn output_profile(p: &ResolvedProfile) -> Result<OutputProfile, String> {
    static CACHE: OnceLock<Mutex<HashMap<String, OutputProfile>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(o) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&p.key) {
        return Ok(o.clone());
    }
    let o = OutputProfile::new(p.bytes.to_vec()).map_err(|e| e.to_string())?;
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 16 {
        c.clear();
    }
    c.insert(p.key.clone(), o.clone());
    Ok(o)
}

/// The renderer-level print setup → oc-render's.
pub fn to_pdf_print(p: &PrintSetup) -> Result<oc_render::pdf::PrintSetup, String> {
    let output = match &p.profile {
        Some(rp) => Some(oc_render::pdf::OutputIntent {
            profile: output_profile(rp)?,
            condition_id: rp.condition_id.clone(),
            condition: rp.condition.clone(),
            registry: rp.registry.clone(),
            info: rp.info.clone(),
        }),
        None => None,
    };
    Ok(oc_render::pdf::PrintSetup {
        bleed_mm: p.bleed_mm,
        crop_marks: p.crop_marks,
        registration_marks: p.registration_marks,
        color_bars: p.color_bars,
        slug_mm: p.slug_mm,
        standard: oc_render::pdf::PdfStandard::parse(&p.standard),
        output,
        intent: Intent::parse(&p.intent),
        flatten: p.flatten,
        ..oc_render::pdf::PrintSetup::default()
    })
}

fn json_status(status: StatusCode, v: Value) -> Response {
    (status, axum::Json(v)).into_response()
}

/// `POST /internal/icc/inspect`: validate an uploaded ICC profile with moxcms.
pub async fn icc_inspect(body: Bytes) -> Response {
    match oc_color::inspect(&body) {
        Ok(info) => json_status(
            StatusCode::OK,
            json!({ "ok": true, "colorSpace": info.color_space, "channels": info.channels, "class": info.class, "description": info.description, "version": info.version }),
        ),
        Err(e) => json_status(StatusCode::UNPROCESSABLE_ENTITY, json!({ "ok": false, "error": "invalid_icc", "message": e.to_string() })),
    }
}

#[derive(Debug, Deserialize)]
struct GamutBody {
    profile: ProfileArg,
    #[serde(default)]
    intent: Option<String>,
    colors: Vec<Value>,
}

/// `POST /internal/print/gamut`: out-of-gamut check of colours against a CMYK profile.
pub async fn gamut(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let b: GamutBody = match serde_json::from_value(b) {
        Ok(b) => b,
        Err(e) => return json_status(StatusCode::BAD_REQUEST, json!({ "error": "bad_request", "message": e.to_string() })),
    };
    if b.colors.len() > 512 {
        return json_status(StatusCode::BAD_REQUEST, json!({ "error": "bad_request", "message": "at most 512 colours" }));
    }
    let rp = match load_profile(&st.svc, &b.profile).await {
        Ok(p) => p,
        Err(e) => return json_status(StatusCode::NOT_FOUND, json!({ "error": "profile_not_found", "message": e.to_string() })),
    };
    let conv = match output_profile(&rp).and_then(|o| o.converter(Intent::parse(b.intent.as_deref().unwrap_or("relative"))).map_err(|e| e.to_string())) {
        Ok(c) => c,
        Err(e) => return json_status(StatusCode::UNPROCESSABLE_ENTITY, json!({ "error": "unsupported_profile", "message": e })),
    };
    let results: Vec<Value> = b
        .colors
        .iter()
        .map(|c| match serde_json::from_value::<ColorValue>(c.clone()) {
            Ok(cv) => {
                let g = conv.gamut(&cv);
                let r1 = |v: f64| (v * 10.0).round() / 10.0;
                json!({ "inGamut": g.in_gamut, "deltaE": r1(g.delta_e), "cmyk": g.cmyk.map(|v| r1(v * 100.0)) })
            }
            Err(_) => json!({ "inGamut": true, "deltaE": 0, "cmyk": [0, 0, 0, 0], "error": "bad_color" }),
        })
        .collect();
    json_status(StatusCode::OK, json!({ "results": results }))
}
