// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { ColorValue, Paint } from './primitives';
export type Rgba = {
    r: number;
    g: number;
    b: number;
    a: number;
};
export declare function rgb(r: number, g: number, b: number, a?: number): ColorValue;
/** '#rgb', '#rgba', '#rrggbb', '#rrggbbaa' → ColorValue (sRGB). */
export declare function hex(s: string): ColorValue;
/** Convert any authored colour to display sRGB (naive for CMYK/spot in the MVP; LittleCMS later, ADR-0037). */
export declare function toRgba(c: ColorValue): Rgba;
export declare function toHex(c: ColorValue, withAlpha?: boolean): string;
export declare function toCss(c: ColorValue): string;
export declare function solid(c: ColorValue | string): Paint;
/** The representative colour of a paint (first stop for gradients); used by swatches and pickers. */
export declare function paintColor(p: Paint | undefined): ColorValue | undefined;
/** WCAG relative luminance and contrast ratio. */
export declare function luminance(c: ColorValue): number;
export declare function contrastRatio(a: ColorValue, b: ColorValue): number;
