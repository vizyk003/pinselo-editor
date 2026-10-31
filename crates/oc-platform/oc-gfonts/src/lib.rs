//! Google Fonts on demand (FT-030). The whole catalogue (~1,900 families, all OFL / Apache / UFL) is listed
//! in `fixtures/fonts/google-catalog.json` (written by `pnpm fonts:sync`). Faces are fetched from Google the
//! first time anything needs them and pinned: bytes go to the object store and the `google_font_files` row
//! records which ones, so every service lays text out with identical files — the same guarantee the bundled
//! fonts have (ADR-0043). Paths look like `gf/<slug>/<weight>[i].ttf`.
//!
//! Font *parsing* belongs to `oc-text`:
//! - [`manifest`] re-exports `oc-text`'s `FontFace`/`FontFamily`/`FontManifest` (the `@oc/text` JSON shape).
//! - [`FontInspector`] validates downloaded bytes. [`GoogleFonts::new`] uses `oc-text`'s [`ParsingInspector`]
//!   (the TS check — ≤ 40 MiB, sfnt/`true`/`OTTO` magic — plus a real parse, so bytes that cannot be laid out are
//!   never pinned); [`MagicBytesInspector`] is exactly the TS check and can be chosen with
//!   [`GoogleFonts::with_inspector`].

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use oc_storage::ObjectStore;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub mod manifest {
    //! `packages/text/src/fonts.ts` manifest types, from `oc-text`.
    pub use oc_text::{FontFace, FontFamily, FontManifest};
}

use manifest::{FontFace, FontFamily, FontManifest};

pub use oc_text::{FontInspector, MAX_FONT_BYTES, MagicBytesInspector, ParsingInspector};

/// Compact catalogue row: faces are `"400"`, `"400i"`, `"700"`, …
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogFamily {
    pub family: String,
    pub slug: String,
    pub category: String,
    pub faces: Vec<String>,
    pub popularity: serde_json::Number,
    pub subsets: Vec<String>,
}

/// Google's catalogue metadata (`fonts.google.com/metadata/fonts`), the fields we use.
#[derive(Debug, Clone, Deserialize)]
pub struct GoogleMetadata {
    #[serde(rename = "familyMetadataList")]
    pub family_metadata_list: Vec<MetadataFamily>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataFamily {
    pub family: String,
    pub category: String,
    pub fonts: serde_json::Map<String, serde_json::Value>,
    pub popularity: serde_json::Number,
    pub subsets: Vec<String>,
}

/// `slugOf(family)`: lowercase, runs of non-`[a-z0-9]` → `-`, trimmed of one leading/trailing `-`.
pub fn slug_of(family: &str) -> String {
    let lower = js_to_lower(family);
    let mut s = String::with_capacity(lower.len());
    let mut in_run = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            s.push(c);
            in_run = false;
        } else if !in_run {
            s.push('-');
            in_run = true;
        }
    }
    let s = s.strip_prefix('-').unwrap_or(&s);
    s.strip_suffix('-').unwrap_or(s).to_owned()
}

/// `String.prototype.toLowerCase` (Unicode default case mapping, like Rust's `to_lowercase`).
fn js_to_lower(s: &str) -> String {
    s.to_lowercase()
}

fn category_of(google: &str) -> &'static str {
    match google {
        "Sans Serif" => "sans",
        "Serif" => "serif",
        "Display" => "display",
        "Handwriting" => "handwriting",
        "Monospace" => "mono",
        _ => "display",
    }
}

fn is_face_key(k: &str) -> bool {
    let b = k.as_bytes();
    (b.len() == 3 || (b.len() == 4 && b[3] == b'i')) && b[..3].iter().all(u8::is_ascii_digit)
}

