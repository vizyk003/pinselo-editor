// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Core value types shared by every element. See docs/plan/03-document-model-and-renderer.md §2.1. */
export type Id<T extends string> = string & {
    readonly __brand: T;
};
export type DesignId = Id<'design'>;
export type PageId = Id<'page'>;
export type ElementId = Id<'el'>;
export type AssetId = Id<'asset'>;
export type Vec2 = readonly [number, number];
export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}
/** Affine [a,b,c,d,e,f]: x' = a x + c y + e; y' = b x + d y + f. */
export type Mat2D = readonly [number, number, number, number, number, number];
/** Canonical element transform. Written atomically to the CRDT (one value). */
export interface Xf {
    /** Top-left of the unrotated box, in parent (page) space, document px at 96 dpi. */
    x: number;
    y: number;
    w: number;
    h: number;
    /** Degrees, normalized to (-180, 180]. Rotation is about the box centre. */
    rot: number;
    flipX?: boolean;
    flipY?: boolean;
}
export type Unit = 'px' | 'mm' | 'cm' | 'in' | 'pt';
/** Authored colour. Only sRGB is produced by the MVP editor; other spaces are preserved verbatim. */
export type ColorValue = {
    space: 'srgb' | 'display-p3';
    r: number;
    g: number;
    b: number;
    a: number;
} | {
    space: 'cmyk';
    c: number;
    m: number;
    y: number;
    k: number;
    a: number;
} | {
    space: 'spot';
    swatch: string;
    tint: number;
    a: number;
};
export interface GradientStop {
    t: number;
    color: ColorValue;
}
export type Paint = {
    k: 'solid';
    color: ColorValue;
} | {
    k: 'linear';
    stops: GradientStop[];
    angle: number;
} | {
    k: 'radial';
    stops: GradientStop[];
}
/**
 * `fit` scales the whole image into the box (`fill` = cover, centred). `crop` (optional, normalized source rect
 * like ImageEl.crop) pins the part of the image shown in the box and wins over `fit`: a photo dropped onto a
 * shape is stored this way so it can be repositioned inside the outline.
 */
 | {
    k: 'image';
    asset: AssetId;
    fit: 'fill' | 'fit' | 'tile';
    crop?: Rect;
};
export interface Stroke {
    paint: Paint;
    width: number;
    align: 'center' | 'inside' | 'outside';
    cap: 'butt' | 'round' | 'square';
    join: 'miter' | 'round' | 'bevel';
    dash?: number[];
}
export type BlendMode = 'normal' | 'multiply' | 'screen' | 'overlay' | 'darken' | 'lighten' | 'color-dodge' | 'color-burn' | 'hard-light' | 'soft-light' | 'difference' | 'exclusion' | 'hue' | 'saturation' | 'color' | 'luminosity';
export type LinkTarget = {
    kind: 'url';
    url: string;
} | {
    kind: 'page';
    pageId: PageId;
};
