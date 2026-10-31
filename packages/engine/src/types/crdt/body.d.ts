// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * Text ops on a detached TextBody (table cells, A12). Runs the exact Y.XmlFragment code text elements use on a
 * scratch doc, so typing, deleting, formatting and paragraph attributes behave identically in a cell.
 */
import type { ParagraphAttrs, TextBody } from '../schema';
type Marks = Record<string, unknown>;
export type BodyOp = {
    op: 'textInsert';
    offset: number;
    text: string;
    marks?: Marks;
} | {
    op: 'textDelete';
    offset: number;
    len: number;
} | {
    op: 'textFormat';
    marks: Marks;
    offset?: number;
    len?: number;
} | {
    op: 'textParagraph';
    attrs: Partial<ParagraphAttrs>;
    offset?: number;
    len?: number;
};
export declare function editBody(body: TextBody, ops: BodyOp[]): TextBody;
