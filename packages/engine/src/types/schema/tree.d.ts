// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Read-side helpers over a DesignDoc snapshot. Normalizes invariants at read time (cycles, orphans, ties). */
import type { DesignDoc, Page } from './design';
import type { Element } from './elements';
import type { ElementId, PageId } from './primitives';
export declare function sortedPages(doc: Pick<DesignDoc, 'pages'>): Page[];
export declare function liveElements(doc: Pick<DesignDoc, 'elements'>): Element[];
/** Children of a parent (page or group), bottom-most first. */
export declare function childrenOf(doc: Pick<DesignDoc, 'elements'>, parentId: PageId | ElementId): Element[];
/** The page an element ultimately belongs to, or undefined for orphans / cycles. */
export declare function pageOf(doc: Pick<DesignDoc, 'elements' | 'pages'>, id: ElementId): PageId | undefined;
/** All live descendants of an element (depth-first, paint order). */
export declare function descendants(doc: Pick<DesignDoc, 'elements'>, id: ElementId | PageId): Element[];
/** Elements on a page in paint order (flattened, groups expanded). */
export declare function pageElements(doc: Pick<DesignDoc, 'elements'>, pageId: PageId): Element[];
/** Ancestors of an element up to (not including) its page, nearest first. */
export declare function ancestors(doc: Pick<DesignDoc, 'elements'>, id: ElementId): Element[];
