// SPDX-License-Identifier: Apache-2.0
//! The service's state and data access — port of the non-route half of `services/render/src/main.ts` and of
//! `docExport.ts`: asset bytes (with the sha cache, posters, recoloured SVGs and pictures generated during an
//! export), uploaded fonts, document loading (Yjs state via oc-crdt; Docs laid out onto pages), and the Docs embeds/charts/Word/Markdown wiring.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oc_crdt::doc_body::{DOC_FIELD, PmNode, fragment_to_json};
use oc_gfonts::GoogleFonts;
use oc_gfonts::manifest::{FontFace, FontFamily, FontManifest};
use oc_schema::design::AssetRef;
use oc_schema::doc_blocks::{DocChartData, doc_chart_element};
use oc_schema::elements::Element;
use oc_schema::factory::{MetaInit, empty_design, place_all_elements, xf};
use oc_schema::svg::{parse_recolor_key, recolor_svg};
use oc_schema::tree::sorted_pages;
use oc_schema::{DesignDoc, Page};
use oc_storage::ObjectStore;
use regex::Regex;
use sqlx::PgPool;
use yrs::{Transact, WriteTxn};

use crate::doc_layout::{DocLayoutDeps, LayoutError, doc_to_design};
use crate::doc_markdown::{MarkdownOptions, doc_to_markdown};
use crate::docx::{
    DepError, DepResult, DocxDeps, DocxError, DocxImage, DocxMeta, ImageType, export_docx,
};
use crate::renderer::{
    ImageFormat, RasterOptions, RenderError, RenderFuture, RenderHost, Renderer, Rgba,
};

/// Service settings (`@oc/config` values the render service reads).
#[derive(Debug, Clone)]
pub struct Settings {
    pub web_origin: String,
    pub usercontent_origin: String,
    pub brand_name: String,
    pub fonts_dir: PathBuf,
    /// Bundled print colour profiles (`pnpm icc:sync`; `OC_ICC_DIR` overrides `fixtures/icc`).
    pub icc_dir: PathBuf,
}

impl Settings {
    pub fn from_config(cfg: &oc_config::Config) -> Settings {
        Settings {
            web_origin: cfg.web_origin.clone(),
            usercontent_origin: cfg.usercontent_origin.clone(),
            brand_name: cfg.brand.name.to_string(),
            fonts_dir: oc_config::paths().fonts.clone(),
            icc_dir: icc_dir(),
        }
    }
}

/// `OC_ICC_DIR`, else `<repo>/fixtures/icc`.
pub fn icc_dir() -> PathBuf {
    std::env::var_os("OC_ICC_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| oc_config::paths().root.join("fixtures/icc"))
}

/// Errors of the service layer.
#[derive(Debug, thiserror::Error)]
pub enum SvcError {
    #[error(transparent)]
    Renderer(#[from] RenderError),
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("storage: {0}")]
    Storage(String),
    #[error("{0}")]
    Other(String),
    #[error("Sheets are no longer supported, so this design can't be exported.")]
    SheetUnsupported,
}

impl From<LayoutError> for SvcError {
    fn from(e: LayoutError) -> Self {
        match e {
            LayoutError::Renderer(r) => SvcError::Renderer(r),
            LayoutError::Invalid(s) => SvcError::Other(s),
        }
    }
}

impl From<DocxError> for SvcError {
    fn from(e: DocxError) -> Self {
        match e {
            DocxError::Renderer(r) => SvcError::Renderer(r),
            other => SvcError::Other(other.to_string()),
        }
    }
}

/// The render service.
pub struct Service {
    pub pool: PgPool,
    pub objects: ObjectStore,
    pub renderer: Arc<dyn Renderer>,
    pub fonts: GoogleFonts,
    pub manifest: FontManifest,
    pub settings: Settings,
    sha_cache: Mutex<HashMap<String, String>>,
    /// Rendered pictures (embedded pages) registered as synthetic assets for the duration of an export.
    generated: Mutex<IndexMap<String, Vec<u8>>>,
    /// Renders are serialised (one painter, memory-heavy rasters), like the TS `serial` queue.
    pub serial: tokio::sync::Mutex<()>,
    /// Starts ffmpeg for video/audio exports.
    pub spawner: Arc<dyn crate::media::Spawner>,
    /// Temporary files of video/audio exports (`os.tmpdir()`).
    pub tmp: PathBuf,
}

static CUSTOM_FONT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^custom/([\w-]+)\.\w+$").expect("regex"));
static ASSET_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/api/assets/([\w-]+)").expect("regex"));
static NAME_DROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_\- ]+").expect("regex"));

