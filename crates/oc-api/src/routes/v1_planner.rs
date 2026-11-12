//! Content Planner over the REST API, for tools that publish posts themselves (for example a team's own
//! Instagram/Facebook publisher) and for AI assistants that draft posts.
//!
//!   GET   /v1/planner/posts                list posts (`?due=1`: approved external posts whose time has come)
//!   GET   /v1/planner/posts/{id}           one post; `{id}` is the post's id or its `ref` field (e.g. `P-001`)
//!   POST  /v1/planner/posts                create a post (scope `planner:write`)
//!   PATCH /v1/planner/posts/{id}           change a post; an approved post goes back to review
//!   POST  /v1/planner/posts/{id}/claim     lock an approved, due external post for publishing (scope `planner:publish`)
//!   POST  /v1/planner/posts/{id}/results   report per-channel results and, with `done`, finish the post
//!
//! Approving stays a person's decision in the web app; the API cannot approve.

use axum::extract::Request;
use axum::response::Response;
use chrono::{DateTime, Utc};
use oc_social::publish::{PostRow, log_event};
use serde_json::Map;

use crate::prelude::*;
use crate::routes::social::{CHANNELS, create_post_as, public_post, update_post_as};
use crate::routes::v1::{ApiUser, Limit, api, need_any, not_found, respond};

pub const LIMITS: &[RouteLimit] = &[];

/// Scopes that may read the planner.
const READ: [&str; 3] = ["planner:read", "planner:write", "planner:publish"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/planner/posts", get(list).post(create))
        .route("/v1/planner/posts/{id}", get(read).patch(update))
        .route("/v1/planner/posts/{id}/claim", post(claim))
        .route("/v1/planner/posts/{id}/results", post(results))
}

/// The token's user sees the post (its owner, or a member of its workspace); a token limited to a workspace only
/// sees posts whose design lies in that workspace (personal posts have no `workspace_id` of their own).
const VISIBLE: &str = "(p.owner_id = $1 OR p.workspace_id IN (SELECT workspace_id FROM memberships WHERE user_id = $1))
  AND ($2::uuid IS NULL OR EXISTS (SELECT 1 FROM designs d WHERE d.id = p.design_id AND d.workspace_id = $2))";

fn is_uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok()
}

/// The post `key` (id or `ref`) the token's user may see. `Err(409 ambiguous_ref)` when a ref names several posts.
async fn find(s: &AppState, u: &ApiUser, key: &str) -> Result<Option<PostRow>, ApiError> {
    match find_for(s, u.id, u.workspace_id, key).await? {
        Ok(p) => Ok(p),
        Err(()) => Err(reply::json(409, json!({ "error": "ambiguous_ref", "message": "Several posts use this post ID; use the post's id." })).into()),
    }
}

/// The post `key` (its id, or its `ref` field such as `P-001`) that `user` may see, optionally only in workspace
/// `ws`. `Ok(Err(()))` when a ref names several posts.
pub(crate) async fn find_for(
    s: &AppState,
    user: Uuid,
    ws: Option<Uuid>,
    key: &str,
) -> Result<Result<Option<PostRow>, ()>, ApiError> {
    let key = key.trim();
    let rows: Vec<PostRow> = if is_uuid(key) {
        sqlx::query_as(&format!(
            "SELECT p.* FROM social_posts p WHERE {VISIBLE} AND p.id = $3::uuid"
        ))
        .bind(user)
        .bind(ws)
        .bind(key)
        .fetch_all(&s.pool)
        .await?
    } else {
        sqlx::query_as(&format!(
            "SELECT p.* FROM social_posts p WHERE {VISIBLE} AND p.fields->>'ref' = $3 LIMIT 2"
        ))
        .bind(user)
        .bind(ws)
        .bind(key)
        .fetch_all(&s.pool)
        .await?
    };
    if rows.len() > 1 {
        return Ok(Err(()));
    }
    Ok(Ok(rows.into_iter().next()))
}

/// A post as the API returns it: the web app's shape plus the design's current version, the post's `ref` and the
/// team (brand) it belongs to (`null` for a personal post).
pub(crate) async fn out(s: &AppState, p: &PostRow) -> Result<Value, ApiError> {
    let mut v = public_post(p, None);
    v["designVersion"] = json!(crate::routes::v1_export::content_version(s, &p.design_id).await?);
    v["ref"] = p.fields.0.get("ref").cloned().unwrap_or(Value::Null);
    v["designUrl"] = json!(format!("{}/design/{}", s.cfg.web_origin, p.design_id));
    let team: Option<String> = match p.workspace_id {
        Some(w) => sqlx::query_scalar("SELECT name FROM workspaces WHERE id = $1").bind(w).fetch_optional(&s.pool).await?,
        None => None,
    };
    v["workspaceId"] = json!(p.workspace_id.map(|w| w.to_string()));
    v["workspaceName"] = json!(team);
    Ok(v)
}

