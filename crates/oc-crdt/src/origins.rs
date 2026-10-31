// SPDX-License-Identifier: Apache-2.0
//! Transaction origins. In yjs these are singleton instances matched by identity (`LocalOrigin.I`, …); yrs
//! origins are byte strings, so each singleton maps to a fixed tag. Origins are local to a replica (they are
//! never encoded into updates), so the tags only need to be stable within the Rust code base.
//! docs/plan/03-document-model-and-renderer.md §4.3.

use yrs::Origin;

/// The transaction origins of `@oc/crdt` (`LocalOrigin.I`, `AgentOrigin.I`, …, plus the `REPAIR_ORIGIN` and
/// `BOARD_ORIGIN` symbols).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OcOrigin {
    Local,
    Agent,
    Template,
    Migration,
    Repair,
    Restore,
    Remote,
    /// `REPAIR_ORIGIN` (the load-time repair symbol; distinct from `RepairOrigin.I` like in TS).
    RepairSymbol,
    /// `BOARD_ORIGIN` (whiteboard timer / votes; never tracked by undo).
    Board,
}

impl OcOrigin {
    pub const ALL: [OcOrigin; 9] = [
        OcOrigin::Local,
        OcOrigin::Agent,
        OcOrigin::Template,
        OcOrigin::Migration,
        OcOrigin::Repair,
        OcOrigin::Restore,
        OcOrigin::Remote,
        OcOrigin::RepairSymbol,
        OcOrigin::Board,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            OcOrigin::Local => "oc:local",
            OcOrigin::Agent => "oc:agent",
            OcOrigin::Template => "oc:template",
            OcOrigin::Migration => "oc:migration",
            OcOrigin::Repair => "oc:repair",
            OcOrigin::Restore => "oc:restore",
            OcOrigin::Remote => "oc:remote",
            OcOrigin::RepairSymbol => "oc:repair-load",
            OcOrigin::Board => "oc:board",
        }
    }

    /// The yrs origin value for transactions.
    pub fn origin(self) -> Origin {
        Origin::from(self.tag())
    }

    /// Which Pinselo origin a yrs origin is, if any.
    pub fn from_origin(o: &Origin) -> Option<OcOrigin> {
        OcOrigin::ALL.into_iter().find(|x| x.tag().as_bytes() == o.as_ref())
    }
}

impl From<OcOrigin> for Origin {
    fn from(o: OcOrigin) -> Origin {
        o.origin()
    }
}

/// `REPAIR_ORIGIN`.
pub const REPAIR_ORIGIN: OcOrigin = OcOrigin::RepairSymbol;
/// `BOARD_ORIGIN`.
pub const BOARD_ORIGIN: OcOrigin = OcOrigin::Board;