/// Google's catalogue metadata → compact rows (used by fonts-sync), sorted by popularity (stable).
pub fn catalog_from_metadata(meta: &GoogleMetadata) -> Vec<CatalogFamily> {
    let mut v: Vec<CatalogFamily> = meta
        .family_metadata_list
        .iter()
        .filter(|f| !f.fonts.is_empty())
        .map(|f| {
            let mut faces: Vec<String> = f.fonts.keys().filter(|k| is_face_key(k)).cloned().collect();
            faces.sort_by(|a, b| oc_js::cmp_utf16(a, b));
            CatalogFamily {
                family: f.family.clone(),
                slug: slug_of(&f.family),
                category: category_of(&f.category).to_owned(),
                faces,
                popularity: f.popularity.clone(),
                subsets: f.subsets.iter().filter(|s| *s != "menu").cloned().collect(),
            }
        })
        .filter(|f| !f.faces.is_empty())
        .collect();
    v.sort_by(|a, b| {
        let (x, y) = (a.popularity.as_f64().unwrap_or(f64::NAN), b.popularity.as_f64().unwrap_or(f64::NAN));
        x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}

/// `gf/<slug>/<weight>[i].ttf` → (slug, weight, italic).
pub fn parse_file(file: &str) -> Option<(&str, &str, bool)> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let c = RE.get_or_init(|| regex::Regex::new(r"^gf/([a-z0-9-]+)/(\d{3})(i?)\.ttf$").unwrap()).captures(file)?;
    Some((c.get(1)?.as_str(), c.get(2)?.as_str(), !c.get(3)?.as_str().is_empty()))
}

/// The CSS2 URL for a full face (`&display=swap`).
pub fn css2_face_url(family: &str, weight: &str, italic: bool) -> String {
    format!("https://fonts.googleapis.com/css2?family={}:ital,wght@{},{}&display=swap", oc_js::encode_uri_component(family), u8::from(italic), weight)
}

/// The CSS2 URL for a preview subset containing just the family's name. `face` is a catalogue face key.
pub fn css2_preview_url(family: &str, face: &str) -> String {
    format!(
        "https://fonts.googleapis.com/css2?family={}:ital,wght@{},{}&text={}",
        oc_js::encode_uri_component(family),
        u8::from(face.ends_with('i')),
        &face[..face.len().min(3)],
        oc_js::encode_uri_component(family)
    )
}

/// The first `fonts.gstatic.com` URL declared `format('truetype')` in a CSS2 response.
pub fn extract_ttf_url(css: &str) -> Option<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"url\((https://fonts\.gstatic\.com/[^)\s]+)\) format\('truetype'\)").unwrap()).captures(css).map(|c| c[1].to_owned())
}

/// Bundled (pinned) families plus every Google family not already bundled.
pub fn merge_manifest(pinned: FontManifest, catalog: &[CatalogFamily]) -> FontManifest {
    let have: std::collections::HashSet<&str> = pinned.families.iter().map(|f| f.family.as_str()).collect();
    let extra: Vec<FontFamily> = catalog
        .iter()
        .filter(|c| !have.contains(c.family.as_str()))
        .map(|c| FontFamily {
            family: c.family.clone(),
            slug: c.slug.clone(),
            category: c.category.clone(),
            license: "Google Fonts (OFL / Apache / UFL)".into(),
            source: Some("google".into()),
            faces: c
                .faces
                .iter()
                .map(|k| FontFace { weight: k[..3].parse().unwrap_or(0), italic: k.ends_with('i'), file: format!("gf/{}/{k}.ttf", c.slug), sha256: String::new(), bytes: 0 })
                .collect(),
        })
        .collect();
    let mut families = pinned.families;
    families.extend(extra);
    FontManifest { version: pinned.version, families }
}

/// Old UA ⇒ Google serves full TrueType files.
pub const USER_AGENT: &str = "Mozilla/4.0 (compatible; Pinselo font fetch)";

#[derive(Debug, Clone, thiserror::Error)]
pub enum GfontsError {
    #[error("unknown Google font {0}")]
    UnknownFile(String),
    #[error("unknown family")]
    UnknownFamily,
    #[error("{0}")]
    Fetch(String),
    #[error("{0}")]
    Other(String),
}

fn other(e: impl std::fmt::Display) -> GfontsError {
    GfontsError::Other(e.to_string())
}

type Bytes = Arc<Vec<u8>>;
type SharedFetch = Shared<BoxFuture<'static, Result<Bytes, GfontsError>>>;

/// Loader for `custom/…` (uploaded) fonts.
pub type CustomLoader = Arc<dyn Fn(String) -> BoxFuture<'static, Result<Vec<u8>, GfontsError>> + Send + Sync>;

struct Catalog {
    list: Vec<CatalogFamily>,
    by_slug: HashMap<String, usize>,
}

struct Inner {
    fonts_dir: PathBuf,
    pool: PgPool,
    store: ObjectStore,
    http: reqwest::Client,
    inspector: Arc<dyn FontInspector>,
    catalog: OnceLock<Catalog>,
    inflight: Mutex<HashMap<String, SharedFetch>>,
    previews: Mutex<(HashMap<String, SharedFetch>, VecDeque<String>)>,
}

/// The Google Fonts service for one fonts directory.
#[derive(Clone)]
pub struct GoogleFonts(Arc<Inner>);

impl GoogleFonts {
    pub fn new(fonts_dir: impl Into<PathBuf>, pool: PgPool, store: ObjectStore) -> Self {
        Self::with_inspector(fonts_dir, pool, store, Arc::new(ParsingInspector))
    }

