//! Port of services/api/src/search.ts — unified search (g16): one query box across designs, folders, uploads
//! and templates. Pure SQL builders so the visibility rules can be unit-tested. Each group repeats the exact
//! visibility predicate of the route that lists it (GET /api/designs default view, GET /api/folders,
//! GET /api/assets, GET /api/templates), so search can never show more than browsing does. Every user value is a
//! bind parameter; only fixed SQL fragments are concatenated.

use std::cmp::Ordering;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::pg::{self, Arg};
use crate::zod as z;

pub const SEARCH_TYPES: [&str; 5] = ["all", "designs", "folders", "uploads", "templates"];
pub const SEARCH_OWNERS: [&str; 3] = ["anyone", "me", "shared"];
pub const SEARCH_MODIFIED: [&str; 5] = ["any", "1d", "7d", "30d", "365d"];

/// `searchQuery.parse(input)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchParams {
    pub q: String,
    pub kind: String,
    pub owner: String,
    pub modified: String,
    pub surface: Option<String>,
    pub limit: i64,
}

pub fn search_query() -> z::Schema {
    z::obj([
        ("q", z::str().max(200.0).default(json!(""))),
        ("type", z::enumeration(&SEARCH_TYPES).default(json!("all"))),
        ("owner", z::enumeration(&SEARCH_OWNERS).default(json!("anyone"))),
        ("modified", z::enumeration(&SEARCH_MODIFIED).default(json!("any"))),
        ("surface", z::str().max(40.0).optional()),
        ("limit", z::coerce_num().int().min(1.0).max(100.0).default(json!(8))),
    ])
}

pub fn parse_search_query(input: &Value) -> Result<SearchParams, ApiError> {
    let v = search_query().parse(Some(input))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(SearchParams { q: s("q"), kind: s("type"), owner: s("owner"), modified: s("modified"), surface: v.get("surface").and_then(Value::as_str).map(str::to_owned), limit: v["limit"].as_f64().unwrap_or(8.0) as i64 })
}

/// Lower bound for "Date modified", or `None` for any time.
pub fn modified_since(m: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let d = match m {
        "1d" => 1,
        "7d" => 7,
        "30d" => 30,
        "365d" => 365,
        _ => return None,
    };
    Some(now - Duration::milliseconds(d * 86_400_000))
}

/// Escape LIKE wildcards so a literal "%" or "_" in the query matches only itself (used with ESCAPE '\').
pub fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Collapse whitespace; an empty result means "no text filter" (browse by filters only).
pub fn normalize_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    let mut ws = false;
    for c in q.chars() {
        if oc_js::is_js_whitespace(c) {
            ws = true;
        } else {
            if ws {
                out.push(' ');
            }
            ws = false;
            out.push(c);
        }
    }
    if ws {
        out.push(' ');
    }
    crate::js::trim(&out).to_owned()
}

/// Which groups a request asks for. Templates have no owner, so an owner filter leaves them out.
pub fn groups_for(kind: &str, owner: &str) -> Vec<&'static str> {
    ["designs", "folders", "uploads", "templates"]
        .into_iter()
        .filter(|g| {
            if kind != "all" && kind != *g {
                return false;
            }
            if *g == "templates" && owner != "anyone" {
                return false;
            }
            !(*g == "uploads" && owner == "shared") // uploads are private to their owner
        })
        .collect()
}

/// A built query: SQL and its bind parameters.
pub struct Built {
    pub sql: String,
    pub params: Vec<Arg>,
}

/// `col` matches the text (full-text OR substring), appending the bind parameters to `params`.
fn text_match(col: &str, q: &str, params: &mut Vec<Arg>, fts: bool) -> String {
    params.push(pg::text(format!("%{}%", like_escape(q))));
    let like = format!("{col} ILIKE ${} ESCAPE '\\'", params.len());
    if !fts {
        return like;
    }
    params.push(pg::text(q));
    format!("(to_tsvector('simple', {col}) @@ plainto_tsquery('simple', ${}) OR {like})", params.len())
}

/// Rank: exact title, then prefix, then anything else; ties broken by recency.
fn rank_by(col: &str, q: &str, params: &mut Vec<Arg>) -> String {
    params.push(pg::text(q.to_lowercase()));
    let exact = params.len();
    params.push(pg::text(format!("{}%", like_escape(&q.to_lowercase()))));
    format!("CASE WHEN lower({col}) = ${exact} THEN 0 WHEN lower({col}) LIKE ${} ESCAPE '\\' THEN 1 ELSE 2 END", params.len())
}

