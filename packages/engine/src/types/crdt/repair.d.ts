// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
import * as Y from 'yjs';
export declare const REPAIR_ORIGIN: unique symbol;
export interface RepairReport {
    orphans: number;
    cycles: number;
    transforms: number;
    migratedFrom?: number;
}
export declare function repairY(ydoc: Y.Doc): RepairReport;
/** Run registered schema migrators on an older document (ADR-0011), then repair. */
export declare function upgradeY(ydoc: Y.Doc): RepairReport;
