// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/**
 * Text model, snapshot form. In the CRDT the body lives in a Y.XmlFragment
 * (paragraph elements holding Y.XmlText with marks), see @oc/crdt.
 * docs/plan/03-document-model-and-renderer.md §2.4.
 */
import type { ColorValue, LinkTarget, Paint } from './primitives';
export interface RunStyle {
    family: string;
    weight: number;
    italic: boolean;
    /** px */
    size: number;
    fill: Paint;
    /** em/1000 */
    tracking: number;
    underline?: boolean;
    strike?: boolean;
    caseTransform?: 'none' | 'upper' | 'lower' | 'title';
    highlight?: ColorValue;
    /** Superscript or subscript: drawn smaller and raised or lowered (crates/oc-schema text.rs BASELINE_SCALE, baseline_offset). */
    baseline?: 'super' | 'sub';
}
export interface Run {
    text: string;
    style: Partial<RunStyle>;
    link?: LinkTarget;
}
export interface Paragraph {
    runs: Run[];
    align: 'start' | 'center' | 'end' | 'justify';
    /** multiple of font size */
    lineHeight: number;
    spaceAfter: number;
    list?: {
        kind: 'bullet' | 'number';
        level: number;
    };
}
export interface TextBody {
    paragraphs: Paragraph[];
    defaultRun: RunStyle;
}
export type TextEffect = {
    k: 'shadow';
    dx: number;
    dy: number;
    blur: number;
    color: ColorValue;
} | {
    k: 'hollow';
    width: number;
} | {
    k: 'outline';
    width: number;
    color: ColorValue;
} | {
    k: 'background';
    color: ColorValue;
    padding: number;
    radius: number;
} | {
    k: 'neon';
    intensity: number;
}
/** Three fading copies behind the text along (dx, dy). */
 | {
    k: 'echo';
    dx: number;
    dy: number;
}
/** Chromatic split: cyan and magenta copies offset sideways (% of font size). */
 | {
    k: 'glitch';
    offset: number;
};
/** Paragraph attributes that live on the paragraph node (not on runs). */
export type ParagraphAttrs = Omit<Paragraph, 'runs'>;
export declare const DEFAULT_PARAGRAPH: ParagraphAttrs;
export declare function bodyPlainText(body: TextBody): string;
export declare function resolveRunStyle(body: TextBody, run: Run): RunStyle;