/// `safeName`: a file-name-safe title (≤ 80 chars), `design` when nothing is left.
pub fn safe_name(s: &str) -> String {
    let kept = NAME_DROP.replace_all(s, "");
    let t: String = oc_schema::js::trim(&kept).chars().take(80).collect();
    if t.is_empty() { "design".into() } else { t }
}

/// A page by id, or by 1-based number (`"2"`); the first page otherwise.
pub fn pick_page<'a>(doc: &'a DesignDoc, page_id: Option<&str>) -> Option<&'a Page> {
    let pages = sorted_pages(&doc.pages);
    match page_id.filter(|p| !p.is_empty()) {
        Some(id) => {
            if let Some(p) = pages.iter().find(|p| *p.id == *id) {
                return Some(p);
            }
            if id.bytes().all(|b| b.is_ascii_digit()) {
                return id
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|i| pages.get(i).copied());
            }
            None
        }
        None => pages.first().copied(),
    }
}

/// The doc body JSON of a persisted Yjs state (`docBodyOf`).
pub fn doc_body_of(state: &[u8]) -> Result<PmNode, SvcError> {
    let ydoc = oc_crdt::codec::new_doc();
    oc_crdt::codec::apply_update(&ydoc, state, None).map_err(|e| SvcError::Other(e.to_string()))?;
    let mut txn = ydoc.transact_mut();
    let frag = txn.get_or_insert_xml_fragment(DOC_FIELD);
    Ok(fragment_to_json(&txn, &frag))
}

fn storage(e: impl std::fmt::Display) -> SvcError {
    SvcError::Storage(e.to_string())
}

fn render_err(e: SvcError) -> RenderError {
    match e {
        SvcError::Renderer(r) => r,
        other => RenderError::Failed(other.to_string()),
    }
}

/// A page rendered to PNG at a pixel width.
pub struct PagePng {
    pub png: Vec<u8>,
    pub w: f64,
    pub h: f64,
}

impl Service {
    pub fn new(
        pool: PgPool,
        objects: ObjectStore,
        renderer: Arc<dyn Renderer>,
        fonts: GoogleFonts,
        manifest: FontManifest,
        settings: Settings,
    ) -> Service {
        Service {
            pool,
            objects,
            renderer,
            fonts,
            manifest,
            settings,
            sha_cache: Mutex::default(),
            generated: Mutex::default(),
            serial: tokio::sync::Mutex::new(()),
            spawner: Arc::new(crate::media::SystemSpawner),
            tmp: std::env::temp_dir(),
        }
    }

    /// What the video/audio exporters need, over this service.
    pub fn video_deps<'a>(&'a self, media: &'a crate::media::Media) -> crate::video::VideoDeps<'a> {
        crate::video::VideoDeps {
            renderer: self.renderer.as_ref(),
            host: self,
            media,
            spawner: self.spawner.as_ref(),
            tmp: &self.tmp,
        }
    }

    fn remember(&self, id: &str, bytes: Vec<u8>) {
        let mut g = self.generated.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(id.to_string(), bytes);
        while g.len() > 64 {
            g.shift_remove_index(0);
        }
    }

    /// `objects().put(bytes, mime)`: the sha256 the bytes are stored under.
    pub async fn put(&self, bytes: &[u8], mime: &str) -> Result<String, SvcError> {
        self.objects.put(bytes, Some(mime)).await.map_err(storage)
    }

