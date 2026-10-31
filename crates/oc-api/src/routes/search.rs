//! Port of services/api/src/routes/search.ts — unified search (g16): grouped results across designs, folders,
//! uploads and templates, plus autocomplete. The SQL builders live in [`crate::search`].

use crate::prelude::*;
use crate::search::{self as sq, Built, Suggestion};

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/search", get(search)).route("/api/search/suggest", get(suggest))
}

async fn run(s: &AppState, on: bool, b: impl FnOnce() -> Built) -> Result<Vec<Value>, ApiError> {
    if !on {
        return Ok(vec![]);
    }
    let Built { sql, params } = b();
    Ok(pg::json_rows(&s.pool, &sql, &params).await?)
}

/// `GET /api/search?q=&type=all|designs|folders|uploads|templates&owner=anyone|me|shared&modified=any|1d|7d|30d|365d&surface=&limit=`
async fn search(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let p = sq::parse_search_query(&req.query)?;
    let groups = sq::groups_for(&p.kind, &p.owner);
    let now = s.now();
    // Folders and uploads carry no surface, so a surface (category) filter narrows search to designs.
    let surface = p.surface.as_deref().is_some_and(|x| !x.is_empty());
    let (designs, folders, uploads, templates) = futures::try_join!(
        run(&s, groups.contains(&"designs"), || sq::designs_sql(u.id, &p, now)),
        run(&s, groups.contains(&"folders") && !surface, || sq::folders_sql(u.id, &p, now)),
        run(&s, groups.contains(&"uploads") && !surface, || sq::uploads_sql(u.id, &p, now)),
        run(&s, groups.contains(&"templates") && !surface, || sq::templates_sql(u.id, &p, now)),
    )?;
    Ok(Reply::json(json!({ "q": sq::normalize_query(&p.q), "designs": designs, "folders": folders, "uploads": uploads, "templates": templates })))
}

/// `GET /api/search/suggest?q=` — template titles, categories and tags for autocomplete.
async fn suggest(State(s): State<AppState>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let b = z::obj([("q", z::str().max(200.0).default(json!(""))), ("limit", z::coerce_num().int().min(1.0).max(20.0).default(json!(8)))]).parse(Some(&req.query))?;
    let q = b["q"].as_str().unwrap_or("").to_owned();
    let limit = b["limit"].as_f64().unwrap_or(8.0) as i64;
    if sq::normalize_query(&q).is_empty() {
        return Ok(Reply::json(json!({ "suggestions": [] })));
    }
    let Built { sql, params } = sq::suggest_sql(u.id, &q, limit);
    let rows = pg::json_rows(&s.pool, &sql, &params).await?;
    let rows: Vec<Suggestion> = rows
        .iter()
        .map(|r| Suggestion {
            kind: r["kind"].as_str().unwrap_or("").to_owned(),
            text: r["text"].as_str().unwrap_or("").to_owned(),
            id: r["id"].as_str().filter(|x| !x.is_empty()).map(str::to_owned),
        })
        .collect();
    let out: Vec<Value> = sq::rank_suggestions(&rows, &q, limit as usize).iter().map(Suggestion::to_json).collect();
    Ok(Reply::json(json!({ "suggestions": out })))
}
