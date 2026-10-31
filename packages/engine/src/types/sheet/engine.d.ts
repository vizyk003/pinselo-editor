// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/**
 * Formula engine (A15): HyperFormula (GPL-3.0-only — compatible with Pinselo' AGPL-3.0-only via the
 * GPLv3/AGPLv3 §13 combination clause; configured with licenseKey "gpl-v3") kept in sync with the Yjs model.
 *
 * - Tabs map to HyperFormula sheets by name, so `=Sheet2!A1` / `='My tab'!B3` work across tabs.
 * - Cell edits (local or remote) are applied incrementally; row/column inserts and deletes are replayed
 *   from the Y.Array deltas, so references re-point the way they do in Excel.
 * - Structural edits made here (insert/delete rows/cols, rename/delete tab) let HyperFormula rewrite every
 *   affected formula and write the rewritten text back in the SAME transaction, so collaborators and the
 *   stored document agree.
 */
/** The TS engine's HyperFormula instance (opaque here; the facade's engine is oc-sheet). */
type HyperFormula = unknown;
type RawCellContent = unknown;
import * as Y from 'yjs';
import type { Range } from './a1';
import { type CellFormat, splitKey, type TabSnap } from './model';
export type CellValue = number | string | boolean | null | {
    error: string;
};
/** HyperFormula's detailed number kinds (used by "auto" number formatting). */
export type ValueKind = 'number' | 'date' | 'time' | 'datetime' | 'currency' | 'percent' | 'other';
export declare const HF_CONFIG: {
    readonly licenseKey: "gpl-v3";
    readonly dateFormats: readonly ["YYYY-MM-DD", "YYYY/MM/DD", "DD/MM/YYYY", "DD.MM.YYYY"];
    readonly timeFormats: readonly ["hh:mm", "hh:mm:ss"];
    readonly currencySymbol: readonly ["$", "€", "£", "¥"];
    readonly useColumnIndex: false;
    readonly maxRows: 200000;
    readonly maxColumns: 1000;
};
/** What HyperFormula should see for a cell: text-formatted cells are forced to strings. */
export declare function hfInput(raw: string | undefined, fmt: CellFormat | undefined): RawCellContent;
export declare class SheetEngine {
    readonly root: Y.Map<unknown>;
    hf: HyperFormula;
    tabs: TabSnap[];
    /** Bumped on every change (cheap memo key for UIs). */
    version: number;
    private hfSheet;
    private byMap;
    private listeners;
    private observer;
    constructor(root: Y.Map<unknown>, opts?: {
        live?: boolean;
    });
    get ydoc(): Y.Doc;
    destroy(): void;
    subscribe(fn: () => void): () => void;
    private emit;
    /** Rebuild every sheet from scratch (tab added/removed/renamed, or first load). */
    rebuild(): void;
    private indexMaps;
    tab(id: string): TabSnap | undefined;
    private onEvents;
    private resnap;
    private addr;
    value(tabId: string, r: number, c: number): CellValue;
    kind(tabId: string, r: number, c: number): ValueKind;
    /** Computed values of a range (row-major). */
    rangeValues(tabId: string, r: Range): CellValue[][];
    /** Every formula cell's HyperFormula text, keyed "tabId|cellKey". */
    private formulas;
    /**
     * Run a structural change: `hfOp` applies it to HyperFormula, `yOp` to the Yjs model. Formulas that
     * HyperFormula rewrote (shifted references, #REF!, renamed sheets) are written back in the same transaction.
     */
    private structural;
    insert(tabId: string, axis: 'rows' | 'cols', at: number, n: number): void;
    remove(tabId: string, axis: 'rows' | 'cols', at: number, n: number): void;
    renameTab(tabId: string, name: string): string;
    deleteTab(tabId: string): void;
}
/** Split "rowId:colId" (re-exported for UIs). */
export { splitKey };
