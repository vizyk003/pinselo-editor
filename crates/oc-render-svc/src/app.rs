// SPDX-License-Identifier: Apache-2.0
//! The HTTP routes and the `export` job — port of the route half of `services/render/src/main.ts`.
//!
//! | route | |
//! |---|---|
//! | `GET /healthz` | `{ ok: true }` |
//! | `POST /internal/thumbnail` | coalesced (1.2 s) first-page thumbnail → `designs.thumb_sha256` |
//! | `POST /internal/doc-to-design` | Docs laid out onto editable pages (+ generated embed pictures) |
//! | `POST /internal/page-png` | one page as PNG at a pixel width |
//! | `POST /internal/site-build` | the website HTML |
//! | `POST /internal/template-thumbnail` | template catalogue thumbnail → `templates.thumbnail` |
//! | `POST /internal/export` | png/jpg/zip/svg/pdf/pdf-print/pdf-flat/pptx/docx/md/mp4/gif/m4a |
//! | `POST /internal/icc/inspect`, `POST /internal/print/gamut` | colour profiles ([`crate::print`]) |
//!
//! `/internal/*` requires the internal service token (`x-oc-internal`), else 403 `{ error: 'forbidden' }`.
//! Renderer capabilities that are missing answer 501 `{ error: 'not_implemented', message }`.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use oc_schema::DesignDoc;
use oc_schema::design::AssetRef;
use oc_schema::tree::sorted_pages;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::renderer::{
    ImageFormat, PrintSetup, RasterOptions, RasterPdfOptions, RenderError, SvgOptions,
    VectorPdfOptions,
};
use crate::service::{Service, SvcError, safe_name};
use crate::site::{SiteOptions, build_site};
use crate::video::{VideoQuality, export_audio, export_gif, export_mp4};

/// Shared state of the app.
#[derive(Clone)]
pub struct AppState {
    pub svc: Arc<Service>,
    /// The internal service token's secret (`OC_SECRET`).
    pub secret: String,
    pending_thumbs: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// Delay before a thumbnail renders (bursts of saves coalesce).
    pub thumb_delay: Duration,
}

impl AppState {
    pub fn new(svc: Arc<Service>, secret: String) -> AppState {
        AppState {
            svc,
            secret,
            pending_thumbs: Arc::default(),
            thumb_delay: Duration::from_millis(1200),
        }
    }
}

fn json_response(status: StatusCode, v: Value) -> Response {
    (status, axum::Json(v)).into_response()
}

/// An error answer.
pub fn error_response(e: &SvcError) -> Response {
    match e {
        SvcError::Renderer(RenderError::NotImplemented(_)) => json_response(
            StatusCode::NOT_IMPLEMENTED,
            json!({ "error": "not_implemented", "message": e.to_string() }),
        ),
        SvcError::SheetUnsupported => json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "error": "sheet_unsupported", "message": e.to_string() }),
        ),
        _ => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "statusCode": 500, "error": "Internal Server Error", "message": e.to_string() }),
        ),
    }
}

impl IntoResponse for SvcError {
    fn into_response(self) -> Response {
        error_response(&self)
    }
}

fn file(bytes: Vec<u8>, ty: &str, filename: &str) -> Response {
    let mut r = Response::new(Body::from(bytes));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(ty).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

async fn internal_only(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if req.uri().path().starts_with("/internal/") {
        let v = req
            .headers()
            .get(oc_auth::INTERNAL_HEADER)
            .and_then(|v| v.to_str().ok());
        if !oc_auth::is_internal_with(&st.secret, v) {
            return json_response(StatusCode::FORBIDDEN, json!({ "error": "forbidden" }));
        }
    }
    next.run(req).await
}

/// The router (same routes, body limit 1 MiB, internal-token guard).
pub fn router(st: AppState) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|| async { axum::Json(json!({ "ok": true })) }),
        )
        .route("/internal/thumbnail", post(thumbnail))
        .route("/internal/doc-to-design", post(doc_to_design))
        .route("/internal/page-png", post(page_png))
        .route("/internal/page-lint", post(page_lint))
        .route("/internal/site-build", post(site_build))
        .route("/internal/template-thumbnail", post(template_thumbnail))
        .route("/internal/export", post(export))
        .route("/internal/print/gamut", post(crate::print::gamut))
        // ICC uploads are up to 10 MB.
        .route("/internal/icc/inspect", post(crate::print::icc_inspect).layer(DefaultBodyLimit::max(11 * 1024 * 1024)))
        .layer(middleware::from_fn_with_state(st.clone(), internal_only))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(st)
}

