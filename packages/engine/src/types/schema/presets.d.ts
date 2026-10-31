// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { Surface } from './design';
import type { Unit } from './primitives';
export type PresetGroup = 'Social media' | 'Presentations' | 'Print' | 'Video' | 'Marketing' | 'Documents' | 'Web' | 'Whiteboard';
export interface SizePreset {
    id: string;
    label: string;
    group: PresetGroup;
    w: number;
    h: number;
    /** Physical unit shown in the UI. Stored dims are always px @96 dpi. */
    unit: Unit;
    surface: Surface;
    /** Platform name (a proper noun, not translated) used for platform tabs and search, e.g. 'Instagram'. */
    platform?: string;
    /** Shown under "Popular" in the Create dialog. */
    popular?: boolean;
    /** Extra search words (English; the translated label is searched too). */
    keywords?: string[];
}
export declare const SIZE_PRESETS: SizePreset[];
export declare function findPreset(id: string | null | undefined): SizePreset | undefined;
/** Create-dialog categories (g08) in nav order. 'suggestions' and 'custom' are not preset groups. */
export type PresetCategoryId = 'suggestions' | 'social' | 'video' | 'presentations' | 'print' | 'documents' | 'web' | 'marketing' | 'whiteboard' | 'custom';
export declare const PRESET_CATEGORIES: Array<{
    id: PresetCategoryId;
    label: string;
    group?: PresetGroup;
}>;
/** Presets listed under a category. Social media also lists the platform video formats (Reels, Shorts...). */
export declare function presetsInCategory(cat: PresetCategoryId): SizePreset[];
/** Platform tabs for a list of presets, in first-seen order. */
export declare function presetPlatforms(list: SizePreset[]): string[];
/**
 * Presets matching a free-text query: every word must appear in the label (English or translated),
 * group, platform, keywords or the size ("1080", "a4"). An empty query matches nothing.
 */
export declare function searchPresets(q: string, translate?: (s: string) => string): SizePreset[];
/** Popular presets in a stable order (the Create dialog's "Popular" row). */
export declare function popularPresets(): SizePreset[];
export declare function tryNewPresets(recentIds: string[], max?: number): SizePreset[];
/** Most-recent-first list of preset ids with `id` moved to the front, de-duplicated and capped. Unknown ids are dropped. */
export declare function pushRecentPreset(list: unknown, id: string, max?: number): string[];
/** Fields `emptyDesign` needs for a new design from a preset and/or explicit size (custom size wins over the preset's size). */
export declare function presetDesignInit(preset: SizePreset | undefined, o?: {
    w?: number;
    h?: number;
    unit?: Unit;
    title?: string;
}): {
    w: number;
    h: number;
    unit: Unit;
    title: string;
    surface: Surface;
};
/** Shortcuts in the Custom size form (g14). Sizes are in their own unit. */
export declare const POPULAR_LAYOUTS: Array<{
    id: string;
    label: string;
    w: number;
    h: number;
    unit: Unit;
}>;
/**
 * The other side of a size whose aspect ratio is locked: `ratio` is w / h.
 * Kept to `decimals` places (two, what the physical-unit inputs show; 0 for whole pixels).
 */
export declare function lockedSide(changed: 'w' | 'h', value: number, ratio: number, decimals?: number): number;
export declare const UNIT_PX: Record<Unit, number>;
export declare function pxToUnit(px: number, unit: Unit): number;
export declare function unitToPx(v: number, unit: Unit): number;
export declare function formatDims(w: number, h: number, unit: Unit): string;
