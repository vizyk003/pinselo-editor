// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/**
 * Line-end markers as SVG path data in marker space: the line end sits at (0, 0) and the line arrives
 * from −x. Shared by the canvas painter, SVG and PDF exporters so every surface draws identical ends.
 */
import type { LineMarker } from '../schema';
export interface MarkerShape {
    d: string;
    /** 'fill' paints the area; 'stroke' strokes the outline with the line's width (round joins). */
    mode: 'fill' | 'stroke';
    /** How far to pull the line back from its end so it doesn't show through hollow markers. */
    inset: number;
}
export declare function lineMarkerShape(kind: LineMarker, width: number): MarkerShape | null;
/** The polyline with each end pulled back by its marker's inset (never past the neighbouring point). */
export declare function trimLineForMarkers(points: ReadonlyArray<readonly [number, number]>, start: LineMarker, end: LineMarker, width: number): Array<[number, number]>;
