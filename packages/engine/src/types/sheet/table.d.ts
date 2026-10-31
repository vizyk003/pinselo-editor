// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/**
 * Sheet → table elements (A15): a range becomes an ordinary `TableEl` (so it can be pasted into any design),
 * and a whole sheet is paginated onto fixed pages of tables for PDF/PNG export and thumbnails through the
 * existing render pipeline.
 */
import { type DesignDoc, type TableEl } from '../schema';
import type { Range } from './a1';
import type { SheetEngine } from './engine';
import { type TabSnap } from './model';
export interface TableOpts {
    x?: number;
    y?: number;
    scale?: number;
    family?: string;
    rows?: number[];
}
/**
 * A table element for `range` (or for an explicit list of row indexes via `opts.rows`, used to repeat frozen
 * header rows on every page). Sizes follow the sheet's column widths and row heights times `scale`.
 */
export declare function rangeToTable(engine: SheetEngine, t: TabSnap, range: Range, opts?: TableOpts): TableEl;
export interface SheetLayoutOpts {
    title: string;
    w: number;
    h: number;
    margin?: number;
    maxPages?: number;
    tabs?: string[];
}
/**
 * Lay a sheet out on pages: every tab's used range, scaled to fit the page width (down to 50%, after which
 * columns continue on further pages), rows split across pages with frozen header rows repeated.
 */
export declare function sheetToDesign(engine: SheetEngine, opts: SheetLayoutOpts): DesignDoc;
