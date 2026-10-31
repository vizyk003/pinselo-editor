// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/** Frame outlines (A13): library clip paths scale from the unit box; built-in clips use the shape generator. */
import type { FrameEl, FrameOverlay } from '../schema';
/** Clip outline of a frame in its box space (0..w, 0..h). */
export declare function frameOutline(el: Pick<FrameEl, 'clip' | 'clipPath'>, w: number, h: number): string;
/** Overlay chrome paths of a frame in its box space. */
export declare function frameOverlays(el: Pick<FrameEl, 'overlay'>, w: number, h: number): Array<FrameOverlay & {
    path: string;
}>;
