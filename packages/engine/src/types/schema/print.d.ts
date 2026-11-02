// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { DesignDoc } from './design';
import type { ColorValue, ElementId, PageId, Unit, Xf } from './primitives';
/** A standard trim size (or, for apparel/mugs/bags, the print area). Stored in px @ 96 dpi like design dims. */
export interface PrintSize {
    id: string;
    w: number;
    h: number;
}
export interface PrintProduct {
    id: string;
    sizes: PrintSize[];
    /** Bleed added around the trim (0 for print-area products such as t-shirts). */
    bleedMm: number;
    rotatable: boolean;
}
/** The sizes the editor recognises as print products, for default CMYK and bleed in PDF Print export. */
export interface PrintCatalog {
    products: PrintProduct[];
}
export declare const DEFAULT_PRINT_CATALOG: PrintCatalog;
export type PreflightCode = 'size_mismatch' | 'size_scaled' | 'page_count_over' | 'page_count_under' | 'low_dpi' | 'dpi_below_recommended' | 'text_outside_safe' | 'bleed_missing' | 'out_of_gamut' | 'thin_line' | 'font_missing' | 'not_printable';
export type PreflightFix = {
    kind: 'resize-design';
    w: number;
    h: number;
    unit: Unit;
} | {
    kind: 'extend-to-bleed';
    elementId: ElementId;
    xf: Xf;
} | {
    kind: 'thicken-line';
    elementId: ElementId;
    width: number;
} | {
    kind: 'add-page';
};
export interface PreflightIssue {
    code: PreflightCode;
    /** error: blocks an order; warning: needs acknowledging; info: advice. */
    severity: 'error' | 'warning' | 'info';
    pageId?: PageId;
    pageIndex?: number;
    elementId?: ElementId;
    /** Values for the message (numbers are already rounded). */
    vars: Record<string, string | number>;
    /** English message (server logs, emails, staff console); the editor translates by `code`. */
    message: string;
    fix?: PreflightFix;
}
export interface PreflightTarget {
    /** Trim size in px @ 96 dpi. */
    w: number;
    h: number;
    unit: Unit;
    bleedMm: number;
    safeMm: number;
    rotatable: boolean;
    minDpi: number;
    goodDpi: number;
    /** Pages the product prints (sides). */
    pages?: number;
}
export interface PreflightResult {
    ok: boolean;
    issues: PreflightIssue[];
    /** Print size ÷ design size (1 when the design already has the product size). */
    scale: number;
    /** Trim size the design maps onto (orientation matched). */
    trim: {
        w: number;
        h: number;
    };
    counts: {
        error: number;
        warning: number;
        info: number;
    };
}
/** Generic print target for a design on its own (Download → PDF Print): its own size, 3 mm bleed, 5 mm safe zone. */
export declare function standalonePrintTarget(doc: Pick<DesignDoc, 'meta'>): PreflightTarget;
/** Rough check: does this sRGB colour fall outside a typical coated CMYK (FOGRA39/GRACoL) gamut? */
export declare function cmykGamutRisk(c: ColorValue): boolean;
/** Axis-aligned bounds of a (possibly rotated) element box, page px. */
export declare function elementBounds(xf: Xf): {
    x0: number;
    y0: number;
    x1: number;
    y1: number;
};
/**
 * Print preflight. Checks, per page: size against the product (orientation-aware), page count, effective image
 * resolution at print size, text inside the safe zone, artwork that reaches the trim edge without covering the
 * bleed, colours likely outside the CMYK gamut, hairlines thinner than 0.25 pt, fonts that cannot be embedded,
 * and elements that cannot print (video, embeds, forms).
 */
export declare function preflightPrint(doc: DesignDoc, target: PreflightTarget, opts?: {
    availableFonts?: {
        has(family: string): boolean;
    };
    /**
     * Optional out-of-gamut test that replaces the `cmykGamutRisk` heuristic (e.g. a colour-managed check against
     * the chosen output profile). Return `undefined` to fall back to the heuristic for that colour.
     */
    outOfGamut?: (c: ColorValue) => boolean | undefined;
}): PreflightResult;
/** Bleed in design px for a product (for the export job). */
export declare const bleedPxFor: (bleedMm: number) => number;
/**
 * Unique colours that can fall outside a CMYK gamut (sRGB / Display-P3, not transparent), in the order preflight
 * meets them: page backgrounds, then visible elements. Capped at `max` (for a colour-managed gamut check).
 */
export declare function printColors(doc: DesignDoc, max?: number): ColorValue[];
/** Identity of a colour for gamut lookups (space + rounded components, alpha ignored). */
export declare function printColorKey(c: ColorValue): string;
