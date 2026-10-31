// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/**
 * Linked charts (A15): a sheet range → chart labels + series, and the refresh mapping applied to a ChartEl.
 *
 * Layout convention (what Excel/Sheets/Canva infer): series run down columns. When the first column holds
 * text it is the category labels; when the first row holds text it is the series names.
 */
import { type ChartEl, type ChartSeries, type ChartSource } from '../schema';
import type { CellValue } from './engine';
export interface ChartData {
    labels: string[];
    series: Array<{
        name: string;
        values: number[];
    }>;
}
export declare function rangeToChartData(values: CellValue[][]): ChartData;
/** Patch for a chart element with fresh data; existing series colours are kept (by position). */
export declare function chartPatch(el: Pick<ChartEl, 'series'>, data: ChartData, source?: ChartSource): {
    labels: string[];
    series: ChartSeries[];
    source?: ChartSource;
};
/** True when the patch would change what the chart shows. */
export declare function chartChanged(el: Pick<ChartEl, 'labels' | 'series'>, data: ChartData): boolean;
