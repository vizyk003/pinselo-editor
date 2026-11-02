//! oc-api: the Rust port of services/api (ADR-0001 strangler migration). Same port, routes, cookies, status
//! codes and JSON shapes as the Fastify app, so the TS web app works unchanged. ROUTES.md lists the port status
//! of every TS route file.
//!
//! Layout (one Rust module per TS file, so route groups can be ported in parallel without conflicts):
//!
//! * `routes/<file>.rs` ← `services/api/src/routes/<file>.ts`: `pub fn routes() -> Router<AppState>` plus
//!   `pub const LIMITS` (its `config.rateLimit` routes) and the helpers other modules import from it;
//! * `<file>.rs` ← the non-route TS modules (`auth.ts`, `brandhub.ts`, `media.ts`, …);
//! * [`router`]: the one place that assembles every module (main.ts's `app.register(...)` list);
//! * [`http`]: the Fastify behaviour (body parsing, cookies, rate limits, error replies, security headers);
//! * [`zod`]: request validation with zod's exact issue format; [`pg`]: node-postgres-shaped rows.

pub mod analytics;
pub mod auth;
pub mod bearer;
pub mod brandhub;
pub mod doc_access;
pub mod graphics;
pub mod hash_match;
pub mod http;
pub mod importers;
pub mod js;
pub mod jwt;
pub mod login;
pub mod mcp;
pub mod media;
pub mod oauth;
pub mod pg;
pub mod projects;
pub mod router;
pub mod routes;
pub mod search;
pub mod share_access;
pub mod share_links;
pub mod signup;
pub mod site_seo;
pub mod state;
pub mod stock;
pub mod supabase;
pub mod uploads;
pub mod webauthn;
pub mod zod;

pub use router::app;
pub use state::{AppOptions, AppState};

/// Imports shared by the route modules.
pub mod prelude {
    pub use axum::Router;
    pub use axum::extract::{Path, State};
    pub use axum::routing::{delete, get, patch, post, put};
    pub use serde_json::{Value, json};
    pub use uuid::Uuid;

    pub use crate::auth::{self, User, require_user};
    pub use crate::http::error::{ApiError, ApiResult};
    pub use crate::http::ratelimit::{RouteLimit, strict};
    pub use crate::http::reply::{self, Reply};
    pub use crate::http::req::Req;
    pub use crate::pg::{self, Arg};
    pub use crate::state::AppState;
    pub use crate::zod::{self as z};
}
