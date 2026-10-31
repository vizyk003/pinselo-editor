// SPDX-License-Identifier: Apache-2.0
//! `oc-templates`: the template catalogue's database and render side (plan 13, stage 4.2; was the TS
//! `tools/scripts/seed.ts` + `tpl-preview.ts` on `@oc/db`, `@oc/storage` and `@oc/render-canvaskit`).
//!
//! The documents themselves are built in TypeScript (`tools/scripts/templates/*`, on `@oc/engine`), which writes a
//! bundle directory:
//!
//! - `templates.json`: `[{ id, title, category, preset, tags, premium, doc }]`
//! - `assets.json`: `[{ id, sha256, mime, w, h, name, source, attribution, file }]` — the library assets (pinned
//!   photos, emoji art) the templates reference; `file` is the path of the bytes.
//!
//! Commands (`OC_DATABASE_URL` and the object store come from the usual environment, like the services):
//!
//! - `oc-templates seed <bundle>`: validate every template, upload the library assets that are missing (row or bytes),
//!   render each first page as the thumbnail (long side ≤ 480 px) and upsert the `templates` rows.
//! - `oc-templates preview <bundle> <out> [--pages] [--width 900]`: validate and render PNGs to `<out>`
//!   (`<id>.png`, `<id>-p2.png` …) without writing anything to the database. Fonts still come from the configured
//!   database and object store (pinned Google Fonts), like the render service.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oc_gfonts::manifest::{FontFamily, FontManifest};
use oc_render_svc::oc_renderer::OcRenderer;
use oc_render_svc::renderer::{ImageFormat, RasterOptions, RenderError, RenderFuture, RenderHost, Renderer};
use oc_render_svc::service::{Service, Settings};
use oc_schema::DesignDoc;
use oc_schema::tree::sorted_pages;
use oc_schema::validate::validate_doc;
use serde::Deserialize;
use serde_json::Value;

/// The template library's own account: owns the photos and artwork templates reference (never signs in).
const LIBRARY_USER: &str = "00000000-0000-4000-8000-00000000c0de";
const LIBRARY_WS: &str = "00000000-0000-4000-8000-00000000c0df";

#[derive(Deserialize)]
struct Template {
    id: String,
    title: String,
    category: String,
    preset: String,
    tags: Vec<String>,
    #[serde(default)]
    premium: bool,
    doc: Value,
}

#[derive(Deserialize)]
struct LibAsset {
    id: String,
    sha256: String,
    mime: String,
    w: f64,
    h: f64,
    name: Option<String>,
    source: String,
    attribution: Option<Value>,
    file: PathBuf,
}

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Res<T> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(serde_json::from_slice(&raw).map_err(|e| format!("{}: {e}", path.display()))?)
}

/// The render service's host, with the bundle's library assets served from their files (so a preview needs no
/// upload, and a seed renders before its uploads are visible).
struct BundleHost<'a> {
    svc: &'a Service,
    files: HashMap<String, PathBuf>,
}

impl RenderHost for BundleHost<'_> {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            match self.files.get(key) {
                Some(f) => tokio::fs::read(f).await.map_err(|e| RenderError::Failed(format!("{}: {e}", f.display()))),
                None => RenderHost::asset_bytes(self.svc, key).await,
            }
        })
    }
    fn font_file<'a>(&'a self, file: &'a str) -> RenderFuture<'a, Vec<u8>> {
        RenderHost::font_file(self.svc, file)
    }
    fn custom_families<'a>(&'a self, families: &'a [String]) -> RenderFuture<'a, Vec<FontFamily>> {
        RenderHost::custom_families(self.svc, families)
    }
    fn manifest(&self) -> &FontManifest {
        &self.svc.manifest
    }
    fn fonts_dir(&self) -> PathBuf {
        self.svc.settings.fonts_dir.clone()
    }
}