async fn list(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "GET /v1/planner/posts", Limit::Token).await?;
            need_any(&u, &READ)?;
            let q = z::obj([
                ("status", z::str().max(20.0).optional()),
                ("publisher", z::enumeration(&["pinselo", "external"]).optional()),
                ("due", z::enumeration(&["1", "true", "0", "false"]).optional()),
                ("from", z::str().datetime_offset().optional()),
                ("to", z::str().datetime_offset().optional()),
            ])
            .parse(Some(&req.query))?;
            let qs = |k: &str| q.get(k).and_then(Value::as_str).map(str::to_owned);
            let due = matches!(qs("due").as_deref(), Some("1" | "true"));
            let rows: Vec<PostRow> = sqlx::query_as(&format!(
                "SELECT p.* FROM social_posts p WHERE {VISIBLE}
                   AND ($3::text IS NULL OR p.status = $3)
                   AND ($4::text IS NULL OR p.publisher = $4)
                   AND (NOT $5 OR (p.publisher = 'external' AND p.status = 'approved' AND p.scheduled_at <= now()))
                   AND ($6::timestamptz IS NULL OR p.scheduled_at >= $6::timestamptz)
                   AND ($7::timestamptz IS NULL OR p.scheduled_at < $7::timestamptz)
                 ORDER BY p.scheduled_at NULLS LAST, p.created_at, p.id LIMIT 500"
            ))
            .bind(u.id)
            .bind(u.workspace_id)
            .bind(qs("status"))
            .bind(qs("publisher"))
            .bind(due)
            .bind(qs("from"))
            .bind(qs("to"))
            .fetch_all(&s.pool)
            .await?;
            let mut posts = Vec::with_capacity(rows.len());
            for p in &rows {
                posts.push(out(&s, p).await?);
            }
            Ok(Reply::json(json!({ "posts": posts })))
        }
        .await,
    )
}

async fn read(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "GET /v1/planner/posts/:id", Limit::Token).await?;
            need_any(&u, &READ)?;
            let Some(p) = find(&s, &u, &id).await? else {
                return Ok(not_found());
            };
            Ok(Reply::json(json!({ "post": out(&s, &p).await? })))
        }
        .await,
    )
}

async fn api_reply(s: &AppState, r: crate::routes::social::PostOutcome) -> ApiResult {
    Ok(match r? {
        Ok(p) => Reply::json(json!({ "post": out(s, &p).await? })),
        Err(refusal) => refusal.reply(),
    })
}

async fn create(State(s): State<AppState>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/planner/posts", Limit::Token).await?;
            need_any(&u, &["planner:write"])?;
            api_reply(
                &s,
                create_post_as(&s, u.id, req.body(), u.workspace_id).await,
            )
            .await
        }
        .await,
    )
}

async fn update(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "PATCH /v1/planner/posts/:id", Limit::Token).await?;
            need_any(&u, &["planner:write"])?;
            let Some(p) = find(&s, &u, &id).await? else {
                return Ok(not_found());
            };
            api_reply(&s, update_post_as(&s, u.id, p, req.body()).await).await
        }
        .await,
    )
}

async fn claim(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, _req) = api(&s, raw, "POST /v1/planner/posts/:id/claim", Limit::Token).await?;
            need_any(&u, &["planner:publish"])?;
            let Some(p) = find(&s, &u, &id).await? else { return Ok(not_found()) };
            let version = crate::routes::v1_export::content_version(&s, &p.design_id).await?;
            // The design changed after it was approved: what would be posted is not what was approved.
            if p.status == "approved" && p.approved_version.is_some() && version != p.approved_version {
                let back: Option<PostRow> = sqlx::query_as(
                    "UPDATE social_posts SET status = 'in_review', approved_by = NULL, approved_at = NULL, approved_version = NULL, updated_at = now()
                     WHERE id = $1 AND status = 'approved' RETURNING *",
                )
                .bind(p.id)
                .fetch_optional(&s.pool)
                .await?;
                if back.is_some() {
                    let msg = format!("The design changed after approval (approved version {}, now {}); review it again.", p.approved_version.unwrap_or(0), version.unwrap_or(0));
                    log_event(&s.pool, p.id, "changes_requested", Some(&msg), None, None).await?;
                }
                return Ok(reply::json(
                    409,
                    json!({ "error": "version_changed", "message": "The design was edited after approval; the post went back to review.", "approvedVersion": p.approved_version, "currentVersion": version }),
                ));
            }
            let token = format!("clm_{}", oc_db::uuid().simple());
            let claimed: Option<PostRow> = sqlx::query_as(
                "UPDATE social_posts SET status = 'publishing', claim_token = $2, claimed_at = now(), error = NULL, updated_at = now()
                 WHERE id = $1 AND publisher = 'external' AND status = 'approved' AND scheduled_at <= now() RETURNING *",
            )
            .bind(p.id)
            .bind(&token)
            .fetch_optional(&s.pool)
            .await?;
            let Some(post) = claimed else {
                let reason = if p.publisher != "external" {
                    "Pinselo publishes this post itself."
                } else if p.status != "approved" {
                    "Only approved posts can be claimed."
                } else {
                    "This post is not due yet."
                };
                return Ok(reply::json(409, json!({ "error": "not_claimable", "message": reason, "status": p.status })));
            };
            log_event(&s.pool, post.id, "claimed", Some("Claimed by a publishing tool"), None, None).await?;
            Ok(Reply::json(json!({ "claimToken": token, "post": out(&s, &post).await? })))
        }
        .await,
    )
}

