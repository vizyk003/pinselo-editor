// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * DocOp: the single write contract (ADR-0041). Humans, the AI agent, templates and imports all call
 * `applyOps`, which validates, checks locks and writes inside ONE Yjs transaction (all-or-nothing).
 * Text edits use flat offsets (paragraphs joined by '\n') instead of ProseMirror steps in this build.
 */
import { type AssetRef, type DesignMeta, type Element, type ElementId, type Page, type PageId, type ParagraphAttrs, type TextBody, type Xf } from '../schema';
import * as Y from 'yjs';
export type After = ElementId | 'top' | 'bottom';
type Marks = Record<string, unknown>;
export type DocOp = {
    op: 'add';
    el: Element;
    parent: PageId | ElementId;
    after?: After;
} | {
    op: 'update';
    id: ElementId;
    patch: Record<string, unknown>;
} | {
    op: 'xf';
    id: ElementId;
    xf: Xf;
} | {
    op: 'reparent';
    id: ElementId;
    parent: PageId | ElementId;
    after?: After;
    xf?: Xf;
} | {
    op: 'reorder';
    id: ElementId;
    after: After;
} | {
    op: 'delete';
    id: ElementId;
} | {
    op: 'setText';
    id: ElementId;
    body: TextBody | string;
} | {
    op: 'textInsert';
    id: ElementId;
    offset: number;
    text: string;
    marks?: Marks;
} | {
    op: 'textDelete';
    id: ElementId;
    offset: number;
    len: number;
} | {
    op: 'textFormat';
    id: ElementId;
    marks: Marks;
    offset?: number;
    len?: number;
} | {
    op: 'textParagraph';
    id: ElementId;
    attrs: Partial<ParagraphAttrs>;
    offset?: number;
    len?: number;
} | {
    op: 'page';
    action: 'add';
    page?: Partial<Page>;
    after?: PageId | null;
} | {
    op: 'page';
    action: 'delete';
    pageId: PageId;
} | {
    op: 'page';
    action: 'duplicate';
    pageId: PageId;
    newPageId?: PageId;
} | {
    op: 'page';
    action: 'move';
    pageId: PageId;
    after: PageId | null;
} | {
    op: 'page';
    action: 'update';
    pageId: PageId;
    patch: Partial<Page>;
} | {
    op: 'meta';
    patch: Partial<DesignMeta>;
} | {
    op: 'asset';
    asset: AssetRef;
};
export type DocOpErrorCode = 'SCHEMA' | 'NOT_FOUND' | 'LOCKED' | 'CYCLE' | 'INVALID';
export interface DocOpResult {
    ok: boolean;
    applied: number;
    created: string[];
    errors: Array<{
        index: number;
        code: DocOpErrorCode;
        message: string;
    }>;
}
export interface ApplyOptions {
    actor?: string;
    now?: () => number;
    partial?: boolean;
}
export declare function applyOps(ydoc: Y.Doc, ops: DocOp[], origin: unknown, opts?: ApplyOptions): DocOpResult;
