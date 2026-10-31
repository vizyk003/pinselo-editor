// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-sheet (see ../README.md).
/** Display formatting of computed cell values (deterministic — the editor, exports and server agree). */
import type { CellValue, ValueKind } from './engine';
import type { CellFormat } from './model';
export declare function serialToDate(serial: number): Date;
export declare function dateToSerial(d: Date): number;
/** "General" number display: up to 10 significant digits, no trailing zeros, exponent for extremes. */
export declare function generalNumber(n: number): string;
export declare function formatValue(v: CellValue, fmt?: CellFormat, kind?: ValueKind): string;
/** Default horizontal alignment: numbers right, booleans/errors centre, text left. */
export declare function autoAlign(v: CellValue, fmt?: CellFormat): 'left' | 'center' | 'right';
