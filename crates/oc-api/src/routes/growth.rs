//! Port of services/api/src/routes/growth.ts — consent (PL-008), analytics intake and staff metrics (PL-040),
//! support tickets (PL-050), public status page with incidents (PL-052).

use crate::analytics::{CLIENT_EVENTS, CONSENT_VERSION, analytics_consent};
use crate::http::ratelimit::MINUTE;
use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[
    RouteLimit { method: "POST", path: "/api/events", max: 120, window_ms: MINUTE },
    RouteLimit { method: "POST", path: "/api/support", max: 5, window_ms: 10 * MINUTE },
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/consent", post(consent_set).get(consent_get))
        .route("/api/events", post(events))
        .route("/api/support", post(support_new).get(support_list))
        .route("/api/support/{id}/reply", post(support_reply))
        .route("/api/status", get(status))
        .route("/api/admin/analytics", get(admin_analytics))
        .route("/api/admin/support", get(admin_support))
        .route("/api/admin/support/{id}/reply", post(admin_support_reply))
        .route("/api/admin/incidents", post(incident_new))
        .route("/api/admin/incidents/{id}/updates", post(incident_update))
}

const COMPONENTS: [(&str, &str); 6] = [
    ("web", "Website and editor"),
    ("api", "API and sign-in"),
    ("collab", "Real-time collaboration"),
    ("render", "Exports and thumbnails"),
    ("worker", "Email and background jobs"),
    ("database", "Storage"),
];

const TOPICS: [&str; 8] = ["account", "billing", "editor", "export", "collaboration", "bug", "feature", "other"];

async fn decision(s: &AppState, to: &str, subject: String, body: String) -> Result<(), ApiError> {
    oc_mail::send_email(&s.pool, to, &oc_mail::Template::Decision { subject, body }, None).await?;
    Ok(())
}

/// `id.slice(0, 8).toUpperCase()`.
fn ticket_ref(id: &str) -> String {
    crate::js::take(id, 8).to_uppercase()
}

// ---------------------------------------------------------------- consent

async fn consent_set(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([("anonId", z::str().max(64.0).optional()), ("analytics", z::bool()), ("marketing", z::bool().default(json!(false)))]).parse(req.body())?;
    let u = auth::current_user(&s, &req).await?;
    let uid = u.as_ref().map(|u| u.id);
    let anon = b.get("anonId").and_then(Value::as_str);
    let analytics = b["analytics"].as_bool().unwrap_or(false);
    for (kind, granted) in [("analytics", analytics), ("marketing", b["marketing"].as_bool().unwrap_or(false))] {
        sqlx::query("INSERT INTO consents(user_id, anon_id, kind, granted, version) VALUES ($1,$2,$3,$4,$5)")
            .bind(uid)
            .bind(anon)
            .bind(kind)
            .bind(granted)
            .bind(CONSENT_VERSION)
            .execute(&s.pool)
            .await?;
    }
    // Withdrawing consent also detaches past events from the person.
    if let (false, Some(uid)) = (analytics, uid) {
        sqlx::query("UPDATE analytics_events SET user_id = NULL WHERE user_id = $1").bind(uid).execute(&s.pool).await?;
    }
    Ok(Reply::json(json!({ "ok": true, "version": CONSENT_VERSION })))
}

async fn consent_get(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = auth::current_user(&s, &req).await?;
    let granted = analytics_consent(&s, u.map(|u| u.id), req.query_str("anonId")).await?;
    Ok(Reply::json(json!({ "analytics": granted, "version": CONSENT_VERSION })))
}

// ---------------------------------------------------------------- analytics intake

static PATH_TAIL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"[?#].*$").expect("regex"));

async fn events(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([
        ("anonId", z::str().max(64.0)),
        ("events", z::array(z::obj([("name", z::str().max(40.0)), ("props", z::record(z::any()).optional()), ("path", z::str().max(300.0).optional())])).max(50.0)),
    ])
    .parse(req.body())?;
    let u = auth::current_user(&s, &req).await?;
    let uid = u.map(|u| u.id);
    let anon = b["anonId"].as_str().unwrap_or("");
    if !analytics_consent(&s, uid, Some(anon)).await? {
        return Ok(Reply::json(json!({ "stored": 0 })));
    }
    let rows: Vec<&Value> = b["events"].as_array().into_iter().flatten().filter(|e| e["name"].as_str().is_some_and(|n| CLIENT_EVENTS.contains(&n))).collect();
    for e in &rows {
        let props = crate::http::jsjson::stringify(e.get("props").unwrap_or(&json!({})));
        if crate::js::len(&props) > 2000 {
            continue;
        }
        let path = e.get("path").and_then(Value::as_str).map(|p| PATH_TAIL.replace(p, "").into_owned());
        sqlx::query("INSERT INTO analytics_events(user_id, anon_id, name, props, path) VALUES ($1,$2,$3,$4::jsonb,$5)")
            .bind(uid)
            .bind(anon)
            .bind(e["name"].as_str())
            .bind(props)
            .bind(path)
            .execute(&s.pool)
            .await?;
    }
    Ok(Reply::json(json!({ "stored": rows.len() })))
}

