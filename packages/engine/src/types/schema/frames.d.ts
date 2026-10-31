// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { FrameClip, FrameOverlay } from './elements';
export type FrameCategory = 'basic' | 'shapes' | 'organic' | 'photo' | 'devices' | 'letters' | 'numbers';
export declare const FRAME_CATEGORIES: readonly FrameCategory[];
export interface FrameLibraryItem {
    id: string;
    /** English display name (the web UI translates it). */
    name: string;
    category: FrameCategory;
    /** Clip outline in the unit box. */
    d: string;
    /** Default width / height. */
    aspect: number;
    /** Built-in clip that old clients fall back to when they do not understand `clipPath`. */
    fallback: FrameClip;
    /** Decorative chrome above the content, unit box, nonzero fill (holes are counter-wound). */
    overlay?: FrameOverlay[];
    /** Extra search words. */
    tags?: string;
}
export declare const FRAME_LIBRARY: readonly FrameLibraryItem[];
export declare const frameLibraryItem: (id: string) => FrameLibraryItem | undefined;