    /// Asset bytes by key; `<id>/poster` is a video's poster frame, `<id>/recolor/<pairs>` a recoloured SVG.
    pub async fn asset(&self, key: &str) -> Result<Vec<u8>, SvcError> {
        if let Some(b) = self
            .generated
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
        {
            return Ok(b);
        }
        if let Some(rk) = parse_recolor_key(key) {
            return self.recolored_svg_raster(&rk.asset, &rk.map).await;
        }
        let cached = self
            .sha_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned();
        let sha = match cached {
            Some(s) => s,
            None => {
                let mut parts = key.split('/');
                let id = parts.next().unwrap_or("");
                let variant = parts.next();
                let row: Option<(String, Option<String>)> =
                    sqlx::query_as("SELECT sha256, poster_sha256 FROM assets WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&self.pool)
                        .await?;
                let Some((sha, poster)) = row else {
                    return Err(SvcError::Other(format!("asset {key} not found")));
                };
                let sha = if variant == Some("poster") {
                    poster.unwrap_or_default()
                } else {
                    sha
                };
                if sha.is_empty() {
                    return Err(SvcError::Other(format!(
                        "asset {key} has no {}",
                        variant.unwrap_or("")
                    )));
                }
                self.sha_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.to_string(), sha.clone());
                sha
            }
        };
        self.objects.get(&sha).await.map_err(storage)
    }

    /// Rasterise an SVG asset's stored source with a colour map, at the width of its original raster.
    async fn recolored_svg_raster(
        &self,
        id: &str,
        map: &IndexMap<String, String>,
    ) -> Result<Vec<u8>, SvcError> {
        let row: Option<(String, Option<String>, i32)> =
            sqlx::query_as("SELECT sha256, svg_sha256, width FROM assets WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let Some((sha, svg_sha, width)) = row else {
            return Err(SvcError::Other(format!("asset {id} not found")));
        };
        let Some(svg_sha) = svg_sha else {
            return self.objects.get(&sha).await.map_err(storage);
        };
        let src = String::from_utf8_lossy(&self.objects.get(&svg_sha).await.map_err(storage)?)
            .into_owned();
        let svg = recolor_svg(&src, map);
        let w = if width == 0 { 1024 } else { width };
        Ok(self
            .renderer
            .rasterize_svg(&svg, w.clamp(16, 4096) as u32)
            .await?)
    }