/// Validates a template and parses it; prints the first issues of an invalid one.
fn parse(t: &Template) -> Option<DesignDoc> {
    if let Err(issues) = validate_doc(&t.doc) {
        let first: Vec<String> = issues.iter().take(5).map(|i| format!("{}: {}", i.path, i.message)).collect();
        eprintln!("template {} is invalid: {}", t.id, first.join("; "));
        return None;
    }
    match serde_json::from_value::<DesignDoc>(t.doc.clone()) {
        Ok(d) => Some(d),
        Err(e) => {
            eprintln!("template {} does not parse: {e}", t.id);
            None
        }
    }
}

async fn render(host: &BundleHost<'_>, renderer: &OcRenderer, doc: &DesignDoc, page: &str, scale: f64) -> Res<Vec<u8>> {
    Ok(renderer.render_image(host, doc, page, &RasterOptions::at_scale(scale), ImageFormat::Png).await?)
}

async fn seed(svc: &Service, renderer: &OcRenderer, templates: &[Template], assets: &[LibAsset], host: &BundleHost<'_>) -> Res<bool> {
    let pool = &svc.pool;
    sqlx::query(
        "INSERT INTO users(id, email, name, color, disabled_at, disabled_reason) VALUES ($1::uuid, 'library@opencanvas.invalid', 'Pinselo Library', '#7c3aed', now(), 'system') ON CONFLICT (id) DO NOTHING",
    )
    .bind(LIBRARY_USER)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO workspaces(id, name, personal, owner_id) VALUES ($1::uuid, 'Pinselo Library', true, $2::uuid) ON CONFLICT (id) DO NOTHING")
        .bind(LIBRARY_WS)
        .bind(LIBRARY_USER)
        .execute(pool)
        .await?;

    let mut uploaded = 0;
    for a in assets {
        // The row can exist while the bytes are missing (seeded before the object store moved), so check both.
        let row: Option<i32> = sqlx::query_scalar("SELECT 1 FROM assets WHERE id = $1").bind(&a.id).fetch_optional(pool).await?;
        if row.is_some() && svc.objects.exists(&a.sha256).await? {
            continue;
        }
        let bytes = std::fs::read(&a.file).map_err(|e| format!("{}: {e}", a.file.display()))?;
        let sha = svc.objects.put(&bytes, Some(&a.mime)).await?;
        if sha != a.sha256 {
            return Err(format!("asset {}: sha256 {sha} of {} is not {}", a.id, a.file.display(), a.sha256).into());
        }
        sqlx::query(
            "INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, source, attribution)
             VALUES ($1,$2::uuid,$3::uuid,$4,$5,'image',$6,$7,$8,$9,$10,$11) ON CONFLICT (id) DO NOTHING",
        )
        .bind(&a.id)
        .bind(LIBRARY_WS)
        .bind(LIBRARY_USER)
        .bind(&a.sha256)
        .bind(&a.mime)
        .bind(bytes.len() as i64)
        .bind(a.w.round() as i32)
        .bind(a.h.round() as i32)
        .bind(&a.name)
        .bind(&a.source)
        .bind(a.attribution.as_ref().map(sqlx::types::Json))
        .execute(pool)
        .await?;
        uploaded += 1;
    }

    let (mut n, mut ok) = (0, true);
    for t in templates {
        let Some(doc) = parse(t) else {
            ok = false;
            continue;
        };
        let Some(page) = sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else {
            eprintln!("template {} has no page", t.id);
            ok = false;
            continue;
        };
        let scale = 1f64.min(480.0 / doc.meta.dims.w.max(doc.meta.dims.h));
        let png = render(host, renderer, &doc, &page, scale).await.map_err(|e| format!("template {}: {e}", t.id))?;
        let thumbnail = svc.objects.put(&png, Some("image/png")).await?;
        let mut search = vec![t.title.clone(), t.category.clone()];
        search.extend(t.tags.iter().cloned());
        sqlx::query(
            "INSERT INTO templates(id, title, category, preset, tags, search_text, width, height, doc, thumbnail, premium)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (id) DO UPDATE SET title = EXCLUDED.title, category = EXCLUDED.category, preset = EXCLUDED.preset, tags = EXCLUDED.tags,
               search_text = EXCLUDED.search_text, width = EXCLUDED.width, height = EXCLUDED.height, doc = EXCLUDED.doc, thumbnail = EXCLUDED.thumbnail,
               premium = EXCLUDED.premium",
        )
        .bind(&t.id)
        .bind(&t.title)
        .bind(&t.category)
        .bind(&t.preset)
        .bind(&t.tags)
        .bind(search.join(" "))
        .bind(doc.meta.dims.w)
        .bind(doc.meta.dims.h)
        .bind(sqlx::types::Json(&t.doc))
        .bind(&thumbnail)
        .bind(t.premium)
        .execute(pool)
        .await?;
        n += 1;
    }
    println!("seeded {n}/{} templates, {uploaded} new library assets", templates.len());
    Ok(ok)
}

