// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
/**
 * Design-wide styles (A17, also used by brand kits): list the colours a design uses, replace colours
 * everywhere, apply a palette (by luminance rank, so light/dark structure and contrast survive) and apply
 * a font pair (by role, else by size rank). Colour mapping is a deep walk over every element's data, so
 * new element types are covered automatically.
 */
import type { DocOp } from '../crdt';
import { type DesignDoc } from '../schema';
/** Distinct colours used by the design, most used first. */
export declare function designColors(doc: DesignDoc): Array<{
    hex: string;
    count: number;
}>;
/** Ops replacing colours (hex → hex) everywhere; alpha is preserved. */
export declare function recolorOps(doc: DesignDoc, map: Record<string, string>): DocOp[];
/** Map the design's colours onto a palette by luminance rank (darkest → darkest, lightest → lightest). */
export declare function paletteMap(doc: DesignDoc, palette: string[]): Record<string, string>;
export interface FontSet {
    heading: string;
    subheading?: string;
    body: string;
}
/** Ops applying a font pair: headline/subhead roles, else the largest text gets the heading font. */
export declare function fontSetOps(doc: DesignDoc, set: FontSet): DocOp[];
