//! Port of services/api/src/uploads.ts — uploads library (g19) and asset trash (g22): the listing and trash
//! queries behind GET /api/assets, GET /api/assets/trash and the trash/restore/delete actions. Every statement is
//! scoped by `owner_id`; every user value is a bind parameter.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::pg::{self, Arg};
use crate::search::{Built, like_escape, normalize_query};
use crate::zod as z;

/// Days a trashed upload stays restorable (the worker's maintenance purge).
pub const TRASH_DAYS: i64 = 30;
pub const UPLOAD_TYPES: [&str; 4] = ["images", "videos", "audio", "files"];

/// `uploadsQuery`.
pub fn uploads_query() -> z::Schema {
    z::obj([
        ("q", z::str().max(200.0).default(json!(""))),
        ("type", z::enumeration(&["all", "images", "videos", "audio", "files"]).default(json!("all"))),
        ("sort", z::enumeration(&["newest", "oldest", "name", "size"]).default(json!("newest"))),
        ("limit", z::coerce_num().int().min(1.0).max(500.0).default(json!(500))),
        ("offset", z::coerce_num().int().min(0.0).max(100_000.0).default(json!(0))),
        ("id", z::str().min(1.0).max(64.0).optional()),
    ])
}

/// `batchBody`.
pub fn batch_body() -> z::Schema {
    z::obj([("action", z::enumeration(&["trash", "restore", "delete"])), ("ids", z::array(z::str().min(1.0).max(64.0)).min(1.0).max(500.0))])
}

/// `renameBody`.
pub fn rename_body() -> z::Schema {
    z::obj([("name", z::str().trim().min(1.0).max(200.0))])
}

fn type_sql(t: &str) -> &'static str {
    match t {
        "images" => "(kind IN ('image','svg') OR (kind = 'video' AND animated))",
        "videos" => "(kind = 'video' AND NOT animated)",
        "audio" => "kind = 'audio'",
        _ => "kind NOT IN ('image','svg','video','audio')",
    }
}

fn order(sort: &str, trashed: bool) -> &'static str {
    match (sort, trashed) {
        ("newest", true) => "trashed_at DESC, id",
        ("oldest", true) => "trashed_at ASC, id",
        ("oldest", false) => "created_at ASC, id",
        ("name", _) => "lower(coalesce(name, '')) ASC, created_at DESC, id",
        ("size", _) => "bytes DESC, created_at DESC, id",
        _ => "created_at DESC, id",
    }
}

fn scope(trashed: bool) -> &'static str {
    if trashed { "owner_id = $1 AND trashed_at IS NOT NULL AND moderation <> 'removed'" } else { "owner_id = $1 AND trashed_at IS NULL AND source IN ('upload','gif')" }
}

const COLUMNS: &str = r#"id, mime, kind, bytes, width AS w, height AS h, name, sha256, duration_ms AS "durationMs", has_audio AS "hasAudio", animated, source, attribution, created_at"#;

fn s<'a>(p: &'a Value, k: &str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `uploadsListSql(uid, p, { trashed })`.
pub fn uploads_list_sql(uid: Uuid, p: &Value, trashed: bool) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec![scope(trashed).to_owned()];
    let q = normalize_query(s(p, "q"));
    if !q.is_empty() {
        params.push(pg::text(format!("%{}%", like_escape(&q))));
        wh.push(format!("coalesce(name, '') ILIKE ${} ESCAPE '\\'", params.len()));
    }
    let t = s(p, "type");
    if t != "all" && !t.is_empty() {
        wh.push(type_sql(t).to_owned());
    }
    if let Some(id) = p.get("id").and_then(Value::as_str) {
        params.push(pg::text(id));
        wh.push(format!("id = ${}", params.len()));
    }
    params.push(Arg::Int(p.get("limit").and_then(Value::as_f64).map(|n| n as i64)));
    params.push(Arg::Int(p.get("offset").and_then(Value::as_f64).map(|n| n as i64)));
    let extra = if trashed { format!(", trashed_at, trashed_at + interval '{TRASH_DAYS} days' AS purge_at, (moderation <> 'ok') AS held") } else { String::new() };
    let n = params.len();
    Built {
        sql: format!("SELECT {COLUMNS}{extra} FROM assets WHERE {}\n          ORDER BY {} LIMIT ${} OFFSET ${n}", wh.join(" AND "), order(s(p, "sort"), trashed), n - 1),
        params,
    }
}