    /// `assetRef(id)`.
    pub async fn asset_ref(&self, id: &str) -> Result<Option<AssetRef>, SvcError> {
        type Row = (String, String, String, String, i32, i32, Option<String>);
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, sha256, mime, kind, width, height, name FROM assets WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, sha256, mime, kind, w, h, name)| AssetRef {
            id,
            kind: serde_json::from_value(serde_json::Value::String(kind)).unwrap_or_else(|_| {
                serde_json::from_value(serde_json::json!("image")).expect("kind")
            }),
            sha256,
            mime,
            w: f64::from(w),
            h: f64::from(h),
            name,
            animated: None,
            attribution: None,
            duration_ms: None,
            has_audio: None,
            extra: Default::default(),
        }))
    }

    /// `mediaFor(doc)`: playback proxies (H.264/AAC, or VP9 WebM with alpha) of every media asset the design
    /// references — video slots of live elements and soundtrack clips.
    pub async fn media_for(&self, doc: &DesignDoc) -> Result<crate::media::Media, SvcError> {
        let mut ids: Vec<String> = Vec::new();
        for e in doc.elements.values() {
            if e.is_deleted() {
                continue;
            }
            for v in oc_schema::timeline::video_slots(e) {
                if !ids.iter().any(|x| x == v.asset) {
                    ids.push(v.asset.to_string());
                }
            }
        }
        for c in doc.meta.audio.iter().flatten() {
            if !ids.iter().any(|x| *x == *c.asset) {
                ids.push(c.asset.to_string());
            }
        }
        let mut map = crate::media::Media::new();
        if ids.is_empty() {
            return Ok(map);
        }
        type Row = (
            String,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<i32>,
            bool,
            Option<i32>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, proxy_sha256, proxy_mime, width, height, has_audio, duration_ms FROM assets WHERE id = ANY($1::text[])",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        for (id, proxy, mime, w, h, has_audio, duration) in rows {
            let Some(proxy) = proxy else { continue };
            let file = self.objects.file(&proxy).await.map_err(storage)?;
            let dim = |v: Option<i32>| match v {
                Some(n) if n != 0 => n.max(0) as u32,
                _ => 2,
            };
            map.insert(
                id,
                crate::media::MediaInfo {
                    file: file.to_string_lossy().into_owned(),
                    w: dim(w),
                    h: dim(h),
                    has_audio,
                    duration_ms: f64::from(duration.unwrap_or(0)),
                    alpha: mime.as_deref() == Some("video/webm"),
                },
            );
        }
        Ok(map)
    }

    /// Uploaded brand fonts, by family name.
    pub async fn custom_families(&self, families: &[String]) -> Result<Vec<FontFamily>, SvcError> {
        let rows: Vec<(String, String, i32, bool, String, i32, String)> = sqlx::query_as(
            "SELECT DISTINCT ON (family, weight, italic) id, family, weight, italic, sha256, bytes, format FROM custom_fonts WHERE family = ANY($1) ORDER BY family, weight, italic, created_at",
        )
        .bind(families)
        .fetch_all(&self.pool)
        .await?;
        let mut by: IndexMap<String, FontFamily> = IndexMap::new();
        for (id, family, weight, italic, sha256, bytes, format) in rows {
            let f = by.entry(family.clone()).or_insert_with(|| FontFamily {
                family: family.clone(),
                slug: format!("custom-{id}"),
                category: "uploaded".into(),
                license: "Uploaded".into(),
                source: None,
                faces: vec![],
            });
            f.faces.push(FontFace {
                weight: weight as u16,
                italic,
                file: format!("custom/{id}.{format}"),
                sha256,
                bytes: bytes as u64,
            });
        }
        Ok(by.into_values().collect())
    }

    async fn custom_font_file(&self, file: &str) -> Result<Vec<u8>, SvcError> {
        let id = CUSTOM_FONT
            .captures(file)
            .and_then(|m| m.get(1))
            .map(|m| m.as_str().to_string());
        let row: Option<(String,)> = match id {
            Some(id) => {
                sqlx::query_as("SELECT sha256 FROM custom_fonts WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            None => None,
        };
        let Some((sha,)) = row else {
            return Err(SvcError::Other(format!("font {file} not found")));
        };
        self.objects.get(&sha).await.map_err(storage)
    }

    /// The Yjs state of a design: the stored snapshot plus the edits logged since (`doc_updates` newer than the
    /// snapshot; the collab server folds them in after a short debounce, edits made while it was unreachable wait in
    /// the log until it next loads the design). One statement, so the snapshot and its tail are read consistently.
    pub async fn state(&self, design_id: &str) -> Result<Option<Vec<u8>>, SvcError> {
        let row: Option<(Vec<u8>, Vec<Vec<u8>>)> = sqlx::query_as(
            "SELECT s.state, ARRAY(SELECT u.update FROM doc_updates u WHERE u.design_id = s.design_id AND u.seq > s.seq ORDER BY u.seq)
             FROM doc_state s WHERE s.design_id = $1",
        )
        .bind(design_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((state, tail)) = row else { return Ok(None) };
        if tail.is_empty() {
            return Ok(Some(state));
        }
        let ydoc = oc_crdt::new_doc();
        for u in std::iter::once(&state).chain(tail.iter()) {
            oc_crdt::apply_update(&ydoc, u, None).map_err(|e| SvcError::Other(e.to_string()))?;
        }
        Ok(Some(oc_crdt::encode_state(&ydoc)))
    }

    /// `docState(designId)`: a Docs design's state and title (`None` for other surfaces).
    pub async fn doc_state(&self, design_id: &str) -> Result<Option<(Vec<u8>, String)>, SvcError> {
        let row: Option<(Vec<u8>, String, String)> =
            sqlx::query_as("SELECT s.state, d.title, d.surface FROM doc_state s JOIN designs d ON d.id = s.design_id WHERE s.design_id = $1").bind(design_id).fetch_optional(&self.pool).await?;
        Ok(row.filter(|r| r.2 == "doc").map(|r| (r.0, r.1)))
    }

    /// `loadDoc(designId, { embeds, generated })`: the design to draw. Docs are laid out onto pages; sheets can no
    /// longer be drawn.
    pub fn load_doc<'a>(
        &'a self,
        design_id: &'a str,
        embeds: &'a [String],
        generated: Option<&'a Mutex<Vec<AssetRef>>>,
    ) -> BoxFuture<'a, Result<Option<DesignDoc>, SvcError>> {
        Box::pin(async move {
            let Some(state) = self.state(design_id).await? else {
                return Ok(None);
            };
            let doc =
                oc_crdt::decode_doc(&state).map_err(|e| SvcError::Other(e.to_string()))?;
            match doc.meta.surface.as_str() {
                "doc" => {
                    let body = doc_body_of(&state)?;
                    let deps = LayoutDeps {
                        svc: self,
                        allowed: embeds,
                        generated,
                    };
                    let d = doc_to_design(
                        self.renderer.as_ref(),
                        self,
                        &body,
                        &doc.meta.title,
                        doc.meta.dims.w,
                        doc.meta.dims.h,
                        &deps,
                    )
                    .await?;
                    return Ok(Some(d));
                }
                "sheet" => return Err(SvcError::SheetUnsupported),
                _ => {}
            }
            Ok(Some(doc))
        })
    }

    /// `renderPage`: PNG of a design page at a target pixel width (≤ 2400).
    pub async fn render_page(
        &self,
        design_id: &str,
        page_id: Option<&str>,
        width_px: f64,
    ) -> Result<Option<PagePng>, SvcError> {
        let Some(doc) = self.load_doc(design_id, &[], None).await? else {
            return Ok(None);
        };
        let Some(page) = pick_page(&doc, page_id) else {
            return Ok(None);
        };
        let (pw, ph) = (doc.meta.dims.w, doc.meta.dims.h);
        let scale = (2400f64.min(width_px.max(16.0)) / pw).clamp(0.05, 4.0);
        let png = self
            .renderer
            .render_image(
                self,
                &doc,
                &page.id,
                &RasterOptions::at_scale(scale),
                ImageFormat::Png,
            )
            .await?;
        Ok(Some(PagePng {
            png,
            w: oc_schema::js::round(pw * scale),
            h: oc_schema::js::round(ph * scale),
        }))
    }

    /// A page at moments of its playback (page time, ms) as one PNG: a single frame, or a filmstrip of up to 12
    /// frames, four to a row, each `width_px` wide. Animations, element timing and captions are drawn as in the video;
    /// videos show their poster. Times past the page's length show its last moment.
    pub async fn render_page_at(
        &self,
        design_id: &str,
        page_id: Option<&str>,
        width_px: f64,
        times_ms: &[f64],
    ) -> Result<Option<PagePng>, SvcError> {
        let Some(doc) = self.load_doc(design_id, &[], None).await? else {
            return Ok(None);
        };
        let Some(page) = pick_page(&doc, page_id) else {
            return Ok(None);
        };
        let tl = oc_schema::timeline::timeline_of(&doc, true);
        let (start, len) = tl
            .spans
            .iter()
            .find(|s| s.page.id == page.id)
            .map_or((0.0, oc_schema::timeline::DEFAULT_PAGE_MS), |s| (s.start_ms, s.duration_ms));
        let pw = doc.meta.dims.w;
        let scale = (2400f64.min(width_px.max(16.0)) / pw).clamp(0.05, 4.0);
        let mut frames = Vec::new();
        for &t in times_ms.iter().take(12) {
            // The last frame of a page is just before the next page starts.
            let t = t.clamp(0.0, (len - 1.0).max(0.0));
            let opts = RasterOptions {
                scale,
                time_ms: Some(t),
                caption_time_ms: Some(start + t),
                ..RasterOptions::default()
            };
            frames.push(self.renderer.render_rgba(self, &doc, &page.id, &opts).await?);
        }
        let Some(first) = frames.first() else {
            return Ok(None);
        };
        let (fw, fh) = (first.width, first.height);
        let sheet = if frames.len() == 1 {
            frames.pop().expect("one frame")
        } else {
            filmstrip(&frames, fw, fh)
        };
        let (w, h) = (f64::from(sheet.width), f64::from(sheet.height));
        let png = self.renderer.encode_png(&sheet).await?;
        Ok(Some(PagePng { png, w, h }))
    }

    /// A chart block rendered as PNG (2×).
    pub async fn chart_png(
        &self,
        data: &DocChartData,
        w: f64,
        h: f64,
    ) -> Result<Vec<u8>, SvcError> {
        let mut d = empty_design(&MetaInit {
            w,
            h,
            pages: Some(1),
            title: Some("chart".into()),
            ..MetaInit::default()
        });
        let page = sorted_pages(&d.pages)[0].id.to_string();
        let mut els = vec![Element::Chart(doc_chart_element(
            data,
            xf(8.0, 8.0, w - 16.0, h - 16.0, 0.0),
        ))];
        place_all_elements(&page, &mut els, None);
        for e in els {
            d.elements.insert(e.id().unwrap_or("").to_string(), e);
        }
        Ok(self
            .renderer
            .render_image(
                self,
                &d,
                &page,
                &RasterOptions::at_scale(2.0),
                ImageFormat::Png,
            )
            .await?)
    }

    /// Docs → Word.
    pub async fn docx(
        &self,
        state: &[u8],
        title: &str,
        allowed: &[String],
    ) -> Result<Vec<u8>, SvcError> {
        let body = doc_body_of(state)?;
        let mut deps = WordDeps { svc: self, allowed };
        Ok(export_docx(
            &body,
            &DocxMeta {
                title: title.into(),
                font: None,
            },
            &mut deps,
        )
        .await?)
    }

    /// Docs → Markdown.
    pub fn markdown(&self, state: &[u8]) -> Result<String, SvcError> {
        Ok(doc_to_markdown(
            &doc_body_of(state)?,
            &MarkdownOptions {
                origin: Some(self.settings.web_origin.clone()),
            },
        ))
    }
}

