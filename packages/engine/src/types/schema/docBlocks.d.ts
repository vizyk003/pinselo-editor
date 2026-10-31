// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { ChartEl, ChartKind, ChartSeries } from './elements';
import type { Xf } from './primitives';
export declare const DOC_NODES: {
    readonly designEmbed: "designEmbed";
    readonly chart: "chart";
    readonly callout: "callout";
    readonly toc: "tableOfContents";
    readonly pageBreak: "pageBreak";
    readonly videoEmbed: "videoEmbed";
};
/** Content width of the doc page (816 px page − 2 × 80 px margins). Block widths are ≤ this. */
export declare const DOC_CONTENT_WIDTH = 656;
export type CalloutTone = 'info' | 'success' | 'warning' | 'danger' | 'note';
export declare const CALLOUT_TONES: Record<CalloutTone, {
    bg: string;
    border: string;
    icon: string;
}>;
export declare const calloutTone: (v: unknown) => CalloutTone;
export interface DocChartData {
    kind: ChartKind;
    labels: string[];
    series: ChartSeries[];
    style: ChartEl['style'];
}
export declare function defaultDocChart(kind?: ChartKind): DocChartData;
/** Lenient parse of the `data` attribute (a JSON string or object); falls back to defaults field by field. */
export declare function parseDocChart(raw: unknown): DocChartData;
export declare const serializeDocChart: (d: DocChartData) => string;
/** A chart element for a doc chart block (layout and image rendering reuse the design chart engine). */
export declare function docChartElement(d: DocChartData, box: Xf): ChartEl;
/** Clamp a block width (px at 100 % zoom) to the doc content width. */
export declare const docBlockWidth: (w: unknown, fallback?: number) => number;
