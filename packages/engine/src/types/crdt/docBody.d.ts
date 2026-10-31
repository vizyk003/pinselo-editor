// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * Docs body (DC-001): the Tiptap/ProseMirror tree lives in the Yjs XmlFragment "docBody" (y-prosemirror
 * encoding: one XmlElement per node, marks as XmlText formatting attributes). Shared by the render
 * service (layout, DOCX/Markdown export) and the API (embedded-design access checks).
 */
import * as Y from 'yjs';
export interface PMMark {
    type: string;
    attrs?: Record<string, unknown>;
}
export interface PMNode {
    type: string;
    attrs?: Record<string, unknown>;
    content?: PMNode[];
    text?: string;
    marks?: PMMark[];
}
export declare const DOC_FIELD = "docBody";
/** Y.XmlFragment (y-prosemirror encoding) → ProseMirror-style JSON. */
export declare function fragmentToJson(frag: Y.XmlFragment): PMNode;
/** ProseMirror-style JSON → children appended to a fragment (tests, server-side imports). */
export declare function jsonToFragment(frag: Y.XmlFragment, root: PMNode): void;
/** Walk every node (depth-first). */
export declare function walkDoc(root: PMNode, fn: (n: PMNode) => void): void;
/** Design ids embedded in a doc body ("designEmbed" blocks). */
export declare function docEmbeddedDesignIds(root: PMNode): string[];
/** Embedded design ids straight from a persisted Yjs state (no full design decode). */
export declare function embeddedDesignIdsOfState(state: Uint8Array): string[];
/** Plain text of a node (headings for a table of contents, alt text…). */
export declare function nodeText(n: PMNode): string;