/// Designs the user can see in the default "recent" listing of GET /api/designs.
pub fn designs_sql(uid: Uuid, p: &SearchParams, now: DateTime<Utc>) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec![
        format!(
            "d.trashed_at IS NULL AND (d.owner_id = $1 OR {}
          OR EXISTS (SELECT 1 FROM memberships mm JOIN workspaces ww ON ww.id = mm.workspace_id WHERE mm.workspace_id = d.workspace_id AND mm.user_id = $1 AND NOT ww.personal))",
            crate::share_access::shared_with_sql("d.id", "$1")
        ),
    ];
    if p.owner == "me" {
        wh.push("d.owner_id = $1".into());
    }
    if p.owner == "shared" {
        wh.push("d.owner_id <> $1".into());
    }
    if let Some(since) = modified_since(&p.modified, now) {
        params.push(Arg::Time(Some(since)));
        wh.push(format!("d.updated_at >= ${}", params.len()));
    }
    if let Some(surface) = p.surface.as_deref().filter(|s| !s.is_empty()) {
        params.push(pg::text(surface));
        wh.push(format!("d.surface = ${}", params.len()));
    }
    let q = normalize_query(&p.q);
    if !q.is_empty() {
        wh.push(text_match("d.title", &q, &mut params, true));
    }
    let order = if q.is_empty() { "d.updated_at DESC".to_owned() } else { format!("{}, d.updated_at DESC", rank_by("d.title", &q, &mut params)) };
    params.push(Arg::Int(Some(p.limit)));
    Built {
        sql: format!(
            "SELECT d.id, d.title, d.surface, d.width, d.height, d.unit, d.page_count, d.link_access, d.owner_id, u.name AS owner_name,
                 d.folder_id, d.workspace_id, d.thumbnail, d.trashed_at, d.created_at, d.updated_at, d.public_view, (s.user_id IS NOT NULL) AS starred
          FROM designs d
          JOIN users u ON u.id = d.owner_id
          LEFT JOIN stars s ON s.design_id = d.id AND s.user_id = $1
          WHERE {} ORDER BY {order} LIMIT ${}",
            wh.join(" AND "),
            params.len()
        ),
        params,
    }
}

/// Folders as GET /api/folders shows them: my folders plus team folders of teams I belong to.
pub fn folders_sql(uid: Uuid, p: &SearchParams, now: DateTime<Utc>) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec!["((w.personal AND f.owner_id = $1) OR (NOT w.personal AND EXISTS (SELECT 1 FROM memberships m WHERE m.workspace_id = f.workspace_id AND m.user_id = $1)))".to_owned()];
    if p.owner == "me" {
        wh.push("f.owner_id = $1".into());
    }
    if p.owner == "shared" {
        wh.push("f.owner_id <> $1".into());
    }
    if let Some(since) = modified_since(&p.modified, now) {
        params.push(Arg::Time(Some(since)));
        wh.push(format!("f.created_at >= ${}", params.len()));
    }
    let q = normalize_query(&p.q);
    if !q.is_empty() {
        wh.push(text_match("f.name", &q, &mut params, false));
    }
    let order = if q.is_empty() { "f.created_at DESC".to_owned() } else { format!("{}, f.name", rank_by("f.name", &q, &mut params)) };
    params.push(Arg::Int(Some(p.limit)));
    Built {
        sql: format!(
            "SELECT f.id, f.name, f.parent_id, f.workspace_id, f.created_at, CASE WHEN w.personal THEN NULL ELSE w.name END AS team,
                 (SELECT count(*)::int FROM designs d WHERE d.folder_id = f.id AND d.trashed_at IS NULL) AS designs
          FROM folders f JOIN workspaces w ON w.id = f.workspace_id
          WHERE {} ORDER BY {order} LIMIT ${}",
            wh.join(" AND "),
            params.len()
        ),
        params,
    }
}

/// The user's own uploads, as GET /api/assets lists them (never anyone else's).
pub fn uploads_sql(uid: Uuid, p: &SearchParams, now: DateTime<Utc>) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec!["owner_id = $1 AND trashed_at IS NULL AND source IN ('upload','gif')".to_owned()];
    if let Some(since) = modified_since(&p.modified, now) {
        params.push(Arg::Time(Some(since)));
        wh.push(format!("created_at >= ${}", params.len()));
    }
    let q = normalize_query(&p.q);
    if !q.is_empty() {
        wh.push(text_match("coalesce(name, '')", &q, &mut params, false));
    }
    let order = if q.is_empty() { "created_at DESC".to_owned() } else { format!("{}, created_at DESC", rank_by("coalesce(name, '')", &q, &mut params)) };
    params.push(Arg::Int(Some(p.limit)));
    Built {
        sql: format!(
            r#"SELECT id, mime, kind, bytes, width AS w, height AS h, name, sha256, duration_ms AS "durationMs", animated, created_at FROM assets
          WHERE {} ORDER BY {order} LIMIT ${}"#,
            wh.join(" AND "),
            params.len()
        ),
        params,
    }
}