// ---------------------------------------------------------------- support

const MESSAGES: &str = "(SELECT json_agg(json_build_object('fromStaff', m.from_staff, 'body', m.body, 'at', m.created_at) ORDER BY m.id) FROM support_messages m WHERE m.ticket_id = t.id) AS messages";

async fn support_new(State(s): State<AppState>, req: Req) -> ApiResult {
    let b = z::obj([
        ("topic", z::enumeration(&TOPICS)),
        ("subject", z::str().min(3.0).max(200.0)),
        ("body", z::str().min(10.0).max(10000.0)),
        ("email", z::str().email().optional()),
    ])
    .parse(req.body())?;
    let u = auth::current_user(&s, &req).await?;
    let Some(email) = u.as_ref().map(|u| u.email.clone()).or_else(|| b.get("email").and_then(Value::as_str).map(str::to_owned)) else {
        return Ok(reply::json(400, json!({ "error": "email_required", "message": "Add an email so we can reply." })));
    };
    let uid = u.as_ref().map(|u| u.id);
    let (topic, subject, body) = (b["topic"].as_str().unwrap_or(""), b["subject"].as_str().unwrap_or(""), b["body"].as_str().unwrap_or(""));
    let id = oc_db::uuid();
    sqlx::query("INSERT INTO support_tickets(id, user_id, email, topic, subject) VALUES ($1,$2,$3,$4,$5)").bind(id).bind(uid).bind(&email).bind(topic).bind(subject).execute(&s.pool).await?;
    sqlx::query("INSERT INTO support_messages(ticket_id, from_staff, author_id, body) VALUES ($1,false,$2,$3)").bind(id).bind(uid).bind(body).execute(&s.pool).await?;
    let id = id.to_string();
    let r = ticket_ref(&id);
    let brand = s.cfg.brand.name;
    decision(&s, &email, format!("We got your message [#{r}]"), format!("Thanks for contacting {brand} support. We usually reply within one business day.\n\nYour message:\n{body}")).await?;
    if let Some(to) = s.env_nonempty("OC_SUPPORT_EMAIL") {
        decision(&s, &to, format!("[#{r}] {topic}: {subject}"), format!("From {email}\n\n{body}")).await?;
    }
    Ok(Reply::json(json!({ "id": id, "ref": r })))
}

async fn support_list(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let tickets = pg::json_rows(
        &s.pool,
        &format!("SELECT t.id, t.topic, t.subject, t.status, t.created_at, t.updated_at,\n         {MESSAGES}\n       FROM support_tickets t WHERE t.user_id = $1 ORDER BY t.updated_at DESC LIMIT 50"),
        &[pg::uuid(u.id)],
    )
    .await?;
    Ok(Reply::json(json!({ "tickets": tickets })))
}