async fn preview(renderer: &OcRenderer, templates: &[Template], host: &BundleHost<'_>, out: &Path, pages: bool, width: f64) -> Res<bool> {
    std::fs::create_dir_all(out)?;
    let mut ok = true;
    for t in templates {
        let Some(doc) = parse(t) else {
            ok = false;
            continue;
        };
        let scale = 1f64.min(width / doc.meta.dims.w.max(doc.meta.dims.h));
        let all = sorted_pages(&doc.pages);
        let take = if pages { all.len() } else { all.len().min(1) };
        for (i, p) in all.iter().take(take).enumerate() {
            let png = render(host, renderer, &doc, &p.id, scale).await.map_err(|e| format!("template {}: {e}", t.id))?;
            let name = if i == 0 { format!("{}.png", t.id) } else { format!("{}-p{}.png", t.id, i + 1) };
            std::fs::write(out.join(name), png)?;
        }
    }
    Ok(ok)
}

fn usage() -> ! {
    eprintln!("usage: oc-templates seed <bundle> | oc-templates preview <bundle> <out> [--pages] [--width <px>]");
    std::process::exit(2);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, bundle) = match (args.first(), args.get(1)) {
        (Some(c), Some(b)) => (c.as_str(), PathBuf::from(b)),
        _ => usage(),
    };
    let r = run(cmd, &bundle, &args[2..]).await;
    match r {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("oc-templates: {e}");
            std::process::exit(1);
        }
    }
}

async fn run(cmd: &str, bundle: &Path, rest: &[String]) -> Res<bool> {
    let templates: Vec<Template> = read_json(&bundle.join("templates.json"))?;
    let assets: Vec<LibAsset> = read_json(&bundle.join("assets.json"))?;
    let cfg = oc_config::config();
    let pool = oc_db::db().clone();
    let objects = oc_storage::objects().await.clone();
    let settings = Settings::from_config(cfg);
    let fonts = oc_gfonts::GoogleFonts::new(settings.fonts_dir.clone(), pool.clone(), objects.clone());
    let manifest = fonts.full_manifest().await?;
    let renderer = OcRenderer::with_manifest(manifest.clone());
    let svc = Service::new(pool, objects, Arc::new(renderer.clone()), fonts, manifest, settings);
    let host = BundleHost { svc: &svc, files: assets.iter().map(|a| (a.id.clone(), a.file.clone())).collect() };
    match cmd {
        "seed" => seed(&svc, &renderer, &templates, &assets, &host).await,
        "preview" => {
            let Some(out) = rest.first() else { usage() };
            let pages = rest.iter().any(|a| a == "--pages");
            let width = rest.iter().position(|a| a == "--width").and_then(|i| rest.get(i + 1)).and_then(|w| w.parse().ok()).unwrap_or(900.0);
            preview(&renderer, &templates, &host, Path::new(out), pages, width).await
        }
        _ => usage(),
    }
}
