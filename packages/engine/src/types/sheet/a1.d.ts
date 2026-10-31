// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/** A1-style addressing: column letters, cell/range parsing and relative-reference shifting for paste. */
export interface CellAddr {
    r: number;
    c: number;
}
/** Inclusive, normalised (r0 ≤ r1, c0 ≤ c1). */
export interface Range {
    r0: number;
    c0: number;
    r1: number;
    c1: number;
}
/** 0 → "A", 25 → "Z", 26 → "AA". */
export declare function colName(c: number): string;
/** "A" → 0, "AA" → 26; -1 when not letters. */
export declare function colIndex(name: string): number;
export declare const a1: (r: number, c: number) => string;
export declare function parseA1(s: string): CellAddr | null;
export declare function normRange(a: CellAddr, b?: CellAddr): Range;
/** "B2", "A1:C5" or "'My tab'!A1:C5" → sheet name (if any) and range. */
export declare function parseRange(s: string): {
    sheet?: string;
    range: Range;
} | null;
export declare function formatRange(r: Range): string;
/** Sheet name as it must appear in a formula reference. */
export declare function quoteSheet(name: string): string;
/**
 * Shift the relative references of a formula by (dr, dc) — what copy/paste and fill do. Absolute parts
 * (`$A`, `$1`) stay; string literals and quoted sheet names are left alone; a reference pushed off the
 * grid becomes `#REF!`. Non-formulas are returned unchanged.
 */
export declare function shiftFormula(src: string, dr: number, dc: number): string;
