// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
import type { SheetEngine } from './engine';
export declare function sheetToXlsx(engine: SheetEngine, opts?: {
    creator?: string;
}): Promise<Uint8Array>;
/** Read the first values of an XLSX workbook back (tests / import): tab name → rows of cell values. */
export declare function readXlsx(bytes: Uint8Array): Promise<Record<string, Array<Array<unknown>>>>;
