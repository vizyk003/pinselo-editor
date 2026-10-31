// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
/** Scene layout (pages stacked vertically) and the scene ⇄ screen mapping. */
import { type DesignDoc, type Page, type PageId, type Rect, type Vec2 } from '../schema';
export interface Viewport {
    /** Scene coordinate shown at the top-left of the canvas. */
    x: number;
    y: number;
    /** Screen px per scene px. */
    zoom: number;
    /** Canvas size in CSS px. */
    width: number;
    height: number;
}
export declare const MIN_ZOOM = 0.1;
export declare const MAX_ZOOM = 8;
export interface PageSlot {
    page: Page;
    index: number;
    rect: Rect;
}
export declare function pageGap(doc: DesignDoc): number;
/** Visible (non-hidden-in-editor) pages laid out top to bottom in scene space. */
export declare function pageSlots(doc: DesignDoc): PageSlot[];
export declare function slotOf(doc: DesignDoc, pageId: PageId): PageSlot | undefined;
/** The page under a scene point (or the nearest one vertically). */
export declare function pageAtScene(doc: DesignDoc, p: Vec2): PageSlot | undefined;
export declare function sceneToScreen(vp: Viewport, p: Vec2): [number, number];
export declare function screenToScene(vp: Viewport, p: Vec2): [number, number];
export declare function clampZoom(z: number): number;
/** Zoom keeping the scene point under `screenPt` fixed. */
export declare function zoomAt(vp: Viewport, zoom: number, screenPt: Vec2): Viewport;
/** Fit a scene rect into the viewport with padding (screen px). */
export declare function fitRect(vp: Viewport, r: Rect, pad?: number): Viewport;