fn str_of<'a>(b: &'a Value, k: &str) -> Option<&'a str> {
    b.get(k).and_then(Value::as_str)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Thumbnail scale: the long side at most 480 px.
fn thumb_scale(doc: &DesignDoc) -> f64 {
    1f64.min(480.0 / doc.meta.dims.w.max(doc.meta.dims.h))
}

async fn render_thumbnail(svc: &Service, design_id: &str) -> Result<(), SvcError> {
    let _g = svc.serial.lock().await;
    let Some(doc) = svc.load_doc(design_id, &[], None).await? else {
        return Ok(());
    };
    let Some(page) = sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else {
        return Ok(());
    };
    let png = svc
        .renderer
        .render_image(
            svc,
            &doc,
            &page,
            &RasterOptions::at_scale(thumb_scale(&doc)),
            ImageFormat::Png,
        )
        .await?;
    let sha = svc.put(&png, "image/png").await?;
    sqlx::query("UPDATE designs SET thumb_sha256 = $2 WHERE id = $1")
        .bind(design_id)
        .bind(sha)
        .execute(&svc.pool)
        .await?;
    Ok(())
}

async fn thumbnail(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let id = str_of(&b, "designId").unwrap_or("").to_string();
    // Coalesce bursts of saves into one render.
    let mut pending = st.pending_thumbs.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = pending.remove(&id) {
        h.abort();
    }
    let (svc, map, delay, key) = (
        st.svc.clone(),
        st.pending_thumbs.clone(),
        st.thumb_delay,
        id.clone(),
    );
    let h = tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        map.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        if let Err(e) = render_thumbnail(&svc, &key).await {
            eprintln!("[render] thumbnail {key}: {e}");
        }
    });
    pending.insert(id, h);
    axum::Json(json!({ "queued": true })).into_response()
}