/// `uploadCountsSql(uid, p, { trashed })`: per-type counts, honouring the name filter but not the type filter.
pub fn upload_counts_sql(uid: Uuid, p: &Value, trashed: bool) -> Built {
    let mut params = vec![pg::uuid(uid)];
    let mut wh = vec![scope(trashed).to_owned()];
    let q = normalize_query(s(p, "q"));
    if !q.is_empty() {
        params.push(pg::text(format!("%{}%", like_escape(&q))));
        wh.push(format!("coalesce(name, '') ILIKE ${} ESCAPE '\\'", params.len()));
    }
    let counts: Vec<String> = UPLOAD_TYPES.iter().map(|t| format!("count(*) FILTER (WHERE {})::int AS {t}", type_sql(t))).collect();
    Built { sql: format!("SELECT count(*)::int AS \"all\", {} FROM assets WHERE {}", counts.join(", "), wh.join(" AND ")), params }
}

pub const RESTORABLE_SQL: &str = "trashed_at IS NOT NULL AND moderation = 'ok'";

/// `batchSql(uid, action, ids)`.
pub fn batch_sql(uid: Uuid, action: &str, ids: &[String]) -> Built {
    let params = vec![Arg::TextArray(Some(ids.to_vec())), pg::uuid(uid)];
    let mine = "id = ANY($1::text[]) AND owner_id = $2";
    let sql = match action {
        "trash" => format!("UPDATE assets SET trashed_at = now() WHERE {mine} AND trashed_at IS NULL RETURNING id"),
        "restore" => format!("UPDATE assets SET trashed_at = NULL WHERE {mine} AND {RESTORABLE_SQL} RETURNING id"),
        _ => format!("DELETE FROM assets WHERE {mine} AND {RESTORABLE_SQL} RETURNING id"),
    };
    Built { sql, params }
}

/// `restoreBytesSql(uid, ids)`.
pub fn restore_bytes_sql(uid: Uuid, ids: &[String]) -> Built {
    Built {
        sql: format!("SELECT coalesce(sum(bytes),0)::bigint AS total FROM assets WHERE id = ANY($1::text[]) AND owner_id = $2 AND {RESTORABLE_SQL}"),
        params: vec![Arg::TextArray(Some(ids.to_vec())), pg::uuid(uid)],
    }
}

fn ext(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        _ => "bin",
    }
}

/// `downloadName(name, mime)`: the upload's name with the extension of the bytes actually sent.
pub fn download_name(name: Option<&str>, mime: &str) -> String {
    let cleaned: String = name.unwrap_or("").chars().filter(|&c| !(c == '\\' || c == '/' || c == '"' || (c as u32) <= 0x1f || c as u32 == 0x7f)).collect();
    let mut base = crate::js::trim(&cleaned).to_owned();
    // /\.[a-z0-9]{1,5}$/i
    if let Some(dot) = base.rfind('.') {
        let tail = &base[dot + 1..];
        if (1..=5).contains(&tail.len()) && tail.bytes().all(|c| c.is_ascii_alphanumeric()) {
            base.truncate(dot);
        }
    }
    let base = crate::js::take(&base, 150);
    let base = if base.is_empty() { "upload".to_owned() } else { base };
    format!("{base}.{}", ext(mime))
}

/// `contentDisposition(filename)`: `attachment` with an ASCII fallback and the UTF-8 name (RFC 6266 / 5987).
pub fn content_disposition(filename: &str) -> String {
    // filename.replace(/[^\x20-\x7e]/g, '_') works on UTF-16 units: an astral character becomes two underscores.
    let mut ascii = String::new();
    for u in filename.encode_utf16() {
        let c = if (0x20..=0x7e).contains(&u) { u as u8 as char } else { '_' };
        ascii.push(if c == '%' || c == ';' || c == '\\' { '_' } else { c });
    }
    let enc: String = crate::js::encode_uri_component(filename)
        .chars()
        .map(|c| match c {
            '\'' => "%27".to_owned(),
            '(' => "%28".to_owned(),
            ')' => "%29".to_owned(),
            '*' => "%2A".to_owned(),
            c => c.to_string(),
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{enc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(download_name(Some("a/b\"c.PNG"), "image/png"), "abc.png");
        assert_eq!(download_name(None, "x/y"), "upload.bin");
        assert_eq!(content_disposition("café (1).png"), "attachment; filename=\"caf_ (1).png\"; filename*=UTF-8''caf%C3%A9%20%281%29.png");
    }
}
