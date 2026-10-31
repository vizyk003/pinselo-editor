// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Design + pages. docs/plan/03-document-model-and-renderer.md §2.2. */
import type { Element } from './elements';
import type { AssetId, ElementId, PageId, Paint, Unit } from './primitives';
export type Surface = 'graphic' | 'presentation' | 'video' | 'doc' | 'sheet' | 'whiteboard' | 'print' | 'website' | 'email';
export declare const CURRENT_SCHEMA: 1;
export declare const LAYOUT_VERSION: 1;
export interface DesignMeta {
    schemaVersion: number;
    minClientSchema: number;
    layoutVersion: number;
    surface: Surface;
    /** Page size in document px at 96 dpi; `unit` is display-only. */
    dims: {
        w: number;
        h: number;
        unit: Unit;
        dpi: number;
    };
    lang: string;
    title: string;
    epoch: number;
    /** Template this design started from; `brand` = a team brand template whose locks apply (A8). */
    templateOrigin?: {
        templateId: string;
        brand?: boolean;
        workspaceId?: string;
        version?: number;
    };
    /** Design soundtrack (atomic list, LWW). */
    audio?: AudioClip[];
    captions?: CaptionTrack;
}
export interface Page {
    id: PageId;
    /** Fractional index; page order = sort by (order, id). */
    order: string;
    title?: string;
    hidden?: boolean;
    /** Locked page (g96): its elements can't be selected, moved or edited until the page is unlocked. */
    locked?: boolean;
    background: Paint[];
    notes?: string;
    /** Page length in video/GIF export and auto-advancing presentations (default 5000). */
    durationMs?: number;
    /** Custom ruler guides (ED-061), page px. */
    guides?: Array<{
        axis: 'x' | 'y';
        pos: number;
    }>;
    /** Transition played when this page appears (MV-010). */
    transition?: {
        kind: TransitionKind;
        durationMs: number;
        direction?: import('./elements').AnimDirection;
    };
    /** Page animation: applied (staggered) to every element without its own animation. */
    anim?: {
        preset: import('./elements').AnimPreset;
        durationMs: number;
    };
    /**
     * Published websites: how this page looks on phones. 'auto' (default) reflows wide pages into a single
     * column; 'desktop' keeps the desktop layout scaled down.
     */
    mobileLayout?: 'auto' | 'desktop';
}
/** Page transitions (MV-010). */
export type TransitionKind = 'fade' | 'dissolve' | 'slide' | 'push' | 'cover' | 'reveal' | 'zoom' | 'wipe' | 'iris' | 'split' | 'flip' | 'blur' | 'spin' | 'drop';
export declare const TRANSITION_KINDS: readonly TransitionKind[];
export interface AssetRef {
    id: AssetId;
    kind: 'image' | 'svg' | 'video' | 'audio';
    sha256: string;
    mime: string;
    w: number;
    h: number;
    name?: string;
    /** Animated GIF stored as a looping, silent video. */
    animated?: boolean;
    /** Licence attribution for stock/library media. */
    attribution?: {
        provider: string;
        author?: string;
        authorUrl?: string;
        license?: string;
        licenseUrl?: string;
        sourceUrl?: string;
    };
    /** Media length (video/audio). */
    durationMs?: number;
    hasAudio?: boolean;
}
/** One clip on the design-level soundtrack (MV-030). Times in design time (pages played back-to-back). */
export interface AudioClip {
    id: string;
    asset: AssetId;
    name?: string;
    /** Where the clip starts in design time. */
    startMs: number;
    /** In-point inside the source. */
    trimStartMs: number;
    /** Played length (source time). */
    durationMs: number;
    volume: number;
    fadeInMs: number;
    fadeOutMs: number;
    loop?: boolean;
}
/** A caption cue rendered over the design (MV captions). Times in design time. */
export interface CaptionCue {
    id: string;
    startMs: number;
    endMs: number;
    text: string;
}
export interface CaptionTrack {
    cues: CaptionCue[];
    style: {
        family: string;
        size: number;
        color: string;
        background: string;
        position: 'bottom' | 'top' | 'middle';
    };
    visible: boolean;
}
/** Plain-JSON snapshot of a design (the form used by templates, fixtures, exports and the API). */
export interface DesignDoc {
    meta: DesignMeta;
    pages: Record<PageId, Page>;
    elements: Record<ElementId, Element>;
    assets: Record<AssetId, AssetRef>;
}
