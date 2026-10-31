//! Port of services/api/src/routes/graphics.ts — the graphics & icon library (A4): search the synced open-licence
//! catalogue (Lucide, Tabler, Phosphor, illustrations and Pinselo originals), serve the SVGs, and turn one into an
//! `svg` design asset that can be recoloured non-destructively.
//!
//! The PNG raster comes from Rust resvg instead of `@resvg/resvg-js`: same size, different bytes (see ROUTES.md).

use crate::auth::personal_workspace;
use crate::graphics::{GRAPHIC_ID, SearchOpts, load_graphics, search_graphics};
use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[];

/// Icons paint with `currentColor`; as design assets they start in the editor's default ink colour (recolourable).
pub const DEFAULT_ICON_COLOR: &str = "#1f2328";
/// Longest side of the raster an inserted graphic renders from (exports and recolours use the same size).
const RASTER: u32 = 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/graphics/sets", get(sets))
        .route("/api/graphics/search", get(search))
        .route("/api/graphics/{file}", get(file))
        .route("/api/graphics/{id}/asset", post(asset))
}

async fn sets(State(s): State<AppState>, _req: Req) -> ApiResult {
    let m = load_graphics(&s.paths.graphics);
    Ok(Reply::json(json!({ "sets": m.sets })).header("cache-control", "public, max-age=3600"))
}

async fn search(State(s): State<AppState>, req: Req) -> ApiResult {
    let q = z::obj([
        ("q", z::str().max(100.0).default(json!(""))),
        ("set", z::str().max(40.0).optional()),
        ("style", z::str().max(20.0).optional()),
        ("page", z::coerce_num().int().min(1.0).max(500.0).default(json!(1))),
        ("perPage", z::coerce_num().int().min(1.0).max(200.0).default(json!(60))),
    ])
    .parse(Some(&req.query))?;
    let m = load_graphics(&s.paths.graphics);
    let o = SearchOpts { set: q.get("set").and_then(Value::as_str), style: q.get("style").and_then(Value::as_str), page: q["page"].as_f64(), per_page: q["perPage"].as_f64() };
    let r = search_graphics(&m.items, q["q"].as_str().unwrap_or(""), &o);
    Ok(Reply::json(r).header("cache-control", "public, max-age=300"))
}

fn strip_svg(f: &str) -> &str {
    f.strip_suffix(".svg").unwrap_or(f)
}

async fn file(State(s): State<AppState>, Path(file): Path<String>, _req: Req) -> ApiResult {
    let id = strip_svg(&file);
    let f = s.paths.graphics.join("svg").join(format!("{id}.svg"));
    if !GRAPHIC_ID.is_match(id) || !f.exists() {
        return Ok(Reply::empty(404));
    }
    Ok(Reply::bytes("image/svg+xml", tokio::fs::read(&f).await?)
        .header("cache-control", "public, max-age=31536000, immutable")
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'; sandbox"))
}

/// `new Resvg(svg, { fitTo: { mode: 'width' | 'height', value } }).render().asPng()`.
fn raster(svg: &str, wide: bool) -> Result<Vec<u8>, String> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let base = tree.size().to_int_size();
    let size = if wide { base.scale_to_width(RASTER) } else { base.scale_to_height(RASTER) }.ok_or("bad size")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("bad size")?;
    let ts = resvg::tiny_skia::Transform::from_scale(size.width() as f32 / tree.size().width(), size.height() as f32 / tree.size().height());
    resvg::render(&tree, ts, &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// A graphic as a design asset: rasterised once, SVG source kept for recolouring, reused per user by content hash.
async fn asset(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let m = load_graphics(&s.paths.graphics);
    let item = if GRAPHIC_ID.is_match(&id) { m.items.iter().find(|x| x.id == id) } else { None };
    let f = s.paths.graphics.join("svg").join(format!("{id}.svg"));
    let Some(item) = item.filter(|_| f.exists()) else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    let set = m.sets.iter().find(|x| x["id"].as_str() == Some(item.set.as_str()));
    let mut svg = oc_schema::svg::sanitize_svg(&String::from_utf8_lossy(&tokio::fs::read(&f).await?));
    if item.kind == "icon" {
        svg = svg.replace("currentColor", DEFAULT_ICON_COLOR);
    }
    let (pw, ph) = crate::routes::assets::svg_size(&svg).map_err(ApiError::internal)?;
    let (pw, ph) = (f64::from(pw), f64::from(ph));
    let wide = pw >= ph;
    let png = raster(&svg, wide).map_err(ApiError::internal)?;
    let r = f64::from(RASTER);
    let w = if wide { r } else { crate::js::round(r * pw / ph).max(1.0) };
    let h = if wide { crate::js::round(r * ph / pw).max(1.0) } else { r };
    let sha = oc_storage::sha256_hex(&png);
    let style = item.style.as_deref().filter(|st| !["outline", "regular"].contains(st)).map(|st| format!(" ({st})")).unwrap_or_default();
    let name = format!("{}{style}", item.name.replace('-', " "));
    let reference = |asset_id: Value, ww: Value, hh: Value| json!({ "id": asset_id, "kind": "svg", "sha256": sha, "mime": "image/svg+xml", "w": ww, "h": hh, "name": name });
    let existing = pg::json_one(
        &s.pool,
        "SELECT id, width, height FROM assets WHERE owner_id = $1 AND sha256 = $2 AND source = 'graphic' AND svg_sha256 IS NOT NULL AND trashed_at IS NULL LIMIT 1",
        &[pg::uuid(u.id), pg::text(&sha)],
    )
    .await?;
    if let Some(e) = existing {
        return Ok(Reply::json(json!({ "asset": reference(e["id"].clone(), e["width"].clone(), e["height"].clone()) })));
    }
    s.store.put(&png, Some("image/png")).await?;
    let svg_sha = s.store.put(svg.as_bytes(), Some("image/svg+xml")).await?;
    let asset_id = oc_schema::ids::new_id();
    let mut attribution = serde_json::Map::new();
    attribution.insert("provider".into(), set.and_then(|x| x.get("label")).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(item.set)));
    attribution.insert("license".into(), item.raw.get("license").cloned().unwrap_or(Value::Null));
    if let Some(l) = set.and_then(|x| x.get("licenseUrl")).filter(|v| crate::js::truthy(v.as_str())) {
        attribution.insert("licenseUrl".into(), l.clone());
    }
    if let Some(l) = set.and_then(|x| x.get("source")).filter(|v| crate::js::truthy(v.as_str())) {
        attribution.insert("sourceUrl".into(), l.clone());
    }
    let ws = personal_workspace(&s, u.id).await?;
    sqlx::query(
        "INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, source, attribution, svg_sha256) VALUES ($1,$2,$3,$4,'image/svg+xml','svg',$5,$6,$7,$8,'graphic',$9,$10)",
    )
    .bind(&asset_id)
    .bind(ws)
    .bind(u.id)
    .bind(&sha)
    .bind(png.len() as i64)
    .bind(w as i32)
    .bind(h as i32)
    .bind(&name)
    .bind(sqlx::types::Json(Value::Object(attribution)))
    .bind(&svg_sha)
    .execute(&s.pool)
    .await?;
    let num = |x: f64| crate::http::jsjson::number_value(x).unwrap_or(Value::Null);
    Ok(Reply::json(json!({ "asset": reference(json!(asset_id), num(w), num(h)) })))
}
