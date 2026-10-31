// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/**
 * Parametric shape outlines as SVG path data in the element's box space (0..w, 0..h).
 * Shared by the renderer (both backends), hit-testing and exporters, so every surface agrees.
 */
import type { FrameClip, ShapeKind } from '../schema';
/** Scale a unit-box path (absolute M/L/C/Q/Z, coordinates in 0..1) to w × h. */
export declare function scaleUnitPath(d: string, w: number, h: number): string;
export declare function shapePath(kind: ShapeKind | FrameClip, w: number, h: number, params?: Record<string, number>, d?: string): string;
/** Point-in-polygon for straight-edged shapes; ellipse and rounded variants use analytical checks. */
export declare function hitShape(kind: ShapeKind | FrameClip, w: number, h: number, x: number, y: number, params?: Record<string, number>): boolean;
