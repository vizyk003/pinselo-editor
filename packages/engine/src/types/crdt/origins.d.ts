// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * Transaction origins are singleton instances so Y.UndoManager.trackedOrigins can match them by identity.
 * docs/plan/03-document-model-and-renderer.md §4.3.
 */
export declare class LocalOrigin {
    static readonly I: LocalOrigin;
}
export declare class AgentOrigin {
    static readonly I: AgentOrigin;
}
export declare class TemplateOrigin {
    static readonly I: TemplateOrigin;
}
export declare class MigrationOrigin {
    static readonly I: MigrationOrigin;
}
export declare class RepairOrigin {
    static readonly I: RepairOrigin;
}
export declare class RestoreOrigin {
    static readonly I: RestoreOrigin;
}
export declare class RemoteOrigin {
    static readonly I: RemoteOrigin;
}
export type Origin = LocalOrigin | AgentOrigin | TemplateOrigin | MigrationOrigin | RepairOrigin | RestoreOrigin | RemoteOrigin;
