//! Facade over the platform crates (ports of the TS `packages/*` platform libraries, ADR-0001).
pub use oc_audit as audit;
pub use oc_auth as auth;
pub use oc_config as config;
pub use oc_db as db;
pub use oc_gfonts as gfonts;
pub use oc_jobs as jobs;
pub use oc_js as js;
pub use oc_mail as mail;
pub use oc_storage as storage;
pub use oc_webhooks as webhooks;