/// Published templates as GET /api/templates shows them: the public catalogue plus my teams' templates.
pub fn templates_sql(uid: Uuid, p: &SearchParams, now: DateTime<Utc>) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec!["published".to_owned(), "(workspace_id IS NULL OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))".to_owned()];
    if let Some(since) = modified_since(&p.modified, now) {
        params.push(Arg::Time(Some(since)));
        wh.push(format!("updated_at >= ${}", params.len()));
    }
    let q = normalize_query(&p.q);
    if !q.is_empty() {
        wh.push(text_match("search_text", &q, &mut params, true));
    }
    let order = if q.is_empty() {
        "(workspace_id IS NOT NULL) DESC, uses DESC, title".to_owned()
    } else {
        format!("{}, (workspace_id IS NOT NULL) DESC, uses DESC, title", rank_by("title", &q, &mut params))
    };
    params.push(Arg::Int(Some(p.limit)));
    Built {
        sql: format!(
            r#"SELECT id, title, category, preset, tags, width, height, premium, workspace_id IS NOT NULL AS team, brand, workspace_id AS "workspaceId", version
          FROM templates WHERE {} ORDER BY {order} LIMIT ${}"#,
            wh.join(" AND "),
            params.len()
        ),
        params,
    }
}

/// Template titles, categories and tags visible to the user that contain the typed text.
pub fn suggest_sql(uid: Uuid, q: &str, limit: i64) -> Built {
    let params = vec![pg::uuid(uid), pg::text(format!("%{}%", like_escape(&normalize_query(q)))), Arg::Int(Some(limit))];
    let visible = "published AND (workspace_id IS NULL OR workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))";
    Built {
        sql: format!(
            "(SELECT 'template' AS kind, title AS text, id FROM templates WHERE {visible} AND title ILIKE $2 ESCAPE '\\' ORDER BY uses DESC, title LIMIT $3)
          UNION ALL
          (SELECT DISTINCT 'category' AS kind, category AS text, NULL AS id FROM templates WHERE {visible} AND category ILIKE $2 ESCAPE '\\' LIMIT $3)
          UNION ALL
          (SELECT DISTINCT 'tag' AS kind, tag AS text, NULL AS id FROM templates, unnest(tags) AS tag WHERE {visible} AND tag ILIKE $2 ESCAPE '\\' LIMIT $3)"
        ),
        params,
    }
}

/// A suggestion row (`{ kind, text, id? }`).
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub kind: String,
    pub text: String,
    pub id: Option<String>,
}

impl Suggestion {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("kind".into(), json!(self.kind));
        m.insert("text".into(), json!(self.text));
        if let Some(id) = &self.id {
            m.insert("id".into(), json!(id));
        }
        Value::Object(m)
    }
}

/// `a.localeCompare(b)` (ICU root collation, approximated: punctuation and spaces before digits before letters,
/// letters compared case-insensitively first, then lowercase before uppercase).
pub fn locale_cmp(a: &str, b: &str) -> Ordering {
    fn primary(c: char) -> (u8, String) {
        if c.is_numeric() {
            (1, c.to_string())
        } else if c.is_alphabetic() {
            (2, c.to_lowercase().collect())
        } else {
            (0, c.to_string())
        }
    }
    let pa: Vec<(u8, String)> = a.chars().map(primary).collect();
    let pb: Vec<(u8, String)> = b.chars().map(primary).collect();
    pa.cmp(&pb).then_with(|| {
        let ta: Vec<bool> = a.chars().map(char::is_uppercase).collect();
        let tb: Vec<bool> = b.chars().map(char::is_uppercase).collect();
        ta.cmp(&tb)
    })
}

/// Merge suggestion rows into one short list: case-insensitive de-duplication across kinds, prefix matches
/// first, then shorter text, and categories/tags before individual templates on a tie.
pub fn rank_suggestions(rows: &[Suggestion], q: &str, limit: usize) -> Vec<Suggestion> {
    let needle = normalize_query(q).to_lowercase();
    let kind_order = |k: &str| match k {
        "category" => 0,
        "tag" => 1,
        _ => 2,
    };
    let mut sorted = rows.to_vec();
    sorted.sort_by(|a, b| {
        let pa = u8::from(!a.text.to_lowercase().starts_with(&needle));
        let pb = u8::from(!b.text.to_lowercase().starts_with(&needle));
        pa.cmp(&pb).then_with(|| crate::js::len(&a.text).cmp(&crate::js::len(&b.text))).then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind))).then_with(|| locale_cmp(&a.text, &b.text))
    });
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for r in sorted {
        let key = crate::js::trim(&r.text).to_lowercase();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        out.push(r);
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(like_escape(r"50%_a\b"), r"50\%\_a\\b");
        assert_eq!(normalize_query("  brand \t kit \n"), "brand kit");
        assert_eq!(groups_for("all", "anyone"), vec!["designs", "folders", "uploads", "templates"]);
        assert_eq!(groups_for("all", "shared"), vec!["designs", "folders"]);
        assert_eq!(groups_for("templates", "me"), Vec::<&str>::new());
        let s = |k: &str, t: &str| Suggestion { kind: k.into(), text: t.into(), id: None };
        let got = rank_suggestions(&[s("template", "Sale flyer"), s("category", "flyers"), s("tag", "Flyers"), s("tag", "sale")], "fly", 8);
        assert_eq!(got.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), vec!["flyers", "sale", "Sale flyer"]);
    }
}
