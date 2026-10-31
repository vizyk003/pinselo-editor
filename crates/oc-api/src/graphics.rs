//! Port of services/api/src/graphics.ts — the graphics & icon library (A4): the synced manifest
//! (fixtures/graphics, `pnpm graphics:sync`) and keyword search over it. The catalogue is static, so it is loaded
//! once per directory and searched in memory (~15k entries).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde_json::{Map, Value, json};

/// One manifest item, with the fields search needs pulled out (the JSON object is kept as it came).
pub struct GraphicItem {
    pub raw: Map<String, Value>,
    pub id: String,
    pub set: String,
    pub style: Option<String>,
    pub name: String,
    pub tags: Vec<String>,
    pub kind: String,
}

pub struct GraphicsManifest {
    pub sets: Vec<Value>,
    pub items: Vec<GraphicItem>,
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, Arc<GraphicsManifest>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn str_of(m: &Map<String, Value>, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// `loadGraphics(dir)`: an empty manifest when the catalogue is not synced (not cached, so a later sync is seen).
pub fn load_graphics(dir: &Path) -> Arc<GraphicsManifest> {
    if let Some(m) = CACHE.lock().expect("graphics cache").get(dir) {
        return m.clone();
    }
    let f = dir.join("manifest.json");
    let Ok(text) = std::fs::read_to_string(&f) else { return Arc::new(GraphicsManifest { sets: vec![], items: vec![] }) };
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let items = v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|m| GraphicItem {
            id: str_of(m, "id"),
            set: str_of(m, "set"),
            style: m.get("style").and_then(Value::as_str).map(str::to_owned),
            name: str_of(m, "name"),
            tags: m.get("tags").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
            kind: str_of(m, "kind"),
            raw: m.clone(),
        })
        .collect();
    let m = Arc::new(GraphicsManifest { sets: v["sets"].as_array().cloned().unwrap_or_default(), items });
    CACHE.lock().expect("graphics cache").insert(dir.to_owned(), m.clone());
    m
}

pub static GRAPHIC_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("regex"));

/// `icons` / `graphics` select by kind; anything else is a set id.
#[derive(Default)]
pub struct SearchOpts<'a> {
    pub set: Option<&'a str>,
    pub style: Option<&'a str>,
    pub page: Option<f64>,
    pub per_page: Option<f64>,
}

static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("regex"));
static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex"));

fn words(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    NON_WORD.split(&lower).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// `scoreGraphic(item, tokens, phrase)`: 0 = not a match (every token must hit the name or a tag); exact names beat
/// name words, which beat name prefixes, which beat tags.
pub fn score_graphic(name: &str, tags: &[String], tokens: &[String], phrase: &str) -> f64 {
    let name_words = words(name);
    let name = name_words.join(" ");
    let tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    let mut total = 0.0;
    for tok in tokens {
        let mut best: f64 = 0.0;
        if name_words.contains(tok) {
            best = 50.0;
        } else if name_words.iter().any(|w| w.starts_with(tok.as_str())) {
            best = 30.0;
        }
        let tlen = crate::js::len(tok);
        if tags.contains(tok) {
            best = best.max(25.0);
        } else if tlen >= 3 && name.contains(tok.as_str()) {
            best = best.max(15.0);
        } else if tlen >= 2 && tags.iter().any(|t| t.starts_with(tok.as_str()) || WS.split(t).any(|w| w == tok)) {
            best = best.max(10.0);
        }
        if best == 0.0 {
            return 0.0;
        }
        total += best;
    }
    if name == phrase {
        total += 200.0;
    } else if name.starts_with(&format!("{phrase} ")) {
        total += 40.0;
    }
    // Fewer extra words in the name = closer match ("heart" over "heart-handshake").
    let extra = (name_words.len() as f64 - tokens.len() as f64).clamp(0.0, 10.0);
    (total - extra * 2.0).max(1.0)
}

fn base_style(s: Option<&str>) -> bool {
    matches!(s, None | Some("outline") | Some("regular"))
}

/// `searchGraphics(items, q, opts)` → `{ items, total, page, hasMore }` (items without their tags).
pub fn search_graphics(items: &[GraphicItem], q: &str, o: &SearchOpts<'_>) -> Value {
    let per_page = o.per_page.unwrap_or(60.0).clamp(1.0, 200.0);
    let page = o.page.unwrap_or(1.0).max(1.0);
    let set = o.set.filter(|s| !s.is_empty() && *s != "all");
    let style = o.style.filter(|s| !s.is_empty());
    let pool: Vec<&GraphicItem> = items
        .iter()
        .filter(|it| {
            set.is_none_or(|set| match set {
                "icons" => it.kind == "icon",
                "graphics" => it.kind == "graphic",
                other => it.set == other,
            }) && style.is_none_or(|st| it.style.as_deref() == Some(st))
        })
        .collect();
    let mut tokens = words(q);
    tokens.truncate(8);
    let ranked: Vec<&GraphicItem> = if tokens.is_empty() {
        pool
    } else {
        let phrase = tokens.join(" ");
        let mut scored: Vec<(&GraphicItem, usize, f64)> =
            pool.into_iter().enumerate().map(|(i, it)| (it, i, score_graphic(&it.name, &it.tags, &tokens, &phrase))).filter(|x| x.2 > 0.0).collect();
        scored.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| base_style(b.0.style.as_deref()).cmp(&base_style(a.0.style.as_deref())))
                .then_with(|| crate::js::len(&a.0.name).cmp(&crate::js::len(&b.0.name)))
                .then_with(|| a.1.cmp(&b.1))
        });
        scored.into_iter().map(|x| x.0).collect()
    };
    let start = (page - 1.0) * per_page;
    let total = ranked.len();
    let page_items: Vec<Value> = ranked
        .iter()
        .skip(start as usize)
        .take(per_page as usize)
        .map(|it| {
            let mut m = it.raw.clone();
            m.shift_remove("tags");
            Value::Object(m)
        })
        .collect();
    json!({ "items": page_items, "total": total, "page": crate::http::jsjson::number_value(page), "hasMore": start + per_page < total as f64 })
}
