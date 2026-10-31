// SPDX-License-Identifier: Apache-2.0
//! Schema migrations (ADR-0011). Additive changes need no migrator. A breaking change registers
//! `{ from: n, to: n + 1, up }`; `migrate()` runs the chain on a JSON snapshot.
//!
//! Unlike the TS module-level registry, the Rust registry is an explicit value (no global mutable state);
//! `migrate` uses the built-in registry, which is empty at schema 1 — exactly like the TS build.

use thiserror::Error;

use crate::design::{CURRENT_SCHEMA, DesignDoc};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MigrateError {
    #[error("migrators must step exactly one version")]
    BadStep,
    #[error("duplicate migrator from v{0}")]
    Duplicate(i64),
    #[error("cannot downgrade schema v{0} to v{1}")]
    Downgrade(i64, i64),
    #[error("no migrator from schema v{0}")]
    Missing(i64),
}

pub struct Migrator {
    pub from: i64,
    pub to: i64,
    pub description: &'static str,
    pub up: fn(DesignDoc) -> DesignDoc,
}

#[derive(Default)]
pub struct MigratorRegistry {
    migrators: Vec<Migrator>,
}

impl MigratorRegistry {
    /// The migrators shipped with this build (none at schema 1).
    pub fn builtin() -> Self {
        MigratorRegistry::default()
    }

    pub fn register(&mut self, m: Migrator) -> Result<(), MigrateError> {
        if m.to != m.from + 1 {
            return Err(MigrateError::BadStep);
        }
        if self.migrators.iter().any(|r| r.from == m.from) {
            return Err(MigrateError::Duplicate(m.from));
        }
        self.migrators.push(m);
        self.migrators.sort_by_key(|r| r.from);
        Ok(())
    }

    pub fn migrate(&self, doc: DesignDoc, target: i64) -> Result<DesignDoc, MigrateError> {
        let mut cur = doc;
        let mut v = cur.meta.schema_version;
        if v > target {
            return Err(MigrateError::Downgrade(v, target));
        }
        while v < target {
            let m = self.migrators.iter().find(|r| r.from == v).ok_or(MigrateError::Missing(v))?;
            cur = (m.up)(cur);
            cur.meta.schema_version = m.to;
            v = m.to;
        }
        Ok(cur)
    }
}

/// Migrate a snapshot to `CURRENT_SCHEMA` with the built-in migrators.
pub fn migrate(doc: DesignDoc) -> Result<DesignDoc, MigrateError> {
    MigratorRegistry::builtin().migrate(doc, CURRENT_SCHEMA)
}

/// Migrate to an explicit target schema with the built-in migrators.
pub fn migrate_to(doc: DesignDoc, target: i64) -> Result<DesignDoc, MigrateError> {
    MigratorRegistry::builtin().migrate(doc, target)
}

pub fn needs_migration(doc: &DesignDoc) -> bool {
    doc.meta.schema_version < CURRENT_SCHEMA
}