impl RenderHost for Service {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move { self.asset(key).await.map_err(render_err) })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            if file.starts_with("custom/") {
                return self.custom_font_file(file).await.map_err(render_err);
            }
            self.fonts
                .load(file, None)
                .await
                .map_err(|e| RenderError::Failed(e.to_string()))
        })
    }
    fn custom_families<'a>(&'a self, families: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>> {
        Box::pin(async move {
            Service::custom_families(self, families)
                .await
                .map_err(render_err)
        })
    }
    fn manifest(&self) -> &FontManifest {
        &self.manifest
    }
    fn fonts_dir(&self) -> PathBuf {
        self.settings.fonts_dir.clone()
    }
}

impl crate::pptx::PptxDeps for Service {
    fn asset_bytes<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, crate::pptx::AssetError>> {
        Box::pin(async move {
            self.asset(key).await.map_err(|e| match e {
                SvcError::Renderer(r @ RenderError::NotImplemented(_)) => {
                    crate::pptx::AssetError::Fatal(r)
                }
                other => crate::pptx::AssetError::Missing(other.to_string()),
            })
        })
    }
}

/// docLayout deps: assets from the database; embedded pages rendered for allowed designs only.
struct LayoutDeps<'a> {
    svc: &'a Service,
    allowed: &'a [String],
    generated: Option<&'a Mutex<Vec<AssetRef>>>,
}

