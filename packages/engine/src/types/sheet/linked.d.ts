// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/** Resolving linked-chart data from a sheet design's stored Yjs state (API and render service). */
import type { DesignDoc } from '../schema';
import { SheetEngine, type CellValue } from './engine';
/** A read-only engine over a stored state (call `destroy()` when done). */
export declare function openSheetState(state: Uint8Array): SheetEngine;
/** Values of `range` on tab `sheet` (tab id or name; a "Tab!A1:B2" range overrides it). */
export declare function readRange(engine: SheetEngine, sheet: string, range: string): {
    tabId: string;
    tabName: string;
    values: CellValue[][];
} | null;
/**
 * Refresh every linked chart in `doc` from its source sheet. `load` returns a sheet design's state (or null
 * when missing / not accessible — the chart then keeps its cached values). Returns the number refreshed.
 */
export declare function resolveLinkedCharts(doc: DesignDoc, load: (designId: string) => Promise<Uint8Array | null>): Promise<number>;
