//! Background worker — the Rust port of `services/worker` (ADR-0001). It drains the Postgres job queues that
//! are not rendering work and runs maintenance, with the same queues, concurrency, schedules, SQL and
//! outputs as the TypeScript worker:
//!
//! | queue / schedule   | handler                                   | concurrency / period |
//! |--------------------|-------------------------------------------|----------------------|
//! | `webhook`          | [`jobs::webhook`] (`deliverWebhook`)      | 4                    |
//! | `email`            | [`jobs::email`] (`deliver`)               | 4                    |
//! | `account-export`   | [`jobs::account_export`] (GDPR Art. 15/20)| 1                    |
//! | `account-delete`   | [`jobs::account_delete`] (GDPR Art. 17)   | 1                    |
//! | `publish`          | [`jobs::publish`] (`runPublish`)          | 2                    |
//! | `maintenance`      | [`jobs::maintenance`]                     | every hour + at start|
//! | `social-sweep`     | [`jobs::social_sweep`]                    | every minute         |
//! | `object-gc`        | [`gc::collect_garbage`]                   | every day            |
//!
//! Social publishing itself lives in the shared `oc-social` crate (re-exported as [`social`]).

pub mod gc;
pub mod jobs;
pub mod pgjson;

pub use oc_social as social;

/// One day.
pub const DAY: std::time::Duration = std::time::Duration::from_secs(86_400);
