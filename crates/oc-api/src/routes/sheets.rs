//! Port of services/api/src/routes/sheets.ts — sheets (A15): read a computed range of a sheet design, the data
//! behind linked charts. The caller needs view access to the SOURCE sheet; values are computed server-side with
//! the same engine as the editor (oc-sheet, behind the default `sheet` feature; without it the route answers
//! 501 like sheet creation in designs.rs).

use oc_auth::Role;

use crate::auth::{at_least, design_role};
use crate::prelude::*;

pub const LIMITS: &[RouteLimit] = &[];

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/designs/{id}/sheet/values", get(values))
}

async fn values(State(s): State<AppState>, Path(id): Path<String>, req: Req) -> ApiResult {
    let u = require_user(&s, &req).await?;
    let q = z::obj([("sheet", z::str().max(200.0).default(json!(""))), ("range", z::str().min(2.0).max(200.0))]).parse(Some(&req.query))?;
    if !at_least(design_role(&s, Some(u.id), &id).await?, Role::View) {
        return Ok(reply::json(404, json!({ "error": "not_found" })));
    }
    let row: Option<(Vec<u8>, String, String)> =
        sqlx::query_as("SELECT s.state, d.surface, d.title FROM doc_state s JOIN designs d ON d.id = s.design_id WHERE s.design_id = $1").bind(&id).fetch_optional(&s.pool).await?;
    let Some((state, _, title)) = row.filter(|r| r.1 == "sheet") else {
        return Ok(reply::json(404, json!({ "error": "not_a_sheet" })));
    };
    read(&s, &state, &title, q["sheet"].as_str().unwrap_or(""), q["range"].as_str().unwrap_or(""))
}

#[cfg(feature = "sheet")]
fn read(s: &AppState, state: &[u8], title: &str, sheet: &str, range: &str) -> ApiResult {
    // `Date.now()` / `Math.random()` of the TS engine: the app clock and a fixed seed.
    let host = oc_sheet::formula::workbook::FixedHost::new(s.now_ms() as f64, 1);
    let engine = oc_sheet::linked::open_sheet_state(state, Box::new(host));
    let Some(got) = oc_sheet::linked::read_range(&engine, sheet, range) else {
        return Ok(reply::json(400, json!({ "error": "bad_range", "message": "Range or tab not found." })));
    };
    Ok(Reply::json(json!({ "title": title, "sheet": got.tab_id, "sheetName": got.tab_name, "values": serde_json::to_value(&got.values)? })))
}

#[cfg(not(feature = "sheet"))]
fn read(_s: &AppState, _state: &[u8], _title: &str, _sheet: &str, _range: &str) -> ApiResult {
    Ok(reply::json(501, json!({ "error": "capability_unavailable", "message": "Sheets need the oc-api `sheet` feature." })))
}
