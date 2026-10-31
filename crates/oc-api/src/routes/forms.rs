//! Port of services/api/src/routes/forms.ts — form responses (FM-001): public submission for published designs,
//! and the responses table (JSON or CSV) for editors.

use oc_auth::Role;
use serde_json::Map;
use sha2::{Digest, Sha256};

use crate::auth::{at_least, design_role};
use crate::http::ratelimit::MINUTE;
use crate::js;
use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[RouteLimit { method: "POST", path: "/api/forms/{design_id}/{element_id}", max: 10, window_ms: MINUTE }];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/forms/{design_id}/{element_id}", post(submit))
        .route("/api/designs/{id}/forms/{element_id}/responses", get(responses))
        .route("/api/designs/{id}/forms/{element_id}/responses/{rid}", delete(remove))
}

/// `formOf(designId, elementId)`: the live form element of the stored document.
async fn form_of(s: &AppState, design_id: &str, element_id: &str) -> Result<Option<Value>, ApiError> {
    let st: Option<Vec<u8>> = sqlx::query_scalar("SELECT state FROM doc_state WHERE design_id = $1").bind(design_id).fetch_optional(&s.pool).await?;
    let Some(st) = st else { return Ok(None) };
    let doc = oc_crdt::decode_doc_value(&st)?;
    let el = doc.get("elements").and_then(|e| e.get(element_id)).cloned();
    Ok(el.filter(|e| e.get("type").and_then(Value::as_str) == Some("form") && !truthy(e.get("deleted"))))
}

/// JS truthiness of a JSON value (`undefined` = `None`).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `String(v)` of a JSON value as a form value (strings, arrays of strings).
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js::num(n.as_f64().unwrap_or(f64::NAN)),
        Value::Array(a) => a.iter().map(|x| if x.is_null() { String::new() } else { js_string(x) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn received(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
    }
}

fn invalid_type(expected: &str, got: &str, path: Vec<Value>) -> Value {
    json!({ "expected": expected, "code": "invalid_type", "path": path, "message": format!("Invalid input: expected {expected}, received {got}") })
}

fn too_big(origin: &str, max: usize, unit: &str, path: Vec<Value>) -> Value {
    json!({ "origin": origin, "code": "too_big", "maximum": max, "inclusive": true, "path": path, "message": format!("Too big: expected {origin} to have <={max} {unit}") })
}

/// `z.record(z.string(), z.union([z.string().max(5000), z.array(z.string().max(500)).max(50)])).parse(body)`.
/// A union keeps the issues of the one option whose issues are all non-aborting checks (a type mismatch
/// aborts, and since zod 4.6.6 so does an array over its max, before any item is parsed); when no option
/// qualifies it reports `invalid_union` with every option's issues.
fn parse_body(body: &Value) -> Result<Map<String, Value>, ApiError> {
    let Value::Object(m) = body else {
        return Err(ApiError::Invalid(vec![invalid_type("record", received(body), vec![])]));
    };
    let mut issues = Vec::new();
    for (k, v) in m {
        let key = Value::String(k.clone());
        match v {
            Value::String(s) => {
                if js::len(s) > 5000 {
                    issues.push(too_big("string", 5000, "characters", vec![key]));
                }
            }
            Value::Array(a) if a.len() <= 50 && a.iter().all(Value::is_string) => {
                for (i, x) in a.iter().enumerate() {
                    if js::len(x.as_str().unwrap_or("")) > 500 {
                        issues.push(too_big("string", 500, "characters", vec![key.clone(), json!(i)]));
                    }
                }
            }
            other => {
                let string_branch = vec![invalid_type("string", received(other), vec![])];
                let array_branch: Vec<Value> = match other {
                    Value::Array(a) if a.len() > 50 => vec![too_big("array", 50, "items", vec![])],
                    Value::Array(a) => {
                        let mut out = Vec::new();
                        for (i, x) in a.iter().enumerate() {
                            match x {
                                Value::String(s) if js::len(s) > 500 => out.push(too_big("string", 500, "characters", vec![json!(i)])),
                                Value::String(_) => {}
                                y => out.push(invalid_type("string", received(y), vec![json!(i)])),
                            }
                        }
                        out
                    }
                    o => vec![invalid_type("array", received(o), vec![])],
                };
                issues.push(json!({ "code": "invalid_union", "errors": [string_branch, array_branch], "path": [key], "message": "Invalid input" }));
            }
        }
    }
    if issues.is_empty() { Ok(m.clone()) } else { Err(ApiError::Invalid(issues)) }
}

static EMAIL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("regex"));

