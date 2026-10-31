// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
import { type DesignDoc, type Element, type ElementId, type Rect, type TextBody, type Vec2, type Xf } from '../schema';
export type Handle = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';
export declare const CORNERS: readonly Handle[];
export declare const EDGES: readonly Handle[];
export interface ResizeResult {
    xf: Xf;
    sx: number;
    sy: number;
    anchor: [number, number];
}
/**
 * Resize an (optionally rotated) box by dragging `handle` to point `p` (parent space).
 * The opposite edge/corner stays fixed. `keepAspect` forces uniform scaling.
 */
export declare function resizeBox(xf: Xf, handle: Handle, p: Vec2, keepAspect: boolean, min?: number): ResizeResult;
/** Scale a box uniformly about an anchor point (multi-selection resize). */
export declare function scaleAbout(xf: Xf, anchor: Vec2, s: number): Xf;
/** Rotate a box's centre about a pivot and add to its rotation. */
export declare function rotateAbout(xf: Xf, pivot: Vec2, delta: number): Xf;
export declare function scaleBody(body: TextBody, s: number): TextBody;
/**
 * Element-specific consequences of a box scale (sx, sy) on an element whose xf was already updated:
 * text font sizes (uniform only), line points, and the same for every descendant of a group.
 * Returns patches keyed by element id (the root's patch excludes xf).
 */
export declare function scalePatches(doc: Pick<DesignDoc, 'elements'>, el: Element, sx: number, sy: number): Map<ElementId, Partial<Element>>;
/** Image edge-drag crops instead of stretching: keep the image scale, reveal more or less of the source. */
export declare function cropResize(xf: Xf, crop: Rect, next: Xf, handle: Handle): {
    xf: Xf;
    crop: Rect;
};
/**
 * Crop for a photo shown in a box of another aspect without stretching (photo-filled shapes): `from` is the shape
 * the crop was made for (e.g. the crop in source pixels), `to` the box. The visible part grows along the axis that
 * grew relatively more (revealing more of the source at the same scale) and zooms in once it reaches the edge of
 * the source. The same aspect (within 1e-6) keeps the crop as is. Centre kept, clamped inside the source.
 * render-core `refitCropFor` is a copy (keep them identical: the editor and every renderer must agree).
 */
export declare function refitCrop(crop: Rect, from: {
    w: number;
    h: number;
}, to: {
    w: number;
    h: number;
}): Rect;
