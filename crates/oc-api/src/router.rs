//! The one place where route modules are assembled — the `app.register(...)` list of services/api/src/main.ts.
//! A new route module only needs its own file; it is already listed here (as a stub until ported).

use axum::Router;
use axum::middleware::from_fn_with_state;

use crate::http::ratelimit::{RateLimits, RouteLimit};
use crate::routes as r;
use crate::state::AppState;

/// Every module's routes, in main.ts registration order.
fn modules() -> Vec<(Router<AppState>, &'static [RouteLimit])> {
    vec![
        (r::health::routes(), r::health::LIMITS),
        (r::account::routes(), r::account::LIMITS),
        (r::supabase::routes(), r::supabase::LIMITS),
        (r::misc::routes(), r::misc::LIMITS),
        (r::assets::routes(), r::assets::LIMITS),
        (r::designs::routes(), r::designs::LIMITS),
        (r::brand::routes(), r::brand::LIMITS),
        (r::brandhub::routes(), r::brandhub::LIMITS),
        (r::media::routes(), r::media::LIMITS),
        (r::teams::routes(), r::teams::LIMITS),
        (r::safety::routes(), r::safety::LIMITS),
        (r::dev::routes(), r::dev::LIMITS),
        (r::billing::routes(), r::billing::LIMITS),
        (r::growth::routes(), r::growth::LIMITS),
        (r::content::routes(), r::content::LIMITS),
        (r::graphics::routes(), r::graphics::LIMITS),
        (r::imports::routes(), r::imports::LIMITS),
        (r::forms::routes(), r::forms::LIMITS),
        (r::docs::routes(), r::docs::LIMITS),
        (r::sheets::routes(), r::sheets::LIMITS),
        (r::sites::routes(), r::sites::LIMITS),
        (r::v1::routes(), r::v1::LIMITS),
        (r::oauth::routes(), r::oauth::LIMITS),
        (crate::mcp::routes(), crate::mcp::LIMITS),
        (r::social::routes(), r::social::LIMITS),
        (r::embeds::routes(), r::embeds::LIMITS),
        (r::print::routes(), r::print::LIMITS),
        (r::print_profiles::routes(), r::print_profiles::LIMITS),
        (r::search::routes(), r::search::LIMITS),
        (r::sharing::routes(), r::sharing::LIMITS),
        (r::openapi::routes(), r::openapi::LIMITS),
    ]
}

/// Every route's own rate limit (`config.rateLimit`), collected from the route modules.
pub fn route_limits() -> Vec<RouteLimit> {
    let mut v = Vec::new();
    for (_, limits) in modules() {
        v.extend_from_slice(limits);
    }
    v
}

pub fn rate_limits() -> RateLimits {
    RateLimits::new(&route_limits())
}

/// The whole API as an axum router.
pub fn app(state: AppState) -> Router {
    let mut router: Router<AppState> = Router::new();
    for (m, _) in modules() {
        router = router.merge(m);
    }
    let inner = router.fallback(crate::http::not_found).method_not_allowed_fallback(crate::http::not_found).with_state(state.clone());
    // `Router::layer` wraps each route; the global middleware must see the finished response of the whole router
    // (including what axum adds after a route's own layers), so the router is the fallback of an outer one.
    Router::new().fallback_service(inner).layer(from_fn_with_state(state, crate::http::global))
}
