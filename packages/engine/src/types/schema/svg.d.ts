// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/**
 * SVG helpers shared by the API (sanitising uploads and library graphics), the renderers (recoloured variants) and the editor
 * (colour swatches for a selected SVG). Pure string functions: no DOM, so they run in Node, workers and the browser alike.
 *
 * Recolouring is non-destructive: an image element stores `svgColors` (original → replacement) and every renderer asks for the
 * derived asset key `recolorKey(asset, svgColors)` ("<asset>/recolor/<pairs>"), which hosts rasterise from the stored SVG source.
 */
import type { AssetId } from './primitives';
/**
 * Strip active content and external references from an SVG (defence in depth: resvg runs no scripts and fetches nothing,
 * but sanitised sources are also served to browsers). Removes scripts, foreign content, event handlers, DOCTYPE/entities,
 * external hrefs (only `#fragment` and raster `data:` URIs survive), external `url(...)` references and CSS `@import`.
 */
export declare function sanitizeSvg(svg: string): string;
/** The colour key for `currentColor` (icon sets inherit their colour through it). */
export declare const CURRENT_COLOR = "currentColor";
/**
 * Canonical form of an SVG paint value: `#rrggbb`, `#rrggbbaa` (alpha < 1 only) or `currentColor`; `null` for anything that is
 * not a plain colour (`none`, `url(#…)`, `transparent`, `inherit`, unknown names).
 */
export declare function normalizeSvgColor(raw: string): string | null;
export interface SvgColorUse {
    color: string;
    uses: number;
}
/** Distinct colours an SVG paints with (fills, strokes, gradient stops, `currentColor`), most used first. */
export declare function extractSvgColors(svg: string): SvgColorUse[];
/** Keep only well-formed entries (canonical keys, hex replacements) that actually change something. */
export declare function cleanSvgColorMap(map: Record<string, string> | undefined | null): Record<string, string>;
/** Rewrite every occurrence of each mapped colour (the asset bytes stay unchanged; this produces a derived SVG). */
export declare function recolorSvg(svg: string, map: Record<string, string>): string;
export declare const MAX_SVG_COLORS = 64;
/** Asset key of the recoloured variant of an SVG asset; the asset itself when nothing is remapped. */
export declare function recolorKey(asset: AssetId, map: Record<string, string> | undefined | null): AssetId;
/** Inverse of recolorKey; null for anything else (strict: keys come from URLs). */
export declare function parseRecolorKey(key: string): {
    asset: AssetId;
    map: Record<string, string>;
} | null;
/** Validation patterns for ImageEl.svgColors. */
export declare const SVG_COLOR_KEY: RegExp;
export declare const SVG_COLOR_VALUE: RegExp;