impl DocLayoutDeps for LayoutDeps<'_> {
    fn asset<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<AssetRef>> {
        Box::pin(async move { self.svc.asset_ref(id).await.ok().flatten() })
    }
    fn embed<'a>(
        &'a self,
        design_id: &'a str,
        page_id: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, Option<AssetRef>> {
        Box::pin(async move {
            if !self.allowed.iter().any(|a| a == design_id) {
                return None;
            }
            let r = self
                .svc
                .render_page(design_id, page_id, width_px)
                .await
                .ok()
                .flatten()?;
            let id = oc_schema::ids::new_id();
            let sha = self.svc.put(&r.png, "image/png").await.ok()?;
            self.svc.remember(&id, r.png);
            let a = AssetRef {
                id,
                kind: serde_json::from_value(serde_json::json!("image")).expect("kind"),
                sha256: sha,
                mime: "image/png".into(),
                w: r.w,
                h: r.h,
                name: Some("Embedded design.png".into()),
                animated: None,
                attribution: None,
                duration_ms: None,
                has_audio: None,
                extra: Default::default(),
            };
            if let Some(g) = self.generated {
                g.lock().unwrap_or_else(|e| e.into_inner()).push(a.clone());
            }
            Some(a)
        })
    }
}

/// Word deps: image assets (transcoded when Word can't read them), embedded pages, charts.
struct WordDeps<'a> {
    svc: &'a Service,
    allowed: &'a [String],
}

fn dep_err(e: SvcError) -> DepError {
    match e {
        SvcError::Renderer(r @ RenderError::NotImplemented(_)) => DepError::Fatal(r),
        other => DepError::Failed(other.to_string()),
    }
}

