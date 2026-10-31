// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
import type { Mat2D, Rect, Vec2, Xf } from '../schema';
export declare const IDENTITY: Mat2D;
export declare function multiply(m: Mat2D, n: Mat2D): Mat2D;
export declare function translate(tx: number, ty: number): Mat2D;
export declare function scale(sx: number, sy?: number): Mat2D;
export declare function rotate(deg: number): Mat2D;
export declare function invert(m: Mat2D): Mat2D;
export declare function apply(m: Mat2D, p: Vec2): [number, number];
export declare function applyVector(m: Mat2D, v: Vec2): [number, number];
/**
 * Matrix mapping an element's own box space (0..w, 0..h) to its parent's space.
 * Rotation and flips are about the box centre.
 */
export declare function localMatrix(xf: Xf): Mat2D;
export declare function normalizeAngle(deg: number): number;
export declare function rectCorners(r: Rect): [Vec2, Vec2, Vec2, Vec2];
export declare function boundsOfPoints(pts: readonly Vec2[]): Rect;
export declare function unionRects(rs: readonly Rect[]): Rect;
export declare function rectsIntersect(a: Rect, b: Rect): boolean;
export declare function rectContains(outer: Rect, inner: Rect): boolean;
/** Decompose a (rotation + translation + uniform-ish scale) matrix applied to a w×h box back into an Xf. */
export declare function xfFromMatrix(m: Mat2D, w: number, h: number): Xf;
