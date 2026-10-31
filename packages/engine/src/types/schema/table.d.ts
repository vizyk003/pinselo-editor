// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { CellBorder, TableCell, TableEl } from './elements';
import type { Paint, Rect } from './primitives';
import { type RunStyle, type TextBody } from './text';
export type CellPos = {
    r: number;
    c: number;
};
/** Inclusive cell rectangle. */
export interface CellRange {
    r0: number;
    c0: number;
    r1: number;
    c1: number;
}
export type CellSide = 't' | 'r' | 'b' | 'l';
export type BorderPreset = 'all' | 'outer' | 'inner' | 'horizontal' | 'none';
export declare const BORDER_PRESETS: readonly BorderPreset[];
export declare const cellKey: (r: number, c: number) => string;
/** Track boundaries in table-local px: `xs` has cols+1 entries, `ys` rows+1. Track sizes are relative. */
export declare function tableGeometry(el: Pick<TableEl, 'rows' | 'cols' | 'xf'>): {
    ys: number[];
    xs: number[];
};
/** Span of an anchor cell, clipped to the table (1×1 when not merged). */
export declare function cellSpan(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, r: number, c: number): {
    rows: number;
    cols: number;
};
/** The anchor cell whose (merged) area contains (r, c). */
export declare function anchorOf(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, r: number, c: number): CellPos;
/** True when (r, c) is hidden under another cell's merge. */
export declare function isCovered(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, r: number, c: number): boolean;
/** Effective span honouring overlap resolution (what is actually drawn). */
export declare function effectiveSpan(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, r: number, c: number): {
    rows: number;
    cols: number;
};
/** Visible cells (anchors) in row-major order. */
export declare function tableAnchors(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>): Array<CellPos & {
    rows: number;
    cols: number;
}>;
/** Box of a (possibly merged) cell in table-local px. */
export declare function cellRect(el: Pick<TableEl, 'rows' | 'cols' | 'cells' | 'xf'>, r: number, c: number, geo?: {
    ys: number[];
    xs: number[];
}): Rect;
/** Grid slot under a table-local point (not resolved to its anchor), or null outside. */
export declare function slotAt(el: Pick<TableEl, 'rows' | 'cols' | 'xf'>, x: number, y: number, geo?: {
    ys: number[];
    xs: number[];
}): CellPos | null;
/** Rectangle spanned by two cells, grown until no merged cell straddles its edge. */
export declare function normalizeRange(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, a: CellPos, b: CellPos): CellRange;
/** Anchors whose cell lies inside the range. */
export declare function rangeAnchors(el: Pick<TableEl, 'rows' | 'cols' | 'cells'>, range: CellRange): CellPos[];
/** Run style every cell inherits from the table style (header row colour / weight included). */
export declare function tableDefaultRun(el: Pick<TableEl, 'style'>, r: number, cell?: TableCell): RunStyle;
/**
 * The cell's rich body. Accepts both shapes: rich cells (`body`) and legacy plain cells (`text`, `bold`,
 * `align`), which is the read-side migration — no stored data has to be rewritten. Memoised per cell object
 * so text layout caches (keyed by body identity) stay warm.
 */
export declare function cellBody(el: Pick<TableEl, 'style' | 'cells'>, r: number, c: number): TextBody;
/** Normalise a legacy cell to the rich shape (used when a cell is first edited, and by tests). */
export declare function migrateCell(el: Pick<TableEl, 'style' | 'cells'>, r: number, c: number): TableCell;
/** Cell with a new body and its plain-text mirror kept in sync. */
export declare function withBody(cell: TableCell | undefined, body: TextBody): TableCell;
export declare function cellPlainText(el: Pick<TableEl, 'style' | 'cells'>, r: number, c: number): string;
export declare function cellPadding(el: Pick<TableEl, 'style' | 'cells'>, r: number, c: number): number;
/** Text box of a cell (inside its padding), table-local px. */
export declare function cellContentBox(el: Pick<TableEl, 'rows' | 'cols' | 'cells' | 'xf' | 'style'>, r: number, c: number, geo?: {
    ys: number[];
    xs: number[];
}): Rect;
/** Vertical offset of laid-out text inside the content box for the cell's vertical alignment. */
export declare function cellTextOffset(el: Pick<TableEl, 'cells'>, r: number, c: number, boxH: number, textH: number): number;
/** Merge a range into its top-left cell. Contents of the other non-empty cells are appended as paragraphs. */
export declare function mergeCells(el: TableEl, range: CellRange): TableEl['cells'];
/** Split every merged cell intersecting the range (content stays in the top-left cell). */
export declare function unmergeCells(el: TableEl, range: CellRange): TableEl['cells'];
export declare function canUnmerge(el: TableEl, range: CellRange): boolean;
/**
 * Insert a row/column after index `at` (-1 = before the first) or delete the one at `at`. Cell keys shift,
 * merged cells that straddle the edit grow/shrink, and the box grows/shrinks by one average track.
 */
