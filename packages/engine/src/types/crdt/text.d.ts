// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * Rich text on Y.XmlFragment: <paragraph align lineHeight spaceAfter list> → Y.XmlText with run marks.
 * The shape matches the y-prosemirror "oc-text-v1" schema (ADR-0009), so a ProseMirror-backed editor
 * can bind to the same data later. Editing here uses flat offsets where paragraphs are joined by '\n'.
 */
import { type Paragraph, type ParagraphAttrs, type TextBody } from '../schema';
import * as Y from 'yjs';
type Attrs = Record<string, unknown>;
interface ParaRef {
    el: Y.XmlElement;
    text: Y.XmlText;
}
export declare function paragraphsOf(frag: Y.XmlFragment): ParaRef[];
/** Write a whole TextBody into an (empty or existing) fragment. */
export declare function writeBody(frag: Y.XmlFragment, body: TextBody): void;
export declare function readParagraphs(frag: Y.XmlFragment): Paragraph[];
export declare function plainText(frag: Y.XmlFragment): string;
/** Map a flat offset to (paragraph index, offset within paragraph). Clamps to the end. */
export declare function locate(paras: ParaRef[], offset: number): {
    p: number;
    o: number;
};
/** Marks in effect at an offset (the run before the caret), used to continue typing in the same style. */
export declare function marksAt(frag: Y.XmlFragment, offset: number): Attrs;
export declare function insertText(frag: Y.XmlFragment, offset: number, str: string, attrs?: Attrs): void;
/**
 * A flat text offset as a relative position (COL-02): anchored to the character before it (assoc -1), so text a
 * peer inserts in front of it, or at the same spot, does not move it into the middle of the local user's typing.
 */
export declare function stickyOffset(frag: Y.XmlFragment, offset: number): Y.RelativePosition | null;
/** The flat offset a {@link stickyOffset} points at now (`null` when its paragraph is gone). */
export declare function resolveSticky(frag: Y.XmlFragment, pos: Y.RelativePosition): number | null;
export declare function deleteText(frag: Y.XmlFragment, offset: number, len: number): void;
/** Apply marks to a flat range. A `null` value removes that mark. */
export declare function formatText(frag: Y.XmlFragment, offset: number, len: number, attrs: Attrs): void;
/** Apply marks to every run in the body (whole-element styling from the toolbar). */
export declare function formatAll(frag: Y.XmlFragment, attrs: Attrs): void;
/** Set paragraph attributes on every paragraph touched by the range (or all when range is omitted). */
export declare function setParagraphs(frag: Y.XmlFragment, attrs: Partial<ParagraphAttrs>, range?: {
    offset: number;
    len: number;
}): void;
