// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { DesignDoc } from './design';
import type { ColorValue, ElementId, PageId, Unit, Xf } from './primitives';
export type PrintCategory = 'business-cards' | 'flyers' | 'posters' | 'invitations' | 'postcards' | 'stickers' | 'apparel' | 'drinkware' | 'bags';
/** A trim size (or, for apparel/mugs/bags, the print area). Stored in px @ 96 dpi like design dims. */
export interface PrintSize {
    id: string;
    label: string;
    w: number;
    h: number;
    unit: Unit;
}
export interface PrintChoice {
    id: string;
    label: string;
    /** Added per unit (minor units). */
    perUnit?: number;
    /** Multiplies the base price (e.g. 1.25 for double-sided). */
    multiplier?: number;
    /** Pages the design needs for this choice (sides). */
    pages?: number;
}
export interface PrintOptionGroup {
    /** 'paper' | 'sides' | 'finish' | 'envelopes' | 'shape' | 'material' | 'garment' | 'color' | … */
    id: string;
    label: string;
    choices: PrintChoice[];
    /** Default choice id (first choice when omitted). */
    default?: string;
}
/** Unit price tiers for one size: the tier with the largest `min` ≤ quantity applies. */
export interface PriceTier {
    min: number;
    unit: number;
}
export interface GelatoMapping {
    /** Product UID template, e.g. `cards_pf_{size}_pt_{paper}_cl_{sides}_{orientation}`; tokens come from `tokens`. */
    productUid: string;
    /** token name → (choice id or size id → Gelato value). `{orientation}` is filled with `hor` / `ver`. */
    tokens: Record<string, Record<string, string>>;
}
export interface PrintfulMapping {
    /** Key template over size and option ids, e.g. `{size}` or `{size}|{garment}|{color}`. */
    variantKey: string;
    /** Catalog variant ids (Printful API v2 `catalog_variant_id`). */
    variants: Record<string, number>;
    /** Placement and technique for API v2 `placements` (e.g. `front` + `dtg`, `default` + `digital`). */
    placement: string;
    technique: string;
}
export interface PrintProduct {
    id: string;
    category: PrintCategory;
    name: string;
    description: string;
    sizes: PrintSize[];
    /** Bleed added around the trim (0 for print-area products such as t-shirts). */
    bleedMm: number;
    /** Keep important content this far inside the trim. */
    safeMm: number;
    /** Designs in the other orientation are accepted (cards, flyers, posters). */
    rotatable: boolean;
    options: PrintOptionGroup[];
    /** Suggested quantities in the picker. */
    quantities: number[];
    minQuantity: number;
    maxQuantity: number;
    /** Unit price tiers per size id ('*' = every size). */
    prices: Record<string, PriceTier[]>;
    /** Shipping weight per unit, grams. */
    weightG: number;
    /** Effective resolution below which images print visibly soft (warning), and the recommended resolution (note). */
    minDpi: number;
    goodDpi: number;
    /** Pages the design must have when no `sides` option says otherwise. */
    pages: number;
    /** Notes shown in the picker (e.g. die-cut limitation). */
    note?: string;
    fulfilment?: {
        gelato?: GelatoMapping;
        printful?: PrintfulMapping;
    };
}
export interface ShippingMethod {
    id: string;
    label: string;
    /** Delivery estimate in business days after production. */
    days: [number, number];
    base: {
        domestic: number;
        international: number;
    };
    /** Per started kilogram. */
    perKg: {
        domestic: number;
        international: number;
    };
}
export interface PrintCatalog {
    version: number;
    /** ISO 4217, lower case. Falls back to the billing currency when empty. */
    currency: string;
    products: PrintProduct[];
    shipping: {
        /** Country the orders ship from (decides domestic vs international rates). */
        origin: string;
        methods: ShippingMethod[];
        /** ISO 3166-1 alpha-2 codes we deliver to. */
        countries: string[];
    };
    tax: {
        /** Percent by `CC` or `CC-STATE` (state wins). Estimate only; Stripe Tax decides when enabled. */
        rates: Record<string, number>;
        shippingTaxable: boolean;
    };
}
export declare const DEFAULT_PRINT_CATALOG: PrintCatalog;
/** Validate the shape of an operator catalogue (thrown message names the first problem). */
export declare function checkPrintCatalog(c: PrintCatalog): PrintCatalog;
export interface PrintSelection {
    productId: string;
    sizeId: string;
    /** option group id → choice id (defaults fill the gaps). */
    options: Record<string, string>;
    quantity: number;
}
export declare function findProduct(catalog: PrintCatalog, id: string): PrintProduct | undefined;
/** Selected choice per option group, defaults applied; unknown choices are an error. */
export declare function resolveOptions(p: PrintProduct, picked: Record<string, string>): Record<string, PrintChoice>;
/** Pages the design needs for this selection. */
export declare function expectedPages(p: PrintProduct, picked: Record<string, string>): number;
export declare class PrintError extends Error {
    readonly code: string;
    constructor(code: string, message: string);
}
export interface PriceLine {
    kind: 'product' | 'option' | 'shipping' | 'tax';
    label: string;
    amount: number;
}
export interface PriceBreakdown {
    currency: string;
    unitPrice: number;
    subtotal: number;
    shipping: number;
    tax: number;
    taxRate: number;
    total: number;
    weightG: number;
    lines: PriceLine[];
    shippingMethod: string;
    deliveryDays: [number, number] | null;
}
export declare function unitPriceFor(p: PrintProduct, sizeId: string, quantity: number): number;
/**
 * Price a selection. Product subtotal = quantity × tier unit price × option multipliers + quantity × per-unit
 * option surcharges. Shipping = method base + per started kg; tax = rate for the destination on the subtotal
 * (plus shipping when taxable). Without a destination, shipping uses the domestic rate and tax is 0.
 */
export declare function quotePrint(catalog: PrintCatalog, sel: PrintSelection, dest?: {
    country?: string;
    state?: string;
}, shippingMethod?: string, currency?: string): PriceBreakdown;
/** Fill a `{token}` template from the selection (size id, option choice ids, orientation). */
export declare function fillTemplate(tpl: string, vars: Record<string, string>, tokens?: Record<string, Record<string, string>>): string;
export interface ShippingAddress {
    name: string;
    company?: string;
    line1: string;
    line2?: string;
    city: string;
    state?: string;
    postalCode: string;
    /** ISO 3166-1 alpha-2. */
    country: string;
    phone?: string;
    email?: string;
}
export type AddressField = keyof ShippingAddress;
export type AddressProblem = 'required' | 'invalid' | 'unsupported' | 'too_long';
/** Normalize and check an address. Postcode and state formats are checked for the common shipping countries. */
export declare function validateAddress(a: Partial<ShippingAddress>, countries?: readonly string[]): {
    ok: boolean;
    address: ShippingAddress;
    errors: Partial<Record<AddressField, AddressProblem>>;
};
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
export declare function preflightTargetFor(p: PrintProduct, sizeId: string, options?: Record<string, string>): PreflightTarget;
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
