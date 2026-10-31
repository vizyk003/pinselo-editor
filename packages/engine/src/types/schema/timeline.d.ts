// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Design timeline (MV-001): pages play back-to-back; each element has an optional visibility window. */
import type { DesignDoc, Page } from './design';
import type { Element, MediaClip, VideoEl } from './elements';
import type { AssetId } from './primitives';
export declare const DEFAULT_PAGE_MS = 5000;
/** Page length: explicit, else long enough for its videos (Canva behaviour), else 5 s. */
export declare function pageDurationMs(doc: Pick<DesignDoc, 'elements'>, page: Page): number;
export interface PageSpan {
    page: Page;
    index: number;
    startMs: number;
    durationMs: number;
}
export declare function timelineOf(doc: DesignDoc, includeHidden?: boolean): {
    spans: PageSpan[];
    totalMs: number;
};
/** Which page plays at design time `t`, and the time inside it. */
export declare function locateTime(doc: DesignDoc, t: number): {
    span: PageSpan;
    pageTimeMs: number;
} | null;
/** Visibility window of an element in page time. */
export declare function elementWindow(el: Element, pageMs: number): {
    start: number;
    end: number;
};
/** Source time (ms) of a video at page time t, or null when it isn't playing. */
export declare function videoSourceTime(el: VideoEl, pageTimeMs: number, pageMs: number): number | null;
/**
 * Source time of a clip played by element `el` (a video, or a frame / grid cell holding a video) at page time t,
 * or null outside the element's window. A finished non-looping clip inside a frame holds its last frame.
 */
export declare function mediaSourceTime(el: Element, clip: MediaClip, pageTimeMs: number, pageMs: number): number | null;
/** One video played by an element: `key` is unique per playback (grid cells get `<id>#<index>`). */
export interface VideoSlot {
    key: string;
    asset: AssetId;
    clip: MediaClip;
}
/** Every video an element plays (A13): video elements, frames holding a video and video grid cells. */
export declare function videoSlots(el: Element): VideoSlot[];
