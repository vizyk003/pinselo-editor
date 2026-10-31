// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/**
 * Sheet data model in Yjs (A15). Lives in the design's own Y.Doc (room `design:{id}`) beside the design
 * roots, under the root map "sheet":
 *
 *   sheet: Y.Map
 *     tabs:  Y.Map<tabId, Y.Map>     one per sheet tab
 *     order: Y.Array<tabId>          tab order
 *   tab: Y.Map
 *     name: string
 *     rows / cols: Y.Array<string>   stable row / column ids, in display order
 *     cells: Y.Map<"rowId:colId", string>        raw input ("12", "hello", "=SUM(A1:A3)")
 *     fmts:  Y.Map<"rowId:colId", CellFormat>    per-cell format (atomic per cell, last writer wins)
 *     colW / rowH: Y.Map<id, number>             custom track sizes in px
 *     merges: Y.Map<"rowId:colId", {rows, cols}> merged range anchored at that cell
 *     frozenRows / frozenCols: number
 *
 * Cells are keyed by stable row/column ids rather than positions, so concurrent edits merge cell by cell
 * and an edit made while someone else inserts or deletes rows still lands on the cell the user meant.
 */
import * as Y from 'yjs';
import type { Range } from './a1';
export declare const SHEET_ROOT = "sheet";
/** Transaction origin for local edits (tracked by the undo manager). */
export declare const SHEET_LOCAL: unique symbol;
export type NumberFormat = 'auto' | 'number' | 'percent' | 'currency' | 'date' | 'datetime' | 'text';
export interface CellFormat {
    nf?: NumberFormat;
    /** Decimal places for number/percent/currency. */
    dp?: number;
    /** Currency symbol (default "$"). */
    cur?: string;
    b?: boolean;
    i?: boolean;
    u?: boolean;
    s?: boolean;
    /** Text colour / fill, "#rrggbb". */
    color?: string;
    fill?: string;
    align?: 'left' | 'center' | 'right';
    valign?: 'top' | 'middle' | 'bottom';
    wrap?: boolean;
    /** Font size in px (default 13). */
    size?: number;
}
export interface Merge {
    rows: number;
    cols: number;
}
export declare const DEFAULT_ROWS = 1000;
export declare const DEFAULT_COLS = 26;
export declare const DEFAULT_COL_W = 100;
export declare const DEFAULT_ROW_H = 24;
export declare const MAX_ROWS = 100000;
export declare const MAX_COLS = 702;
export declare const ckey: (rowId: string, colId: string) => string;
export declare function splitKey(k: string): [string, string];
/** 8 random base-62 characters (~47 bits: collisions are negligible even for concurrent bulk inserts). */
export declare function newTrackId(): string;
export declare const sheetRoot: (ydoc: Y.Doc) => Y.Map<unknown>;
export declare const tabsOf: (root: Y.Map<unknown>) => Y.Map<Y.Map<unknown>> | undefined;
export declare const orderOf: (root: Y.Map<unknown>) => Y.Array<string> | undefined;
export declare function createTab(name: string, rows?: number, cols?: number): Y.Map<unknown>;
/** True when the doc already has a sheet (at least one tab). */
export declare function hasSheet(ydoc: Y.Doc): boolean;
/** Create the sheet structure with one empty tab, unless it exists. Returns the first tab id. */
export declare function initSheet(ydoc: Y.Doc, opts?: {
    name?: string;
    rows?: number;
    cols?: number;
    origin?: unknown;
}): string;
/** Tab ids in display order (order array first, then any tab missing from it — e.g. after a race). */
export declare function tabIds(root: Y.Map<unknown>): string[];
/** Plain, read-only view of one tab with position lookups. Rebuilt when the tab structure changes. */
export interface TabSnap {
    id: string;
    name: string;
    map: Y.Map<unknown>;
    rows: string[];
    cols: string[];
    rowIx: Map<string, number>;
    colIx: Map<string, number>;
    cells: Y.Map<string>;
    fmts: Y.Map<CellFormat>;
    colW: Y.Map<number>;
    rowH: Y.Map<number>;
    merges: Y.Map<Merge>;
    frozenRows: number;
    frozenCols: number;
}
export declare function snapTab(id: string, map: Y.Map<unknown>): TabSnap;
export declare function snapTabs(root: Y.Map<unknown>): TabSnap[];
/** Find a tab by id, then by (case-insensitive) name. */
export declare function findTab(tabs: TabSnap[], ref: string): TabSnap | undefined;
/** Position of a cell key, or null when its row/column was deleted. */
export declare function posOf(t: TabSnap, key: string): {
    r: number;
    c: number;
} | null;
export declare const keyAt: (t: TabSnap, r: number, c: number) => string | null;
export declare const rawAt: (t: TabSnap, r: number, c: number) => string;
export declare const fmtAt: (t: TabSnap, r: number, c: number) => CellFormat | undefined;
export declare const colWidth: (t: TabSnap, c: number) => number;
export declare const rowHeight: (t: TabSnap, r: number) => number;
/** Smallest range holding every non-empty cell (null when the tab is empty). */
export declare function usedRange(t: TabSnap): Range | null;
/** Merged ranges in positions (anchors whose tracks were deleted are skipped; spans clipped to the grid). */
export declare function mergeRanges(t: TabSnap): Range[];
export declare function setRaw(t: TabSnap, r: number, c: number, raw: string): void;
export declare function patchFormat(t: TabSnap, r: number, c: number, patch: Partial<CellFormat> | null): void;
/** Insert `n` new tracks before index `at`. */
export declare function insertTracks(t: TabSnap, axis: 'rows' | 'cols', at: number, n: number): void;
/** Delete tracks [at, at+n) and every cell, format, size and merge anchored on them. */
export declare function deleteTracks(t: TabSnap, axis: 'rows' | 'cols', at: number, n: number): void;
export declare function setMerge(t: TabSnap, r: Range | null, anchor?: {
    r: number;
    c: number;
}): void;
/** Tab names must be unique (case-insensitive) — formulas reference tabs by name. */
export declare function uniqueTabName(root: Y.Map<unknown>, base: string, except?: string): string;
export declare function addTab(root: Y.Map<unknown>, name?: string, after?: string): string;
export declare function deleteTab(root: Y.Map<unknown>, id: string): void;
