// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
import { type DesignDoc, type Element, type PageId } from '../schema';
export type A11yIssueCode = 'alt-missing' | 'low-contrast' | 'small-text' | 'link-name' | 'no-language' | 'empty-text';
export interface A11yIssue {
    code: A11yIssueCode;
    severity: 'error' | 'warning';
    pageId?: PageId;
    id?: Element['id'];
    /** Code-specific data: contrast ratio, font size, … */
    data?: Record<string, number | string>;
}
/** Smallest legible size, in design px, for the design's dimensions (≈ 12 px on a 1080-px social post, 18 px on a 1920 slide). */
export declare function minTextPx(doc: DesignDoc): number;
export declare function auditDesign(doc: DesignDoc): A11yIssue[];
export type ElementKindLabel = 'Text' | 'Image' | 'Video' | 'Shape' | 'Line' | 'Group' | 'Frame' | 'Table' | 'Chart' | 'Photo grid' | 'Form' | 'QR code' | 'Drawing' | 'Embed' | 'Element';
/** Plain-language description parts (the UI translates `kind`). */
export declare function describeElement(doc: DesignDoc, el: Element): {
    kind: ElementKindLabel;
    label: string;
    x: number;
    y: number;
    w: number;
    h: number;
    rot: number;
    locked: boolean;
};
