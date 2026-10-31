// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Keyframe interpolation (MV-100), shared by the renderer (playback/export) and the editor (recording). */
import type { Keyframe, KeyframeEase } from './elements';
export declare const EASES: Record<KeyframeEase, (p: number) => number>;
/** Neutral value of each keyframed property. */
export declare const KEYFRAME_NEUTRAL: Record<string, number>;
/** Interpolated value of a keyframe track at page time `t` (before the first key → first value, after the last → last). */
export declare function keyframeValue(keys: readonly Keyframe[] | undefined, t: number, neutral?: number): number;
