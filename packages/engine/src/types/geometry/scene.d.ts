// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/** Element-level geometry over a DesignDoc snapshot: world matrices, bounds, hit-testing. */
import { type DesignDoc, type Element, type ElementId, type Mat2D, type PageId, type Rect, type Vec2 } from '../schema';
type Doc = Pick<DesignDoc, 'elements' | 'pages'>;
/** Matrix from the element's box space to page space (groups compose). */
export declare function worldMatrix(doc: Doc, id: ElementId): Mat2D;
/** Matrix from page space into the element's parent space (identity for top-level elements). */
export declare function parentInverse(doc: Doc, el: Element): Mat2D;
/** Corners of the element box in page space (rotated quad). */
export declare function worldQuad(doc: Doc, id: ElementId): [Vec2, Vec2, Vec2, Vec2];
export declare function worldBounds(doc: Doc, id: ElementId): Rect;
export declare function selectionBounds(doc: Doc, ids: readonly ElementId[]): Rect | null;
/** Does page-space point p hit the element (shape-accurate for shapes; box for everything else)? */
export declare function hitElement(doc: Doc, el: Element, p: Vec2, tolerance?: number): boolean;
/**
 * Topmost selectable element under p on a page. Clicking inside a group selects the group
 * (unless `enterGroup` names that group, in which case its children are tested).
 */
export declare function hitTest(doc: Doc, pageId: PageId, p: Vec2, opts?: {
    tolerance?: number;
    enterGroup?: ElementId | null;
}): Element | null;
/** Top-level elements of a page whose world bounds intersect (or are contained by) the marquee. */
export declare function marqueeSelect(doc: Doc, pageId: PageId, r: Rect): Element[];
