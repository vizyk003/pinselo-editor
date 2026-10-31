// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/** Smart guides: snap a moving box's edges/centre to page and sibling edges/centres. docs/plan/03 §7.3. */
import type { Rect } from '../schema';
export interface Guide {
    axis: 'x' | 'y';
    /** Page-space coordinate of the guide line. */
    pos: number;
    /** Extent of the guide along the other axis. */
    from: number;
    to: number;
    kind: 'page' | 'element' | 'ruler';
}
/** Equal-spacing marker (ED-060): a measured gap drawn between two boxes. `axis` is the measuring direction. */
export interface SpacingMark {
    axis: 'x' | 'y';
    from: number;
    to: number;
    at: number;
    gap: number;
}
export interface SnapResult {
    dx: number;
    dy: number;
    guides: Guide[];
    spacing: SpacingMark[];
}
/**
 * @param moving    the moving selection's bounds (already offset by the raw drag delta)
 * @param page      page rect (0,0,w,h)
 * @param targets   bounds of the other elements on the page
 * @param threshold snap distance in page px (callers pass screenPx / zoom)
 */
export declare function snapMove(moving: Rect, page: Rect, targets: readonly Rect[], threshold: number, rulers?: ReadonlyArray<{
    axis: 'x' | 'y';
    pos: number;
}>): SnapResult;
/** Snap a single value (used for resize edges). */
export declare function snapValue(v: number, candidates: readonly number[], threshold: number): {
    value: number;
    snapped: number | null;
};
/** Rotation snapping: magnetic to multiples of 45° within ±3°, or hard 15° steps with Shift. */
export declare function snapAngle(deg: number, shift: boolean): number;
/**
 * Equal-spacing snap along one axis: centre the box between its two neighbours, or match a gap that
 * already exists between other elements in the same row/column.
 */
export declare function snapSpacing(moving: Rect, targets: readonly Rect[], axis: 'x' | 'y', threshold: number): {
    delta: number;
    marks: SpacingMark[];
} | null;