    pub fn with_inspector(fonts_dir: impl Into<PathBuf>, pool: PgPool, store: ObjectStore, inspector: Arc<dyn FontInspector>) -> Self {
        GoogleFonts(Arc::new(Inner {
            fonts_dir: fonts_dir.into(),
            pool,
            store,
            http: reqwest::Client::new(),
            inspector,
            catalog: OnceLock::new(),
            inflight: Mutex::default(),
            previews: Mutex::default(),
        }))
    }

    pub fn fonts_dir(&self) -> &Path {
        &self.0.fonts_dir
    }

    fn cat(&self) -> &Catalog {
        self.0.catalog.get_or_init(|| {
            let list = load_catalog(&self.0.fonts_dir);
            let by_slug = list.iter().enumerate().map(|(i, x)| (x.slug.clone(), i)).collect();
            Catalog { list, by_slug }
        })
    }

    /// `googleCatalog(fontsDir)`: the catalogue (empty when the file is missing), loaded once.
    pub fn catalog(&self) -> &[CatalogFamily] {
        &self.cat().list
    }

    pub fn family_by_slug(&self, slug: &str) -> Option<&CatalogFamily> {
        let c = self.cat();
        c.by_slug.get(slug).map(|&i| &c.list[i])
    }

    /// `fullManifest(fontsDir)`.
    pub async fn full_manifest(&self) -> Result<FontManifest, GfontsError> {
        let raw = tokio::fs::read(self.0.fonts_dir.join("manifest.json")).await.map_err(other)?;
        let pinned: FontManifest = serde_json::from_slice(&raw).map_err(other)?;
        Ok(merge_manifest(pinned, self.catalog()))
    }

    async fn fetch_ttf(&self, css2: &str) -> Result<(Vec<u8>, String), GfontsError> {
        let css = self.0.http.get(css2).header("user-agent", USER_AGENT).timeout(Duration::from_secs(15)).send().await.map_err(|e| GfontsError::Fetch(e.to_string()))?;
        if !css.status().is_success() {
            return Err(GfontsError::Fetch(format!("Google Fonts CSS {}", css.status().as_u16())));
        }
        let text = css.text().await.map_err(|e| GfontsError::Fetch(e.to_string()))?;
        let url = extract_ttf_url(&text).ok_or_else(|| GfontsError::Fetch("no TrueType source in Google Fonts CSS".into()))?;
        let r = self.0.http.get(&url).timeout(Duration::from_secs(60)).send().await.map_err(|e| GfontsError::Fetch(e.to_string()))?;
        if !r.status().is_success() {
            return Err(GfontsError::Fetch(format!("Google Fonts file {}", r.status().as_u16())));
        }
        let bytes = r.bytes().await.map_err(|e| GfontsError::Fetch(e.to_string()))?.to_vec();
        self.0.inspector.validate(&bytes).map_err(GfontsError::Fetch)?;
        Ok((bytes, url))
    }

    async fn pinned(&self, file: &str) -> Result<Option<String>, GfontsError> {
        sqlx::query_scalar("SELECT sha256 FROM google_font_files WHERE file = $1").bind(file).fetch_optional(&self.0.pool).await.map_err(other)
    }

    async fn font_file_uncached(&self, file: String, family: String, weight: String, italic: bool) -> Result<Bytes, GfontsError> {
        if let Some(sha) = self.pinned(&file).await? {
            match self.0.store.get(&sha).await {
                Ok(b) => return Ok(Arc::new(b)),
                // The row outlived its bytes (the object store was moved or wiped): drop it, fetch again and re-pin.
                Err(e) if self.0.store.exists(&sha).await.map_err(other)? => return Err(other(e)),
                Err(_) => {
                    sqlx::query("DELETE FROM google_font_files WHERE file = $1 AND sha256 = $2")
                        .bind(&file)
                        .bind(&sha)
                        .execute(&self.0.pool)
                        .await
                        .map_err(other)?;
                }
            }
        }
        let (bytes, url) = self.fetch_ttf(&css2_face_url(&family, &weight, italic)).await?;
        let sha = self.0.store.put(&bytes, Some("font/ttf")).await.map_err(other)?;
        sqlx::query("INSERT INTO google_font_files(file, sha256, bytes, source_url) VALUES ($1,$2,$3,$4) ON CONFLICT (file) DO NOTHING")
            .bind(&file)
            .bind(&sha)
            .bind(bytes.len() as i32)
            .bind(&url)
            .execute(&self.0.pool)
            .await
            .map_err(other)?;
        // Another process may have pinned first: always serve the pinned bytes.
        match self.pinned(&file).await? {
            Some(row) if row != sha => Ok(Arc::new(self.0.store.get(&row).await.map_err(other)?)),
            _ => Ok(Arc::new(bytes)),
        }
    }

