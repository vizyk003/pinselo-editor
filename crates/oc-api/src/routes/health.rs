//! The routes services/api/src/main.ts registers inline: `GET /healthz` and `GET /api/config`.

use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new().route("/healthz", get(healthz)).route("/api/config", get(config))
}

async fn healthz(_req: Req) -> ApiResult {
    Ok(Reply::json(json!({ "ok": true })))
}

async fn config(State(s): State<AppState>, _req: Req) -> ApiResult {
    Ok(Reply::json(json!({
        "assetOrigin": s.cfg.usercontent_origin,
        "brand": s.cfg.brand.name,
        "env": if s.prod() { "production" } else { "development" },
        "plans": crate::billing::plans_enabled(&s),
    })))
}