async fn doc_to_design(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let id = str_of(&b, "designId").unwrap_or("");
    let embeds = strings(b.get("embeds"));
    let generated: Mutex<Vec<AssetRef>> = Mutex::default();
    match st.svc.load_doc(id, &embeds, Some(&generated)).await {
        Ok(Some(mut doc)) => {
            doc.meta.surface = serde_json::from_value(json!("print")).expect("surface");
            let generated = generated.into_inner().unwrap_or_else(|e| e.into_inner());
            match (
                oc_schema::js::to_value(&doc),
                oc_schema::js::to_value(&generated),
            ) {
                (Ok(d), Ok(g)) => axum::Json(json!({ "doc": d, "generated": g })).into_response(),
                (Err(e), _) | (_, Err(e)) => SvcError::Other(e.to_string()).into_response(),
            }
        }
        Ok(None) => json_response(StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
        Err(e) => e.into_response(),
    }
}

async fn page_png(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let id = str_of(&b, "designId").unwrap_or("");
    let w = b.get("w").and_then(Value::as_f64).unwrap_or(800.0);
    let _g = st.svc.serial.lock().await;
    match st
        .svc
        .render_page(id, str_of(&b, "pageId"), oc_schema::js::round(w))
        .await
    {
        Ok(Some(r)) => {
            let mut res = Response::new(Body::from(r.png));
            res.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            res
        }
        Ok(None) => json_response(StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
        Err(e) => e.into_response(),
    }
}

/// Layout checks of some pages (AI assistants, `edit_design`): `{ designId, pageIds?, elementIds? }` → `{ pages: [{ pageId,
/// issues }] }`: the given pages plus the pages of the given elements; without either, the first page. At most 10.
async fn page_lint(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let id = str_of(&b, "designId").unwrap_or("");
    let _g = st.svc.serial.lock().await;
    let doc = match st.svc.load_doc(id, &[], None).await {
        Ok(Some(d)) => d,
        Ok(None) => return json_response(StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
        Err(e) => return e.into_response(),
    };
    let strs = |k: &str| -> Vec<String> { b.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default() };
    let mut wanted = strs("pageIds");
    // The pages of the given elements (what an edit touched).
    for el in strs("elementIds") {
        if let Some(p) = oc_schema::tree::page_of(&doc.pages, &doc.elements, &el)
            && !wanted.iter().any(|w| w == p)
        {
            wanted.push(p.to_owned());
        }
    }
    let ids: Vec<String> = if wanted.is_empty() {
        crate::service::pick_page(&doc, None).map(|p| p.id.to_string()).into_iter().collect()
    } else {
        wanted.into_iter().filter(|p| doc.pages.contains_key(p.as_str())).take(10).collect()
    };
    let mut pages = Vec::new();
    for page in ids {
        let issues = crate::lint::lint_page(st.svc.renderer.as_ref(), st.svc.as_ref(), &doc, &page).await;
        pages.push(json!({ "pageId": page, "issues": issues }));
    }
    json_response(StatusCode::OK, json!({ "pages": pages }))
}

async fn site_build(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let id = str_of(&b, "designId").unwrap_or("").to_string();
    let doc = match st.svc.load_doc(&id, &[], None).await {
        Ok(Some(d)) => d,
        Ok(None) => return json_response(StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
        Err(e) => return e.into_response(),
    };
    let origin = str_of(&b, "assetOrigin").unwrap_or("").to_string();
    let opts = SiteOptions {
        design_id: id.clone(),
        title: str_of(&b, "title").unwrap_or("").into(),
        description: str_of(&b, "description").map(str::to_string),
        noindex: b.get("noindex").is_some_and(oc_schema::tree::truthy),
        asset_url: Arc::new(move |a: &str| format!("{origin}/api/assets/{a}")),
        og_image: Some(format!(
            "{}/api/sites/og/{id}.png",
            st.svc.settings.web_origin
        )),
        badge: b.get("badge").filter(|v| v.is_object()).map(|v| {
            (
                str_of(v, "name").unwrap_or("").to_string(),
                str_of(v, "url").unwrap_or("").to_string(),
            )
        }),
    };
    let _g = st.svc.serial.lock().await;
    match build_site(st.svc.renderer.as_ref(), st.svc.as_ref(), &doc, &opts).await {
        Ok(html) => axum::Json(json!({ "html": html })).into_response(),
        Err(e) => SvcError::Renderer(e).into_response(),
    }
}

async fn template_thumbnail(
    State(st): State<AppState>,
    axum::Json(b): axum::Json<Value>,
) -> Response {
    let id = str_of(&b, "templateId").unwrap_or("");
    let svc = &st.svc;
    let _g = svc.serial.lock().await;
    let run = async {
        let row: Option<(sqlx::types::Json<Value>,)> =
            sqlx::query_as("SELECT doc FROM templates WHERE id = $1")
                .bind(id)
                .fetch_optional(&svc.pool)
                .await?;
        let Some((doc,)) = row else {
            return Ok::<_, SvcError>(false);
        };
        let doc: DesignDoc =
            serde_json::from_value(doc.0).map_err(|e| SvcError::Other(e.to_string()))?;
        let Some(page) = sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else {
            return Ok(false);
        };
        let png = svc
            .renderer
            .render_image(
                svc.as_ref(),
                &doc,
                &page,
                &RasterOptions::at_scale(thumb_scale(&doc)),
                ImageFormat::Png,
            )
            .await?;
        let sha = svc.put(&png, "image/png").await?;
        sqlx::query("UPDATE templates SET thumbnail = $2 WHERE id = $1")
            .bind(id)
            .bind(sha)
            .execute(&svc.pool)
            .await?;
        Ok(true)
    };
    match run.await {
        Ok(ok) => axum::Json(json!({ "ok": ok })).into_response(),
        Err(e) => e.into_response(),
    }
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

fn zip_of(files: Vec<(String, Vec<u8>)>, stored: bool) -> Result<Vec<u8>, SvcError> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let m = if stored {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    let opts = zip::write::SimpleFileOptions::default().compression_method(m);
    for (name, bytes) in files {
        zip.start_file(name, opts)
            .map_err(|e| SvcError::Other(e.to_string()))?;
        zip.write_all(&bytes)
            .map_err(|e| SvcError::Other(e.to_string()))?;
    }
    Ok(zip
        .finish()
        .map_err(|e| SvcError::Other(e.to_string()))?
        .into_inner())
}

/// A PDF of the pages: vector by default; `pdf-print` adds bleed and crop marks; `pdf-flat` is the raster fallback.
pub async fn export_pdf(
    svc: &Service,
    doc: &DesignDoc,
    pages: &[String],
    kind: &str,
    print: Option<PrintSetup>,
) -> Result<Vec<u8>, SvcError> {
    let producer = Some(svc.settings.brand_name.clone());
    if kind == "pdf-flat" {
        return Ok(svc
            .renderer
            .raster_pdf(
                svc,
                doc,
                pages,
                &RasterPdfOptions {
                    print: false,
                    producer,
                },
            )
            .await?);
    }
    let print = (kind == "pdf-print").then(|| {
        print.unwrap_or(PrintSetup {
            bleed_mm: 3.0,
            crop_marks: true,
            ..PrintSetup::default()
        })
    });
    Ok(svc
        .renderer
        .vector_pdf(
            svc,
            doc,
            pages,
            &VectorPdfOptions {
                title: None,
                producer,
                print,
            },
        )
        .await?)
}

/// A progress callback that forwards to the job row through a channel (throttled to one update per 300 ms and
/// capped at 0.99, like the TS `progress`); [`pump_progress`] writes them while the export runs.
fn progress_channel() -> (
    impl Fn(f64, &str) + Send + Sync,
    tokio::sync::mpsc::UnboundedReceiver<(f64, String)>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<(f64, String)>();
    let last: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let cb = move |k: f64, label: &str| {
        let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
        if l.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            return;
        }
        *l = Some(std::time::Instant::now());
        let _ = tx.send((k.min(0.99), label.to_string()));
    };
    (cb, rx)
}

async fn pump_progress(
    ctx: &oc_jobs::JobContext,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<(f64, String)>,
) {
    while let Some((k, label)) = rx.recv().await {
        let _ = ctx.progress(k, Some(&label)).await;
    }
}

#[derive(Debug, Deserialize)]
struct ExportBody {
    #[serde(rename = "designId")]
    design_id: String,
    format: String,
    #[serde(default)]
    pages: Option<Vec<i64>>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    quality: Option<f64>,
    #[serde(default)]
    transparent: Option<bool>,
    #[serde(default)]
    embeds: Option<Vec<String>>,
    /// `pdf-print` options ([`crate::print::PrintBody`]).
    #[serde(default)]
    print: Option<crate::print::PrintBody>,
    /// Text watermark on every page ([`crate::watermark::Watermark`]).
    #[serde(default)]
    watermark: Option<crate::watermark::Watermark>,
}

async fn export(State(st): State<AppState>, axum::Json(b): axum::Json<Value>) -> Response {
    let b: ExportBody = match serde_json::from_value(b) {
        Ok(b) => b,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "statusCode": 400, "error": "Bad Request", "message": e.to_string() }),
            );
        }
    };
    match export_inner(&st.svc, &b).await {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}

async fn export_inner(svc: &Service, b: &ExportBody) -> Result<Response, SvcError> {
    let embeds = b.embeds.clone().unwrap_or_default();
    // Docs → Word / Markdown straight from the flowing body (not the paginated layout).
    if b.format == "docx" || b.format == "md" {
        let Some((state, title)) = svc.doc_state(&b.design_id).await? else {
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "not_a_doc", "message": "Word and Markdown downloads are for docs." }),
            ));
        };
        let name = safe_name(&title);
        if b.format == "md" {
            return Ok(file(
                svc.markdown(&state)?.into_bytes(),
                "text/markdown; charset=utf-8",
                &format!("{name}.md"),
            ));
        }
        let _g = svc.serial.lock().await;
        return Ok(file(
            svc.docx(&state, &title, &embeds).await?,
            DOCX,
            &format!("{name}.docx"),
        ));
    }
    let Some(mut doc) = svc.load_doc(&b.design_id, &embeds, None).await? else {
        return Ok(json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "not_found" }),
        ));
    };
    if let Some(w) = &b.watermark {
        crate::watermark::apply(&mut doc, w);
    }
    let all: Vec<String> = sorted_pages(&doc.pages)
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    let pages: Vec<String> = match &b.pages {
        Some(ix) if !ix.is_empty() => ix
            .iter()
            .filter_map(|i| usize::try_from(*i).ok().and_then(|i| all.get(i).cloned()))
            .collect(),
        _ => all.clone(),
    };
    if pages.is_empty() {
        return Ok(json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "no_pages" }),
        ));
    }
    let name = safe_name(&doc.meta.title);
    let _g = svc.serial.lock().await;
    let fmt = b.format.as_str();
    match fmt {
        "pdf" | "pdf-print" | "pdf-flat" => {
            let print = match (&b.print, fmt) {
                (Some(p), "pdf-print") => match crate::print::resolve(svc, p).await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        return Ok(json_response(
                            StatusCode::BAD_REQUEST,
                            json!({ "error": "bad_print_options", "message": e.to_string() }),
                        ));
                    }
                },
                _ => None,
            };
            return Ok(file(
                export_pdf(svc, &doc, &pages, fmt, print).await?,
                "application/pdf",
                &format!("{name}.pdf"),
            ));
        }
        "mp4" => {
            let media = svc.media_for(&doc).await?;
            let bytes = export_mp4(
                svc.video_deps(&media),
                &doc,
                VideoQuality::P1080,
                None,
                None,
            )
            .await?;
            return Ok(file(bytes, "video/mp4", &format!("{name}.mp4")));
        }
        "gif" => {
            let media = svc.media_for(&doc).await?;
            let bytes = export_gif(svc.video_deps(&media), &doc, None).await?;
            return Ok(file(bytes, "image/gif", &format!("{name}.gif")));
        }
        "m4a" => {
            let media = svc.media_for(&doc).await?;
            return Ok(match export_audio(svc.video_deps(&media), &doc).await? {
                Some(bytes) => file(bytes, "audio/mp4", &format!("{name}.m4a")),
                None => json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": "no_audio", "message": "This design has no audio." }),
                ),
            });
        }
        "pptx" => {
            return Ok(file(
                crate::pptx::export_pptx(svc.renderer.as_ref(), svc, &doc, &pages, svc)
                    .await
                    .map_err(pptx_err)?,
                PPTX,
                &format!("{name}.pptx"),
            ));
        }
        "svg" => {
            let opts = SvgOptions::default();
            if pages.len() == 1 {
                return Ok(file(
                    svc.renderer
                        .page_svg(svc, &doc, &pages[0], &opts)
                        .await?
                        .into_bytes(),
                    "image/svg+xml",
                    &format!("{name}.svg"),
                ));
            }
            let mut files = Vec::new();
            for (i, p) in pages.iter().enumerate() {
                files.push((
                    format!("{name}-{:02}.svg", i + 1),
                    svc.renderer
                        .page_svg(svc, &doc, p, &opts)
                        .await?
                        .into_bytes(),
                ));
            }
            return Ok(file(
                zip_of(files, false)?,
                "application/zip",
                &format!("{name}.zip"),
            ));
        }
        _ => {}
    }
    let jpeg = fmt == "jpg";
    let ext = if jpeg { "jpg" } else { "png" };
    let format = if jpeg {
        ImageFormat::Jpeg {
            quality: b.quality.unwrap_or(92.0).clamp(0.0, 100.0) as u8,
        }
    } else {
        ImageFormat::Png
    };
    let opts = RasterOptions {
        scale: b.scale.unwrap_or(1.0),
        transparent: b.transparent.unwrap_or(false) && !jpeg,
        ..RasterOptions::default()
    };
    if pages.len() == 1 && fmt != "zip" {
        let bytes = svc
            .renderer
            .render_image(svc, &doc, &pages[0], &opts, format)
            .await?;
        return Ok(file(
            bytes,
            if jpeg { "image/jpeg" } else { "image/png" },
            &format!("{name}.{ext}"),
        ));
    }
    let mut files = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        let n = all.iter().position(|x| x == p).map_or(i + 1, |k| k + 1);
        files.push((
            format!("{name}-{n:02}.{ext}"),
            svc.renderer
                .render_image(svc, &doc, p, &opts, format)
                .await?,
        ));
    }
    Ok(file(
        zip_of(files, true)?,
        "application/zip",
        &format!("{name}.zip"),
    ))
}