async fn submit(State(s): State<AppState>, Path((design_id, element_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let body = parse_body(&req.body_or_empty())?;
    // Honeypot: bots fill every field.
    if truthy(body.get("_hp")) {
        return Ok(Reply::json(json!({ "ok": true })));
    }
    type Row = (bool, Uuid, String, Option<bool>);
    let d: Option<Row> = sqlx::query_as(
        "SELECT d.public_view, d.owner_id, d.title, s.published FROM designs d LEFT JOIN sites s ON s.design_id = d.id WHERE d.id = $1 AND d.trashed_at IS NULL",
    )
    .bind(&design_id)
    .fetch_optional(&s.pool)
    .await?;
    let Some((_, owner_id, title, _)) = d.filter(|d| d.0 || d.3 == Some(true)) else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let Some(form) = form_of(&s, &design_id, &element_id).await? else {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    };
    let fields: Vec<Value> = form.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut data = Map::new();
    let mut problems = Map::new();
    for f in &fields {
        let fid = f.get("id").map(js_string).unwrap_or_else(|| "undefined".into());
        let options: Option<Vec<Value>> = f.get("options").and_then(Value::as_array).cloned();
        let includes = |x: &Value| options.as_ref().is_some_and(|o| o.contains(x));
        let v = body.get(&fid);
        let value: Value = match v {
            Some(Value::Array(a)) => Value::Array(a.iter().filter(|x| includes(x)).cloned().collect()),
            Some(Value::String(t)) => Value::String(js::trim(t).to_owned()),
            _ => Value::String(String::new()),
        };
        let present = truthy(Some(&value));
        let empty_array = matches!(&value, Value::Array(a) if a.is_empty());
        let kind = f.get("kind").and_then(Value::as_str).unwrap_or("");
        let text = js_string(&value);
        if truthy(f.get("required")) && (!present || empty_array) {
            problems.insert(fid.clone(), json!("Required"));
        } else if kind == "email" && present && !EMAIL.is_match(&text) {
            problems.insert(fid.clone(), json!("Enter a valid email"));
        } else if kind == "number" && present && !z::js_string_to_number(&text).is_finite() {
            problems.insert(fid.clone(), json!("Enter a number"));
        } else if kind == "choice" && present && !includes(&Value::String(text.clone())) {
            problems.insert(fid.clone(), json!("Choose an option"));
        }
        if present && !empty_array {
            data.insert(fid, value);
        }
    }
    if !problems.is_empty() {
        return Ok(reply::json(400, json!({ "error": "invalid", "problems": problems })));
    }
    let ip_hash = js::take(&hex::encode(Sha256::digest(format!("{}:{}", s.cfg.secret, req.ip).as_bytes())), 32);
    let data_v = Value::Object(data.clone());
    sqlx::query("INSERT INTO form_responses(id, design_id, element_id, data, ip_hash) VALUES ($1,$2,$3,$4::jsonb,$5)")
        .bind(oc_db::uuid())
        .bind(&design_id)
        .bind(&element_id)
        .bind(crate::http::jsjson::stringify(&data_v))
        .bind(&ip_hash)
        .execute(&s.pool)
        .await?;
    let form_title = form.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("Form").to_owned();
    let owner_email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(owner_id).fetch_optional(&s.pool).await?;
    if let Some(email) = owner_email {
        let summary = fields
            .iter()
            .filter_map(|f| {
                let fid = f.get("id").map(js_string)?;
                let v = data.get(&fid)?;
                Some(format!("{}: {}", f.get("label").map(js_string).unwrap_or_else(|| "undefined".into()), js::take(&js_string(v), 200)))
            })
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        let t = oc_mail::Template::FormResponse {
            form: form_title.clone(),
            title: title.clone(),
            summary,
            url: format!("{}/design/{design_id}?responses={element_id}", s.cfg.web_origin),
        };
        let pool = s.pool.clone();
        s.spawn(async move {
            let _ = oc_mail::send_email(&pool, &email, &t, Some(owner_id)).await;
        });
    }
    let pool = s.pool.clone();
    let ev = json!({ "designId": design_id, "formId": element_id, "form": form_title, "data": data_v });
    s.spawn(async move {
        let _ = oc_webhooks::emit_event(&pool, owner_id, oc_webhooks::WebhookEvent::FormResponse, &ev).await;
    });
    let mut out = Map::new();
    out.insert("ok".into(), json!(true));
    if let Some(t) = form.get("thanks") {
        out.insert("message".into(), t.clone());
    }
    Ok(Reply::json(Value::Object(out)))
}

/// `csvCell(v)`: arrays joined with `; `, formulas neutralised (CSV injection), quoted when needed.
fn csv_cell(v: Option<&Value>) -> String {
    let s = match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { js_string(x) }).collect::<Vec<_>>().join("; "),
        Some(x) => js_string(x),
    };
    let safe = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s };
    if safe.contains(['"', ',', '\n']) { format!("\"{}\"", safe.replace('"', "\"\"")) } else { safe }
}

