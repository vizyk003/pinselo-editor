// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import { type DesignDoc, type DesignMeta, type Page, type Surface } from './design';
import type { ChartEl, ChartKind, DrawEl, EmbedEl, FormEl, FormField, GridEl, VideoEl, ElementBase, FrameClip, FrameEl, GroupEl, ImageEl, LineEl, LineMarker, QrEl, ShapeEl, ShapeKind, TableEl, TextEl } from './elements';
import type { AssetId, ElementId, PageId, Paint, Rect, Stroke, Unit, Xf } from './primitives';
import { type RunStyle, type TextBody } from './text';
export declare const DEFAULT_FONT = "Inter";
export declare const DEFAULT_RUN: RunStyle;
type BaseInit = Partial<Omit<ElementBase, 'id' | 'type'>> & {
    xf: Xf;
};
export declare function xf(x: number, y: number, w: number, h: number, rot?: number): Xf;
export declare function textBody(text: string, style?: Partial<RunStyle>, align?: TextBody['paragraphs'][number]['align']): TextBody;
export declare function makeText(init: BaseInit & {
    text?: string;
    style?: Partial<RunStyle>;
    align?: 'start' | 'center' | 'end' | 'justify';
    body?: TextBody;
}): TextEl;
export declare function makeShape(init: BaseInit & {
    kind?: ShapeKind;
    fill?: Paint | string;
    stroke?: Stroke;
    params?: Record<string, number>;
}): ShapeEl;
export declare function makeStroke(color?: string, width?: number, dash?: number[]): Stroke;
export declare function makeLine(init: BaseInit & {
    color?: string;
    width?: number;
    dash?: number[];
    startMarker?: LineMarker;
    endMarker?: LineMarker;
}): LineEl;
export declare function makeImage(init: BaseInit & {
    asset: AssetId;
}): ImageEl;
/** A frame with a built-in `clip`, or a frame library `preset` (A13: its outline and chrome are copied in). */
export declare function makeFrame(init: BaseInit & {
    clip?: FrameClip;
    asset?: AssetId;
    preset?: string;
}): FrameEl;
export declare const GRID_LAYOUTS: Record<string, {
    label: string;
    cells: Rect[];
}>;
export declare function makeGrid(init: BaseInit & {
    layout?: string;
    cells?: Rect[];
    gap?: number;
}): GridEl;
/** Pixel rect of a grid cell inside the element box, after the gap. */
export declare function gridCellBox(el: Pick<GridEl, 'gap' | 'xf'>, cell: Rect): Rect;
export declare function makeGroup(init: BaseInit): GroupEl;
export declare function makeQr(init: BaseInit & {
    data: string;
}): QrEl;
export declare function makePage(init?: Partial<Page>): Page;
export declare function makeMeta(init: {
    w: number;
    h: number;
    unit?: Unit;
    title?: string;
    surface?: Surface;
}): DesignMeta;
export declare function emptyDesign(init: {
    w: number;
    h: number;
    unit?: Unit;
    title?: string;
    surface?: Surface;
    pages?: number;
}): DesignDoc;
/** Assign `parentId` + an ordered z key to a list of elements added to the same parent. */
export declare function placeAll<T extends ElementBase>(parentId: PageId | ElementId, els: T[], after?: string | null): T[];
export declare function makeDraw(init: BaseInit & {
    points: number[];
    brush?: DrawEl['brush'];
    color?: string;
    size?: number;
}): DrawEl;
export declare function makeTable(init: BaseInit & {
    rows?: number;
    cols?: number;
    data?: string[][];
}): TableEl;
export declare const CHART_COLORS: string[];
export declare function makeChart(init: BaseInit & {
    kind?: ChartKind;
    labels?: string[];
    series?: Array<{
        name: string;
        values: number[];
        color?: string;
    }>;
}): ChartEl;
export declare function makeVideo(init: BaseInit & {
    asset: AssetId;
    durationMs: number;
}): VideoEl;
/** Played length of a clip in page time. */
export declare function clipLengthMs(c: {
    trimStartMs: number;
    trimEndMs: number;
    speed: number;
}): number;
/** Form layout (FM-001): vertical, deterministic — shared by the renderer, the editor and the live HTML overlay. */
export interface FormRow {
    kind: 'title' | 'label' | 'input' | 'option' | 'submit';
    fieldId?: string;
    option?: string;
    x: number;
    y: number;
    w: number;
    h: number;
    text?: string;
}
export declare function formLayout(el: Pick<FormEl, 'title' | 'fields' | 'submitLabel' | 'style' | 'xf'>): {
    rows: FormRow[];
    height: number;
};
export declare function makeForm(init: BaseInit & {
    title?: string;
    fields?: FormField[];
}): FormEl;
/** Embedded media (A11); `link` makes static exports (PDF, PPTX) open the original URL. */
export declare function makeEmbed(init: BaseInit & {
    url: string;
    provider: EmbedEl['provider'];
    src: string;
    title?: string;
    poster?: AssetId;
}): EmbedEl;
