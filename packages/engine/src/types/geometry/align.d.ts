// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/** Align and distribute. Pure: returns per-id translation deltas in page space. */
import type { Rect } from '../schema';
export type AlignEdge = 'left' | 'hcenter' | 'right' | 'top' | 'vcenter' | 'bottom';
export declare function alignDeltas(items: ReadonlyArray<{
    id: string;
    bounds: Rect;
}>, edge: AlignEdge, to: Rect): Map<string, [number, number]>;
/** Equal gaps between items along an axis; first and last stay put. */
export declare function distributeDeltas(items: ReadonlyArray<{
    id: string;
    bounds: Rect;
}>, axis: 'x' | 'y'): Map<string, [number, number]>;