async fn results(State(s): State<AppState>, Path(id): Path<String>, raw: Request) -> Response {
    respond(
        async {
            let (u, req) = api(&s, raw, "POST /v1/planner/posts/:id/results", Limit::Token).await?;
            need_any(&u, &["planner:publish"])?;
            let b = z::obj([
                ("claimToken", z::str().min(1.0).max(100.0)),
                (
                    "results",
                    z::record(z::obj([
                        ("status", z::enumeration(&["published", "failed", "unknown"])),
                        ("id", z::str().max(200.0).optional()),
                        ("url", z::str().max(2000.0).optional()),
                        ("error", z::str().max(4000.0).optional()),
                    ]))
                    .default(json!({})),
                ),
                ("images", z::array(z::str().max(2000.0)).max(10.0).optional()),
                ("done", z::bool().default(json!(false))),
            ])
            .parse(req.body())?;
            let Some(p) = find(&s, &u, &id).await? else { return Ok(not_found()) };
            let token = b.get("claimToken").and_then(Value::as_str).unwrap_or("");
            if p.status != "publishing" || p.claim_token.as_deref() != Some(token) {
                return Ok(reply::json(409, json!({ "error": "claim_lost", "message": "This post is not claimed with this token." })));
            }
            let incoming = b.get("results").and_then(Value::as_object).cloned().unwrap_or_default();
            if let Some(bad) = incoming.keys().find(|k| !p.channels.contains(k) || !CHANNELS.contains(&k.as_str())) {
                return Ok(reply::json(400, json!({ "error": "unknown_channel", "message": format!("{bad} is not a channel of this post.") })));
            }
            let now = oc_js::iso_string(&DateTime::<Utc>::from_timestamp_millis(s.now_ms()).unwrap_or_default());
            let mut merged: Map<String, Value> = p.results.0.clone();
            for (channel, r) in &incoming {
                let mut r = r.clone();
                r["at"] = json!(now);
                let status = r.get("status").and_then(Value::as_str).unwrap_or("").to_owned();
                let message = r.get("error").and_then(Value::as_str).or_else(|| r.get("url").and_then(Value::as_str)).map(str::to_owned);
                log_event(&s.pool, p.id, &status, message.as_deref(), Some(json!({ "channel": channel })), None).await?;
                merged.insert(channel.clone(), r);
            }
            let mut fields = p.fields.0.clone();
            if let Some(images) = b.get("images").filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
                fields.insert("imageUrls".into(), json!(images.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"))));
            }
            let done = b.get("done").and_then(Value::as_bool).unwrap_or(false);
            let all_published = p.channels.iter().all(|c| merged.get(c).and_then(|r| r.get("status")).and_then(Value::as_str) == Some("published"));
            let (status, error) = if !done {
                ("publishing".to_owned(), None)
            } else if all_published {
                ("published".to_owned(), None)
            } else {
                let errs: Vec<String> = p
                    .channels
                    .iter()
                    .filter_map(|c| {
                        let r = merged.get(c)?;
                        match r.get("status").and_then(Value::as_str) {
                            Some("published") => None,
                            Some("unknown") => Some(format!("{c}: result unknown, check it by hand")),
                            _ => Some(format!("{c}: {}", r.get("error").and_then(Value::as_str).unwrap_or("not published"))),
                        }
                    })
                    .collect();
                ("failed".to_owned(), Some(errs.join(" · ")))
            };
            let updated: PostRow = sqlx::query_as(
                "UPDATE social_posts SET results = $2::jsonb, fields = $3::jsonb, status = $4, error = $5,
                   published_at = CASE WHEN $4 = 'published' THEN now() ELSE published_at END,
                   claim_token = CASE WHEN $4 = 'publishing' THEN claim_token ELSE NULL END, updated_at = now()
                 WHERE id = $1 AND claim_token = $6 RETURNING *",
            )
            .bind(p.id)
            .bind(crate::http::jsjson::stringify(&Value::Object(merged)))
            .bind(crate::http::jsjson::stringify(&Value::Object(fields)))
            .bind(&status)
            .bind(&error)
            .bind(token)
            .fetch_optional(&s.pool)
            .await?
            .ok_or_else(|| ApiError::status(409, "claim_lost"))?;
            if done {
                log_event(&s.pool, p.id, if status == "published" { "published" } else { "failed" }, error.as_deref(), None, None).await?;
            }
            Ok(Reply::json(json!({ "post": out(&s, &updated).await? })))
        }
        .await,
    )
}
