// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * DocStore: a Y.Doc plus an incrementally maintained immutable JSON snapshot.
 * Only elements touched by a transaction are re-read, so a keystroke in one text box costs O(1) elements.
 */
import type { DesignDoc, ElementId, PageId } from '../schema';
import * as Y from 'yjs';
import { type Roots } from './codec';
import { type ApplyOptions, type DocOp, type DocOpResult } from './ops';
export interface Change {
    elements: Set<ElementId>;
    pages: Set<PageId>;
    meta: boolean;
    assets: boolean;
    local: boolean;
    origin: unknown;
}
export type Listener = (doc: DesignDoc, change: Change) => void;
export declare class DocStore {
    readonly ydoc: Y.Doc;
    readonly roots: Roots;
    readonly undo: Y.UndoManager;
    private snap;
    private pending;
    private readonly listeners;
    actor: string;
    constructor(ydoc?: Y.Doc);
    private change;
    private rebuild;
    get doc(): DesignDoc;
    apply(ops: DocOp[], origin?: unknown, opts?: ApplyOptions): DocOpResult;
    subscribe(fn: Listener): () => void;
    /** Start a fresh undo step (call at the start of a gesture so it is not merged with the previous one). */
    checkpoint(): void;
    /** Re-read everything (after the provider's initial sync replaces the doc contents). */
    reload(): void;
    destroy(): void;
}