async fn support_reply(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("body", z::str().min(1.0).max(10000.0))]).parse(req.body())?;
    if pg::one(&s.pool, "SELECT id FROM support_tickets WHERE id = $1::uuid AND user_id = $2", &[pg::text(&id), pg::uuid(u.id)]).await?.is_none() {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    pg::exec(&s.pool, "INSERT INTO support_messages(ticket_id, from_staff, author_id, body) VALUES ($1::uuid,false,$2,$3)", &[pg::text(&id), pg::uuid(u.id), pg::text(b["body"].as_str().unwrap_or(""))]).await?;
    pg::exec(&s.pool, "UPDATE support_tickets SET status = 'open', updated_at = now() WHERE id = $1::uuid", &[pg::text(&id)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- status (public)

async fn status(State(s): State<AppState>, _req: Req) -> ApiResult {
    let beats: Vec<(String, Option<i32>)> = sqlx::query_as("SELECT service, extract(epoch FROM now() - at)::int AS age FROM service_heartbeats").fetch_all(&s.pool).await?;
    let db_ok = sqlx::query("SELECT 1").execute(&s.pool).await.is_ok();
    let incidents = pg::json_rows(
        &s.pool,
        "SELECT i.*, (SELECT json_agg(json_build_object('status', u.status, 'body', u.body, 'at', u.created_at) ORDER BY u.id DESC) FROM status_updates u WHERE u.incident_id = i.id) AS updates
       FROM status_incidents i WHERE i.resolved_at IS NULL OR i.resolved_at > now() - interval '14 days' ORDER BY i.created_at DESC",
        &[],
    )
    .await?;
    let open: Vec<&Value> = incidents.iter().filter(|i| i["resolved_at"].is_null() && i["status"].as_str() != Some("scheduled")).collect();
    let components: Vec<Value> = COMPONENTS
        .iter()
        .map(|(id, name)| {
            let mut state = "operational";
            if *id == "database" {
                state = if db_ok { "operational" } else { "outage" };
            } else if *id != "web" && *id != "api" {
                state = match beats.iter().find(|(svc, _)| svc == id) {
                    None => "outage",
                    // `b.age > 180` with a NULL age is false (null compares as 0).
                    Some((_, age)) => {
                        let a = age.unwrap_or(0);
                        if a > 180 {
                            "outage"
                        } else if a > 90 {
                            "degraded"
                        } else {
                            "operational"
                        }
                    }
                };
            }
            let inc = open.iter().find(|i| i["components"].as_array().is_some_and(|c| c.iter().any(|x| x.as_str() == Some(id))));
            if let Some(inc) = inc {
                let impact = inc["impact"].as_str().unwrap_or("");
                state = if impact == "maintenance" {
                    "maintenance"
                } else if impact == "minor" && state == "operational" {
                    "degraded"
                } else if impact == "minor" {
                    state
                } else {
                    "outage"
                };
            }
            json!({ "id": id, "name": name, "state": state })
        })
        .collect();
    let worst = if components.iter().any(|c| c["state"] == "outage") {
        "outage"
    } else if components.iter().any(|c| c["state"] != "operational") {
        "degraded"
    } else {
        "operational"
    };
    Ok(Reply::json(json!({ "overall": worst, "components": components, "incidents": incidents, "checkedAt": oc_js::iso_string(&s.now()) })).header("cache-control", "public, max-age=15"))
}

// ---------------------------------------------------------------- staff

async fn admin_analytics(State(s): State<AppState>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    let days = req.query.get("days").cloned().unwrap_or(json!("30"));
    let n = z::js_number(Some(&days));
    let n = if n == 0.0 || n.is_nan() { 30.0 } else { n };
    let d = n.clamp(1.0, 365.0);
    // `$1::int` of a JS number: node-postgres sends its decimal text.
    let d = pg::text(oc_js::number_to_string(d));
    let daily = pg::json_rows(
        &s.pool,
        "WITH days AS (SELECT generate_series(current_date - ($1::int - 1), current_date, interval '1 day')::date AS day)
         SELECT to_char(days.day, 'YYYY-MM-DD') AS day,
           (SELECT count(*)::int FROM users u WHERE u.created_at::date = days.day AND u.deleted_at IS NULL) AS signups,
           (SELECT count(DISTINCT s.user_id)::int FROM sessions s WHERE s.last_seen_at::date = days.day) AS active,
           (SELECT count(*)::int FROM designs x WHERE x.created_at::date = days.day) AS designs,
           (SELECT count(*)::int FROM jobs j WHERE j.queue = 'export' AND j.created_at::date = days.day) AS exports
         FROM days ORDER BY days.day",
        std::slice::from_ref(&d),
    )
    .await?;
    let events = pg::json_rows(
        &s.pool,
        "SELECT name, count(*)::int AS n FROM analytics_events WHERE created_at > now() - make_interval(days => $1::int) GROUP BY name ORDER BY n DESC LIMIT 20",
        std::slice::from_ref(&d),
    )
    .await?;
    let plans = pg::json_rows(&s.pool, "SELECT plan, status, count(*)::int AS n, sum(seats)::int AS seats FROM subscriptions WHERE status IN ('active','trialing','past_due') GROUP BY plan, status", &[]).await?;
    let funnel = pg::json_one(
        &s.pool,
        "SELECT
           (SELECT count(*)::int FROM users WHERE created_at > now() - make_interval(days => $1::int)) AS signed_up,
           (SELECT count(DISTINCT owner_id)::int FROM designs WHERE created_at > now() - make_interval(days => $1::int)) AS created_design,
           (SELECT count(DISTINCT owner_id)::int FROM jobs WHERE queue = 'export' AND created_at > now() - make_interval(days => $1::int)) AS exported,
           (SELECT count(*)::int FROM subscriptions WHERE plan <> 'free' AND updated_at > now() - make_interval(days => $1::int)) AS upgraded",
        &[d],
    )
    .await?;
    Ok(Reply::json(json!({ "daily": daily, "events": events, "plans": plans, "funnel": funnel })))
}

async fn admin_support(State(s): State<AppState>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    let status = match req.query.get("status") {
        None => json!("open"),
        Some(v) => v.clone(),
    };
    let arg = match &status {
        Value::String(v) => pg::text(v),
        // node-postgres sends an array parameter as a Postgres array literal.
        Value::Array(a) => pg::text(format!("{{{}}}", a.iter().map(|x| format!("\"{}\"", x.as_str().unwrap_or("").replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(","))),
        other => pg::text(crate::http::jsjson::stringify(other)),
    };
    let tickets = pg::json_rows(&s.pool, &format!("SELECT t.*, {MESSAGES}\n           FROM support_tickets t WHERE t.status = $1 ORDER BY t.updated_at LIMIT 200"), &[arg]).await?;
    Ok(Reply::json(json!({ "tickets": tickets })))
}

async fn admin_support_reply(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = auth::require_staff(&s, &req).await?;
    let b = z::obj([("body", z::str().min(1.0).max(10000.0)), ("close", z::bool().optional())]).parse(req.body())?;
    let t: Option<(String, String)> = sqlx::query_as("SELECT email, subject FROM support_tickets WHERE id = $1::uuid").bind(&id).fetch_optional(&s.pool).await?;
    let Some((email, subject)) = t else { return Ok(reply::json(404, json!({ "error": "not_found" }))) };
    let body = b["body"].as_str().unwrap_or("");
    pg::exec(&s.pool, "INSERT INTO support_messages(ticket_id, from_staff, author_id, body) VALUES ($1::uuid,true,$2,$3)", &[pg::text(&id), pg::uuid(u.id), pg::text(body)]).await?;
    let st = if b.get("close").and_then(Value::as_bool) == Some(true) { "closed" } else { "answered" };
    pg::exec(&s.pool, "UPDATE support_tickets SET status = $2, updated_at = now() WHERE id = $1::uuid", &[pg::text(&id), pg::text(st)]).await?;
    decision(&s, &email, format!("Re: {subject} [#{}]", ticket_ref(&id)), format!("{body}\n\nReply from Help → Your requests in {}.", s.cfg.brand.name)).await?;
    Ok(Reply::json(json!({ "ok": true })))
}

async fn incident_new(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = auth::require_staff(&s, &req).await?;
    let ids: Vec<&str> = COMPONENTS.iter().map(|c| c.0).collect();
    let b = z::obj([
        ("title", z::str().min(3.0).max(200.0)),
        ("impact", z::enumeration(&["minor", "major", "critical", "maintenance"])),
        ("components", z::array(z::enumeration(&ids)).min(1.0)),
        ("status", z::enumeration(&["investigating", "identified", "monitoring", "scheduled"])),
        ("body", z::str().min(3.0).max(5000.0)),
    ])
    .parse(req.body())?;
    let id = oc_db::uuid();
    let str_of = |k: &str| b[k].as_str().unwrap_or("").to_owned();
    let components: Vec<String> = b["components"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    sqlx::query("INSERT INTO status_incidents(id, title, impact, components, status) VALUES ($1,$2,$3,$4,$5)")
        .bind(id)
        .bind(str_of("title"))
        .bind(str_of("impact"))
        .bind(&components)
        .bind(str_of("status"))
        .execute(&s.pool)
        .await?;
    sqlx::query("INSERT INTO status_updates(incident_id, status, body) VALUES ($1,$2,$3)").bind(id).bind(str_of("status")).bind(str_of("body")).execute(&s.pool).await?;
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent {
            actor_id: Some(u.id),
            action: "status.incident_opened".into(),
            target: Some(id.to_string()),
            meta: Some(json!({ "title": str_of("title") })),
            ip: Some(req.ip.clone()),
            ..Default::default()
        },
        s.now(),
    )
    .await?;
    Ok(Reply::json(json!({ "id": id.to_string() })))
}

async fn incident_update(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    auth::require_staff(&s, &req).await?;
    let b = z::obj([("status", z::enumeration(&["investigating", "identified", "monitoring", "resolved"])), ("body", z::str().min(3.0).max(5000.0))]).parse(req.body())?;
    let st = b["status"].as_str().unwrap_or("");
    pg::exec(&s.pool, "INSERT INTO status_updates(incident_id, status, body) VALUES ($1::uuid,$2,$3)", &[pg::text(&id), pg::text(st), pg::text(b["body"].as_str().unwrap_or(""))]).await?;
    pg::exec(&s.pool, "UPDATE status_incidents SET status = $2, resolved_at = CASE WHEN $2 = 'resolved' THEN now() ELSE NULL END WHERE id = $1::uuid", &[pg::text(&id), pg::text(st)]).await?;
    Ok(Reply::json(json!({ "ok": true })))
}
