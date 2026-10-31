// SPDX-License-Identifier: Apache-2.0
//! Crate errors.

/// Errors from decoding updates or converting snapshots.
#[derive(Debug, thiserror::Error)]
pub enum CrdtError {
    /// The bytes are not a valid lib0 v1 update / state vector.
    #[error("invalid update: {0}")]
    Decode(String),
    /// The snapshot does not have the shape of a design.
    #[error("invalid snapshot: {0}")]
    Json(#[from] serde_json::Error),
    /// A schema migration failed (same message as the TS `migrate`).
    #[error(transparent)]
    Migrate(#[from] oc_schema::migrate::MigrateError),
}
