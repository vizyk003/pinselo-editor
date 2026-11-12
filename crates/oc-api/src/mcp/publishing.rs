//! MCP tools for posts Pinselo publishes itself (planner `publisher: "pinselo"`): which connected accounts an
//! assistant can schedule to and whether they work, the networks' posting limits, and what a post did once it
//! went out. Drafting and submitting stay in `create_planner_post` / `update_planner_post`; approving and
//! publishing stay with people and the worker (an assistant never publishes).

use serde_json::{Map, Value, json};
use uuid::Uuid;

use oc_social::adapters::meta::graph_base;
use oc_social::apps::http_for_account;
use oc_social::http::{Req, js_string, non_null, with_query};
use oc_social::store::{AccountRow, tokens_for, visible_accounts};

use super::tools::{Fail, R, TeamFilter, need, opt_str, resolve_team, text, tool_err};
use crate::bearer::BearerUser;
use crate::state::AppState;

fn db(e: sqlx::Error) -> Fail {
    Fail::from(e)
}

/// Connected accounts the user can schedule to (own ones and those shared with their teams), without the
/// built-in "Download & remind".
async fn accounts(s: &AppState, u: &BearerUser) -> Result<Vec<AccountRow>, Fail> {
    Ok(visible_accounts(&s.pool, u.id).await.map_err(db)?.into_iter().filter(|a| a.provider != "local").collect())
}

async fn team_names(s: &AppState, ids: impl Iterator<Item = Uuid>) -> Result<Map<String, Value>, Fail> {
    let ids: Vec<Uuid> = ids.collect();
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM workspaces WHERE id = ANY($1)").bind(&ids).fetch_all(&s.pool).await.map_err(db)?;
    Ok(rows.into_iter().map(|(id, n)| (id.to_string(), Value::from(n))).collect())
}

fn account_view(r: &AccountRow, teams: &Map<String, Value>, me: Uuid) -> Value {
    json!({
        "id": r.id.to_string(),
        "network": r.provider,
        "name": r.display_name,
        "handle": r.handle,
        "team": r.workspace_id.map(|w| teams.get(&w.to_string()).cloned().unwrap_or(Value::from(w.to_string()))),
        "status": if r.status == "ok" { "ok" } else { "needs reconnecting" },
        "connected_via": if r.connected_via == "system_user" { "Meta system-user token" } else { "login" },
        "team_meta_app": r.social_app_id.is_some(),
        "mine": r.user_id == me,
    })
}

/// `list_social_accounts { team? }`.
pub async fn list(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:read")?;
    let filter = resolve_team(s, u, a).await?;
    let rows: Vec<AccountRow> = accounts(s, u)
        .await?
        .into_iter()
        .filter(|r| match filter {
            TeamFilter::All => true,
            TeamFilter::Personal => r.workspace_id.is_none(),
            TeamFilter::Team(t) => r.workspace_id == Some(t),
        })
        .collect();
    let teams = team_names(s, rows.iter().filter_map(|r| r.workspace_id)).await?;
    // Whether this server (or the account's team app) can publish to it at all.
    let base = crate::routes::social::http(s);
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut v = account_view(r, &teams, u.id);
        let configured = match oc_social::adapter(&r.provider) {
            Some(ad) => ad.configured(&http_for_account(&s.pool, &base, &s.cfg.secret, r).await.map_err(|e| Fail::Other(e.message))?),
            None => false,
        };
        v["can_publish"] = Value::from(configured && r.status == "ok");
        out.push(v);
    }
    let hint = if out.is_empty() {
        "No connected accounts. A person connects them in Settings → Connected accounts (a team can use its own Meta app there)."
    } else {
        "Pass account ids as accounts to create_planner_post. A person approves the post; Pinselo publishes it at its time."
    };
    Ok(text(&json!({ "accounts": out, "note": hint })))
}

