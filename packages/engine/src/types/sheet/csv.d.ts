// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/** CSV / TSV (RFC 4180 quoting) — file import/export and clipboard exchange with other spreadsheets. */
import type { Range } from './a1';
import type { SheetEngine } from './engine';
import { type TabSnap } from './model';
/** Parse delimited text into rows of fields. Handles quoted fields with embedded delimiters, quotes and newlines. */
export declare function parseDelimited(text: string, delim?: string): string[][];
export declare function quoteField(s: string, delim?: string): string;
export declare function toDelimited(rows: string[][], delim?: string, eol?: string): string;
/** Displayed values of a range (what a user sees), as rows of strings. */
export declare function displayRows(engine: SheetEngine, t: TabSnap, r: Range): string[][];
/** Raw inputs (formulas as typed) of a range. */
export declare function rawRows(t: TabSnap, r: Range): string[][];
/** The whole used area of a tab as CSV (displayed values). */
export declare function tabToCsv(engine: SheetEngine, t: TabSnap): string;
/**
 * Write rows of raw inputs at (r, c), growing the grid when needed. Call inside a transaction; `t` must be
 * re-snapped by the caller afterwards if it keeps using it (row/col ids may have been appended).
 */
export declare function writeRows(t: TabSnap, r: number, c: number, rows: string[][], snap?: () => TabSnap): void;
