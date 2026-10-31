// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * JSON snapshot ⇄ Y.Doc. Root layout (docs/plan/03 §4.1, storage 'single'):
 *   meta: Y.Map (scalars)   pages: Y.Map<PageId, Y.Map>   elements: Y.Map<ElementId, Y.Map>   assets: Y.Map<AssetId, plain>
 * Element map: base fields as plain (atomic) values, type-specific fields in `props: Y.Map`,
 * rich text in `text: Y.XmlFragment`.
 */
import { type AssetRef, type DesignDoc, type DesignMeta, type Element, type Page } from '../schema';
import * as Y from 'yjs';
export interface Roots {
    meta: Y.Map<unknown>;
    pages: Y.Map<Y.Map<unknown>>;
    elements: Y.Map<Y.Map<unknown>>;
    assets: Y.Map<AssetRef>;
}
export declare function roots(ydoc: Y.Doc): Roots;
export declare function writeElement(elements: Y.Map<Y.Map<unknown>>, el: Element): Y.Map<unknown>;
export declare function readElement(map: Y.Map<unknown>): Element;
export declare function writePage(pages: Y.Map<Y.Map<unknown>>, page: Page): Y.Map<unknown>;
export declare function readPage(map: Y.Map<unknown>): Page;
export declare function readMeta(meta: Y.Map<unknown>): DesignMeta;
/** Populate an empty Y.Doc from a snapshot (one transaction). */
export declare function toY(doc: DesignDoc, ydoc?: Y.Doc, origin?: unknown): Y.Doc;
export declare function fromY(ydoc: Y.Doc): DesignDoc;
export declare function encodeDoc(doc: DesignDoc): Uint8Array;
export declare function decodeDoc(update: Uint8Array): DesignDoc;
/**
 * Replace the live document's content with a snapshot (version restore) as ONE transaction.
 * Elements missing from the snapshot are tombstoned (so undo can bring them back); the rest are rewritten.
 */
export declare function replaceContent(ydoc: Y.Doc, doc: DesignDoc, origin: unknown, actor?: string): void;