/// The one account `key` names: its id, or its @handle or name (case-insensitive).
async fn find_account(s: &AppState, u: &BearerUser, key: &str) -> Result<AccountRow, Fail> {
    let k = key.trim().trim_start_matches('@').to_lowercase();
    let rows = accounts(s, u).await?;
    let hits: Vec<&AccountRow> = rows
        .iter()
        .filter(|r| r.id.to_string() == k || r.handle.as_deref().is_some_and(|h| h.to_lowercase() == k) || r.display_name.to_lowercase() == k)
        .collect();
    match hits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => tool_err(format!("No connected account {key}. list_social_accounts shows the ones you can use.")),
        many => tool_err(format!(
            "{key} matches {} accounts ({}); pass the account's id.",
            many.len(),
            many.iter().map(|r| format!("{} {}", r.provider, r.id)).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The account's access token and the Graph client of the app it was connected through.
async fn graph_access(s: &AppState, r: &AccountRow) -> Result<(oc_social::Http, String), Fail> {
    let http = http_for_account(&s.pool, &crate::routes::social::http(s), &s.cfg.secret, r).await.map_err(|e| Fail::Other(e.message))?;
    let t = tokens_for(&s.pool, &http, &s.cfg.secret, r, false, s.now_ms() as f64).await.map_err(|e| Fail::Tool(e.message))?;
    Ok((http, t.access_token))
}

/// `check_publishing_limit { account }`: Instagram's API publishing quota (every API post counts: images,
/// carousels as one, Reels), or the reels a Facebook Page published in the last 24 hours (Meta allows 30).
pub async fn limit(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:read")?;
    let r = find_account(s, u, &opt_str(a, "account").unwrap_or_default()).await?;
    let (http, token) = graph_access(s, &r).await?;
    let g = graph_base(&http);
    match r.provider.as_str() {
        "instagram" => {
            let url = with_query(&format!("{g}/{}/content_publishing_limit", r.external_id), &[("fields", Some("quota_usage,config"))]);
            let res = http.call(&url, Req::get().bearer(&token)).await.map_err(|e| Fail::Tool(format!("Meta: {}", e.message)))?;
            let item = res.body.get("data").and_then(|d| d.get(0)).cloned().unwrap_or(Value::Null);
            let used = item.get("quota_usage").and_then(Value::as_i64);
            let total = item.get("config").and_then(|c| c.get("quota_total")).and_then(Value::as_i64);
            Ok(text(&json!({
                "account": r.display_name, "network": "instagram", "window": "24 hours",
                "used": used, "total": total, "remaining": used.zip(total).map(|(u, t)| t - u),
                "counts": "every post published through the API: images, carousels (as one) and Reels",
            })))
        }
        "facebook" => {
            let url = with_query(&format!("{g}/{}/video_reels", r.external_id), &[("fields", Some("id,created_time,updated_time")), ("limit", Some("50"))]);
            let res = http.call(&url, Req::get().bearer(&token)).await.map_err(|e| Fail::Tool(format!("Meta: {}", e.message)))?;
            let since = s.now_ms() - 24 * 3_600_000;
            let used = match non_null(res.body.get("data")) {
                Some(Value::Array(list)) => list
                    .iter()
                    .filter(|v| {
                        let t = js_string(non_null(v.get("created_time")).or(v.get("updated_time")));
                        chrono::DateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S%z").is_ok_and(|d| d.timestamp_millis() >= since)
                    })
                    .count() as i64,
                _ => 0,
            };
            Ok(text(&json!({
                "account": r.display_name, "network": "facebook", "window": "24 hours",
                "reels_used": used, "reels_total": 30, "reels_remaining": 30 - used,
                "counts": "reels on the Page (Meta's limit for API-published reels; reels posted by hand count here too)",
            })))
        }
        other => tool_err(format!("Posting limits are available for Instagram and Facebook accounts, not {other}.")),
    }
}

/// `get_post_results { post }`: each target's outcome (link, error) and, for published Instagram posts, the
/// numbers (reach, saves, likes, comments, shares).
pub async fn results(s: &AppState, u: &BearerUser, a: &Value) -> R {
    need(u, "designs:read")?;
    let key = opt_str(a, "post").unwrap_or_default();
    let p = match crate::routes::v1_planner::find_for(s, u.id, None, &key).await.map_err(Fail::from)? {
        Ok(Some(p)) => p,
        Ok(None) => return tool_err(format!("No planner post {key}.")),
        Err(()) => return tool_err(format!("Several posts use the post ID {key}; use the post's id.")),
    };
    let mut out = json!({
        "post": p.fields.0.get("ref").cloned().unwrap_or(Value::from(p.id.to_string())),
        "id": p.id.to_string(),
        "status": p.status,
        "publisher": p.publisher,
        "scheduled_at": p.scheduled_at.map(|t| oc_js::iso_string(&t)),
        "published_at": p.published_at.map(|t| oc_js::iso_string(&t)),
    });
    if p.publisher == "external" {
        // Results an outside publisher reported, per channel.
        out["targets"] = Value::Object(p.results.0.clone());
        out["note"] = Value::from("Published by an outside tool; its numbers are in that tool.");
        return Ok(text(&out));
    }
    let rows: Vec<AccountRow> = sqlx::query_as("SELECT * FROM social_accounts WHERE id = ANY($1)").bind(&p.accounts).fetch_all(&s.pool).await.map_err(db)?;
    let ids: Vec<(Uuid, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT ON (account_id) account_id, data->>'externalId' FROM social_post_events
         WHERE post_id = $1 AND kind = 'published' AND account_id IS NOT NULL ORDER BY account_id, id DESC",
    )
    .bind(p.id)
    .fetch_all(&s.pool)
    .await
    .map_err(db)?;
    let mut targets = Vec::new();
    for acc in &p.accounts {
        let r = p.results.0.get(&acc.to_string()).cloned().unwrap_or(json!({ "status": "pending" }));
        let row = rows.iter().find(|x| x.id == *acc);
        let mut t = json!({
            "account": row.map_or_else(|| "(disconnected)".to_owned(), |x| x.display_name.clone()),
            "network": row.map(|x| x.provider.clone()),
            "status": r.get("status").cloned().unwrap_or(Value::Null),
        });
        for k in ["url", "error", "note", "at"] {
            if let Some(v) = non_null(r.get(k)) {
                t[k] = v.clone();
            }
        }
        let media = ids.iter().find(|(id, _)| id == acc).and_then(|(_, m)| m.clone());
        if let Some(m) = &media {
            t["post_id"] = Value::from(m.clone());
        }
        if let (Some(row), Some(m)) = (row, media.filter(|_| r.get("status").and_then(Value::as_str) == Some("published")))
            && row.provider == "instagram"
        {
            match instagram_numbers(s, row, &m).await {
                Ok(n) => t["numbers"] = n,
                Err(e) => t["numbers_error"] = Value::from(e),
            }
        }
        targets.push(t);
    }
    out["targets"] = Value::Array(targets);
    Ok(text(&out))
}

async fn instagram_numbers(s: &AppState, row: &AccountRow, media: &str) -> Result<Value, String> {
    let (http, token) = graph_access(s, row).await.map_err(|e| match e {
        Fail::Tool(m) | Fail::Other(m) => m,
    })?;
    let g = graph_base(&http);
    let url = with_query(&format!("{g}/{media}"), &[("fields", Some("permalink,timestamp,media_type,like_count,comments_count"))]);
    let m = http.call(&url, Req::get().bearer(&token)).await.map_err(|e| format!("Meta: {}", e.message))?;
    let mut n = json!({
        "permalink": m.body.get("permalink"), "media_type": m.body.get("media_type"),
        "likes": m.body.get("like_count"), "comments": m.body.get("comments_count"),
    });
    let url = with_query(&format!("{g}/{media}/insights"), &[("metric", Some("reach,saved,likes,comments,shares"))]);
    match http.call(&url, Req::get().bearer(&token)).await {
        Ok(ins) => {
            if let Some(Value::Array(list)) = non_null(ins.body.get("data")) {
                for m in list {
                    let name = js_string(m.get("name"));
                    let v = m.get("total_value").and_then(|t| t.get("value")).or_else(|| m.get("values").and_then(|v| v.get(0)).and_then(|v| v.get("value")));
                    n[if name == "saved" { "saves".to_owned() } else { name }] = v.cloned().unwrap_or(Value::Null);
                }
            }
        }
        Err(e) => n["insights_error"] = Value::from(format!("Meta: {}", e.message)),
    }
    Ok(n)
}