fn pptx_err(e: crate::pptx::PptxError) -> SvcError {
    match e {
        crate::pptx::PptxError::Renderer(r) => SvcError::Renderer(r),
        other => SvcError::Other(other.to_string()),
    }
}

/// The `export` job payload.
#[derive(Debug, Deserialize)]
pub struct ExportJob {
    #[serde(rename = "designId")]
    pub design_id: String,
    #[serde(rename = "userId", default)]
    pub user_id: String,
    pub format: String,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub embeds: Option<Vec<String>>,
    #[serde(default)]
    pub print: Option<JobPrint>,
}

/// The job's `print` field: the same options as the sync export's.
pub type JobPrint = crate::print::PrintBody;

/// Background exports (PL-032): the file goes to the object store; returns `{ sha, type, filename, size }`.
pub async fn run_export(
    svc: &Service,
    b: &ExportJob,
    ctx: &oc_jobs::JobContext,
) -> Result<Value, SvcError> {
    let embeds = b.embeds.clone().unwrap_or_default();
    let done = |sha: String, ty: &str, filename: String, size: usize| {
        let data = json!({
            "designId": b.design_id, "jobId": ctx.job.id, "format": b.format, "filename": filename, "size": size,
            "downloadUrl": format!("{}/v1/exports/{}/file", svc.settings.web_origin, ctx.job.id),
        });
        if let Ok(user) = uuid::Uuid::parse_str(&b.user_id) {
            let pool = svc.pool.clone();
            tokio::spawn(async move {
                let _ = oc_webhooks::emit_event(
                    &pool,
                    user,
                    oc_webhooks::WebhookEvent::ExportCompleted,
                    &data,
                )
                .await;
            });
        }
        json!({ "sha": sha, "type": ty, "filename": filename, "size": size })
    };
    let _g = svc.serial.lock().await;
    if b.format == "docx" {
        let Some((state, title)) = svc.doc_state(&b.design_id).await? else {
            return Err(SvcError::Other("Word downloads are for docs.".into()));
        };
        let _ = ctx.progress(0.0, Some("Preparing")).await;
        let bytes = svc.docx(&state, &title, &embeds).await?;
        let sha = svc.put(&bytes, DOCX).await?;
        return Ok(done(
            sha,
            DOCX,
            format!("{}.docx", safe_name(&title)),
            bytes.len(),
        ));
    }
    let Some(doc) = svc.load_doc(&b.design_id, &embeds, None).await? else {
        return Err(SvcError::Other("design not found".into()));
    };
    let name = safe_name(&doc.meta.title);
    let _ = ctx.progress(0.0, Some("Preparing")).await;
    let pages: Vec<String> = sorted_pages(&doc.pages)
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    let (bytes, ty, ext) = match b.format.as_str() {
        "mp4" | "gif" => {
            let media = svc.media_for(&doc).await?;
            let deps = svc.video_deps(&media);
            let mp4 = b.format == "mp4";
            let quality = VideoQuality::parse(b.quality.as_deref().unwrap_or("1080p"));
            let (cb, rx) = progress_channel();
            let d = &doc;
            // The callback (and with it the channel's sender) is dropped when the export ends, which ends the pump.
            let work = async move {
                if mp4 {
                    export_mp4(deps, d, quality, None, Some(&cb)).await
                } else {
                    export_gif(deps, d, Some(&cb)).await
                }
            };
            let (bytes, ()) = tokio::join!(work, pump_progress(ctx, rx));
            let bytes = bytes?;
            if mp4 {
                (bytes, "video/mp4", "mp4")
            } else {
                (bytes, "image/gif", "gif")
            }
        }
        "m4a" => {
            let media = svc.media_for(&doc).await?;
            let Some(bytes) = export_audio(svc.video_deps(&media), &doc).await? else {
                return Err(SvcError::Other("This design has no audio.".into()));
            };
            (bytes, "audio/mp4", "m4a")
        }
        "pptx" => (
            crate::pptx::export_pptx(svc.renderer.as_ref(), svc, &doc, &pages, svc)
                .await
                .map_err(pptx_err)?,
            PPTX,
            "pptx",
        ),
        kind => {
            let print = match (&b.print, kind) {
                (Some(p), "pdf-print") => Some(crate::print::resolve(svc, p).await?),
                _ => None,
            };
            (
                export_pdf(svc, &doc, &pages, kind, print).await?,
                "application/pdf",
                "pdf",
            )
        }
    };
    let sha = svc.put(&bytes, ty).await?;
    Ok(done(sha, ty, format!("{name}.{ext}"), bytes.len()))
}