static FILE_SAFE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"[^A-Za-z0-9_\- ]+").expect("regex"));

async fn responses(State(s): State<AppState>, Path((id, element_id)): Path<(String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let form = form_of(&s, &id, &element_id).await?;
    let rows = pg::json_rows(
        &s.pool,
        "SELECT id, data, created_at FROM form_responses WHERE design_id = $1 AND element_id = $2 ORDER BY created_at DESC LIMIT 5000",
        &[pg::text(&id), pg::text(&element_id)],
    )
    .await?;
    let fields: Vec<Value> = form.as_ref().and_then(|f| f.get("fields")).and_then(Value::as_array).cloned().unwrap_or_default();
    if req.query_str("format") == Some("csv") {
        let label = |f: &Value| f.get("label").cloned();
        let mut head = vec![csv_cell(Some(&json!("Submitted")))];
        head.extend(fields.iter().map(|f| csv_cell(label(f).as_ref())));
        let mut lines = vec![head.join(",")];
        for r in &rows {
            let mut cells = vec![csv_cell(r.get("created_at"))];
            for f in &fields {
                let fid = f.get("id").map(js_string).unwrap_or_default();
                cells.push(csv_cell(r.get("data").and_then(|d| d.get(&fid))));
            }
            lines.push(cells.join(","));
        }
        let title = form.as_ref().and_then(|f| f.get("title")).and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("responses");
        let name = FILE_SAFE.replace_all(title, "");
        return Ok(Reply::bytes("text/csv; charset=utf-8", lines.join("\r\n")).header("content-disposition", format!("attachment; filename=\"{name}.csv\"")));
    }
    Ok(Reply::json(json!({ "fields": fields, "responses": rows })))
}

async fn remove(State(s): State<AppState>, Path((id, element_id, rid)): Path<(String, String, String)>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::Edit) {
        return Ok(Reply::empty(404));
    }
    pg::exec(&s.pool, "DELETE FROM form_responses WHERE id = $1::uuid AND design_id = $2 AND element_id = $3", &[pg::text(&rid), pg::text(&id), pg::text(&element_id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}
