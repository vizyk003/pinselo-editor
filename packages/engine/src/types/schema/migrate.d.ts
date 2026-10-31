// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/**
 * Schema migrations (ADR-0011). Additive changes need no migrator. A breaking change registers
 * `{ from: n, to: n + 1, up }`; `migrate()` runs the chain on a JSON snapshot. The collab server
 * runs the same chain on first load under a per-document lease.
 */
import { type DesignDoc } from './design';
export interface Migrator {
    from: number;
    to: number;
    description: string;
    up(doc: DesignDoc): DesignDoc;
}
export declare function registerMigrator(m: Migrator): void;
export declare function migrate(doc: DesignDoc, target?: number): DesignDoc;
export declare function needsMigration(doc: DesignDoc): boolean;