    /// `googleFontFile(fontsDir, file)`: bytes of a `gf/…` face — the pinned copy if any, otherwise fetched
    /// from Google and pinned. Concurrent requests for one file share a single fetch.
    pub async fn font_file(&self, file: &str) -> Result<Bytes, GfontsError> {
        let (slug, weight, italic) = parse_file(file).ok_or_else(|| GfontsError::UnknownFile(file.to_owned()))?;
        let face = format!("{weight}{}", if italic { "i" } else { "" });
        let fam = self.family_by_slug(slug).filter(|f| f.faces.contains(&face)).ok_or_else(|| GfontsError::UnknownFile(file.to_owned()))?;
        let fut = {
            let mut m = self.0.inflight.lock().unwrap();
            m.entry(file.to_owned())
                .or_insert_with(|| {
                    let (me, file, family, weight) = (self.clone(), file.to_owned(), fam.family.clone(), weight.to_owned());
                    async move {
                        let r = me.font_file_uncached(file.clone(), family, weight, italic).await;
                        me.0.inflight.lock().unwrap().remove(&file);
                        r
                    }
                    .boxed()
                    .shared()
                })
                .clone()
        };
        fut.await
    }

    /// `googleFontPreview(fontsDir, slug)`: a tiny subset containing just the family's name. Cached
    /// (at most 3000 families; failures are not cached).
    pub async fn font_preview(&self, slug: &str) -> Result<Bytes, GfontsError> {
        let fam = self.family_by_slug(slug).ok_or(GfontsError::UnknownFamily)?;
        let fut = {
            let mut g = self.0.previews.lock().unwrap();
            if let Some(f) = g.0.get(slug) {
                f.clone()
            } else {
                let face = if fam.faces.iter().any(|f| f == "400") { "400".to_owned() } else { fam.faces[0].clone() };
                let (me, slug_s, url) = (self.clone(), slug.to_owned(), css2_preview_url(&fam.family, &face));
                let f = async move {
                    let r = me.fetch_ttf(&url).await.map(|(b, _)| Arc::new(b));
                    if r.is_err() {
                        let mut g = me.0.previews.lock().unwrap();
                        g.0.remove(&slug_s);
                        g.1.retain(|k| *k != slug_s);
                    }
                    r
                }
                .boxed()
                .shared();
                g.0.insert(slug.to_owned(), f.clone());
                g.1.push_back(slug.to_owned());
                if g.0.len() > 3000
                    && let Some(old) = g.1.pop_front()
                {
                    g.0.remove(&old);
                }
                f
            }
        };
        fut.await
    }

    /// `fontFileLoader(fontsDir, custom)(file)`: one loader for every font path — bundled files
    /// (`<fontsDir>/files/<file>`), Google faces (`gf/…`) and, if given, uploaded fonts (`custom/…`).
    pub async fn load(&self, file: &str, custom: Option<&CustomLoader>) -> Result<Vec<u8>, GfontsError> {
        if file.starts_with("gf/") {
            return Ok(self.font_file(file).await?.as_ref().clone());
        }
        if let (true, Some(c)) = (file.starts_with("custom/"), custom) {
            return c(file.to_owned()).await;
        }
        tokio::fs::read(self.0.fonts_dir.join("files").join(file)).await.map_err(other)
    }
}

/// Read `<fontsDir>/google-catalog.json` (empty list if missing).
pub fn load_catalog(fonts_dir: &Path) -> Vec<CatalogFamily> {
    let f = fonts_dir.join("google-catalog.json");
    if !f.exists() {
        return vec![];
    }
    let raw = std::fs::read(&f).unwrap_or_else(|e| panic!("reading {}: {e}", f.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("parsing {}: {e}", f.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug_of("Noto Sans JP"), "noto-sans-jp");
        assert_eq!(slug_of("  M PLUS 1p!"), "m-plus-1p");
        assert_eq!(slug_of("--a--"), "a");
    }

    #[test]
    fn files() {
        assert_eq!(parse_file("gf/roboto/400i.ttf"), Some(("roboto", "400", true)));
        assert_eq!(parse_file("gf/Roboto/400.ttf"), None);
        assert_eq!(parse_file("gf/roboto/40.ttf"), None);
    }

    #[test]
    fn magic() {
        assert!(MagicBytesInspector.validate(b"OTTO....").is_ok());
        assert!(MagicBytesInspector.validate(b"\0\x01\0\0").is_ok());
        assert!(MagicBytesInspector.validate(b"<htm").is_err());
        // The default inspector also parses: a magic number alone is not a font.
        assert!(ParsingInspector.validate(b"\0\x01\0\0").is_err());
    }

    #[test]
    fn css_extract() {
        let css = "src: url(https://fonts.gstatic.com/s/roboto/v1/x.ttf) format('truetype');";
        assert_eq!(extract_ttf_url(css).unwrap(), "https://fonts.gstatic.com/s/roboto/v1/x.ttf");
    }
}