impl DocxDeps for WordDeps<'_> {
    fn image<'a>(&'a mut self, src: &'a str) -> BoxFuture<'a, DepResult> {
        Box::pin(async move {
            let Some(id) = ASSET_URL
                .captures(src)
                .and_then(|m| m.get(1))
                .map(|m| m.as_str().to_string())
            else {
                return Ok(None);
            };
            let Some(a) = self.svc.asset_ref(&id).await.map_err(dep_err)? else {
                return Ok(None);
            };
            let bytes = self.svc.asset(&a.id).await.map_err(dep_err)?;
            let kind = match a.mime.as_str() {
                "image/png" => Some(ImageType::Png),
                "image/jpeg" => Some(ImageType::Jpg),
                "image/gif" => Some(ImageType::Gif),
                _ => None,
            };
            if let Some(kind) = kind {
                return Ok(Some(DocxImage {
                    data: bytes,
                    kind,
                    w: a.w,
                    h: a.h,
                }));
            }
            let t = self
                .svc
                .renderer
                .transcode_png(&bytes)
                .await
                .map_err(|e| dep_err(e.into()))?;
            Ok(t.map(|d| DocxImage {
                data: d.png,
                kind: ImageType::Png,
                w: f64::from(d.width),
                h: f64::from(d.height),
            }))
        })
    }
    fn embed<'a>(
        &'a mut self,
        design_id: &'a str,
        page_id: Option<&'a str>,
        width_px: f64,
    ) -> BoxFuture<'a, DepResult> {
        Box::pin(async move {
            if !self.allowed.iter().any(|a| a == design_id) {
                return Ok(None);
            }
            let r = self
                .svc
                .render_page(design_id, page_id, width_px)
                .await
                .map_err(dep_err)?;
            Ok(r.map(|r| DocxImage {
                data: r.png,
                kind: ImageType::Png,
                w: r.w,
                h: r.h,
            }))
        })
    }
    fn chart<'a>(&'a mut self, data: &'a DocChartData, w: f64, h: f64) -> BoxFuture<'a, DepResult> {
        Box::pin(async move {
            let png = self.svc.chart_png(data, w, h).await.map_err(dep_err)?;
            Ok(Some(DocxImage {
                data: png,
                kind: ImageType::Png,
                w: w * 2.0,
                h: h * 2.0,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(safe_name("  Q3 report: final!  "), "Q3 report final");
        assert_eq!(safe_name("日本"), "design");
    }
}

/// Frames of one size laid out four to a row on a grey sheet, 8 px apart, in reading order.
pub fn filmstrip(frames: &[Rgba], fw: u32, fh: u32) -> Rgba {
    const GAP: u32 = 8;
    let n = frames.len() as u32;
    let cols = n.clamp(1, 4);
    let rows = n.div_ceil(cols);
    let (w, h) = (cols * fw + (cols + 1) * GAP, rows * fh + (rows + 1) * GAP);
    let mut pixels = [0x9a, 0x9a, 0x9a, 0xff].repeat((w * h) as usize);
    for (i, f) in frames.iter().enumerate() {
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        let (x0, y0) = (GAP + c * (fw + GAP), GAP + r * (fh + GAP));
        let (cw, ch) = (f.width.min(fw), f.height.min(fh));
        for y in 0..ch {
            let src = (y * f.width * 4) as usize;
            let dst = (((y0 + y) * w + x0) * 4) as usize;
            pixels[dst..dst + (cw * 4) as usize].copy_from_slice(&f.pixels[src..src + (cw * 4) as usize]);
        }
    }
    Rgba { width: w, height: h, pixels }
}

#[cfg(test)]
mod filmstrip_tests {
    use super::*;

    #[test]
    fn lays_frames_out_four_to_a_row() {
        let frame = |v: u8| Rgba { width: 2, height: 3, pixels: vec![v; 2 * 3 * 4] };
        let s = filmstrip(&[frame(1), frame(2), frame(3), frame(4), frame(5)], 2, 3);
        assert_eq!((s.width, s.height), (4 * 2 + 5 * 8, 2 * 3 + 3 * 8));
        let at = |x: u32, y: u32| s.pixels[((y * s.width + x) * 4) as usize];
        assert_eq!(at(8, 8), 1);
        assert_eq!(at(8 + 3 * 10, 8), 4);
        assert_eq!(at(8, 8 + 3 + 8), 5);
        assert_eq!(at(0, 0), 0x9a);
    }
}