export declare function tableStructureEdit(el: TableEl, action: 'addRow' | 'addCol' | 'delRow' | 'delCol', at: number): Pick<TableEl, 'rows' | 'cols' | 'cells' | 'xf'> | null;
export declare function defaultBorder(el: Pick<TableEl, 'style'>): CellBorder;
/** Resolved border on one side of an anchor cell (as the shared edge is drawn; first neighbour for merged sides). */
export declare function cellSideBorder(el: Pick<TableEl, 'rows' | 'cols' | 'cells' | 'style'>, r: number, c: number, side: CellSide): CellBorder;
export interface TableEdge {
    x1: number;
    y1: number;
    x2: number;
    y2: number;
    border: CellBorder;
}
/** Visible border segments (merged interiors skipped, collinear runs joined), table-local px. */
export declare function tableEdges(el: Pick<TableEl, 'rows' | 'cols' | 'cells' | 'xf' | 'style'>, geo?: {
    ys: number[];
    xs: number[];
}): TableEdge[];
/**
 * Apply a border preset to a range: `all` every line, `outer` the perimeter only, `inner` the inside lines,
 * `horizontal` horizontal lines only, `none` clears. Neighbours outside the range get the matching facing side
 * so the shared edge resolves the same way from both cells.
 */
export declare function applyBorderPreset(el: TableEl, range: CellRange, preset: BorderPreset, border: CellBorder): TableEl['cells'];
export type TableStylePreset = 'minimal' | 'striped' | 'header-dark' | 'bordered' | 'card' | 'colourful';
export declare const TABLE_STYLE_PRESETS: readonly TableStylePreset[];
/** Per-preset swatch colours for the picker (header, stripe/body, border). */
export declare function tableStyleSwatch(preset: TableStylePreset): {
    header: string;
    body: string;
    stripe: string;
    border: string;
};
/**
 * Apply a table style preset. Keeps content, merges, fonts, padding and per-cell alignment; resets per-cell
 * fills and borders so the preset shows as designed.
 */
export declare function applyTableStyle(el: TableEl, preset: TableStylePreset): Pick<TableEl, 'style' | 'cells'>;
/** Fill a cell shows: its own, else header / stripe from the table style. */
export declare function cellFill(el: Pick<TableEl, 'style' | 'cells'>, r: number, c: number): Paint | undefined;
/**
 * Rows grow to fit their content (A12). `measure` returns the laid-out text height of a body at a width.
 * Returns the new track sizes (px; they sum to the new box height) or null when every row already fits.
 * Rows never shrink here: a row keeps the height the user gave it as its minimum.
 */
export declare function tableFitRows(el: Pick<TableEl, 'rows' | 'cols' | 'cells' | 'xf' | 'style'>, measure: (key: string, body: TextBody, width: number) => number, id?: string): {
    rows: number[];
    h: number;
} | null;
/** Even column widths (keeps the box width). */
export declare function distributeColumns(el: Pick<TableEl, 'cols'>): number[];
/** Move the boundary after column `i` to table-local x, keeping both neighbours at least `min` px wide. */
export declare function resizeColumn(el: Pick<TableEl, 'cols' | 'rows' | 'xf'>, i: number, x: number, min?: number): number[];
