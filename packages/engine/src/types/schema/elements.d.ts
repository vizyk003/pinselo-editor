// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/** Element types. docs/plan/03-document-model-and-renderer.md §2.3, §2.5. MVP subset; unknown types are preserved. */
import type { AssetId, BlendMode, ColorValue, ElementId, LinkTarget, PageId, Paint, Rect, Stroke, Vec2, Xf } from './primitives';
import type { TextBody, TextEffect } from './text';
export type ElementType = 'text' | 'shape' | 'line' | 'image' | 'frame' | 'group' | 'qr' | 'draw' | 'table' | 'chart' | 'video' | 'grid' | 'form' | 'embed';
export declare const ELEMENT_TYPES: readonly ElementType[];
export type SemanticRole = 'headline' | 'subhead' | 'body' | 'quote' | 'caption' | 'cta' | 'logo' | 'hero-image' | 'background' | 'decor' | 'price' | 'legal';
/**
 * Element lock. `by: 'user'` locks are a convenience anyone with edit access can toggle. `by: 'brand'` locks are
 * left in old documents and hold nobody.
 */
export interface LockMask {
    move?: boolean;
    resize?: boolean;
    rotate?: boolean;
    content?: boolean;
    style?: boolean;
    delete?: boolean;
    by: 'user' | 'brand';
}
export type Effect = {
    k: 'dropShadow';
    dx: number;
    dy: number;
    blur: number;
    color: ColorValue;
} | {
    k: 'blur';
    radius: number;
} | {
    k: 'outline';
    width: number;
    color: ColorValue;
};
export interface ImageAdjust {
    brightness?: number;
    contrast?: number;
    saturation?: number;
    temperature?: number;
    tint?: number;
    blur?: number;
    vignette?: number;
    /** Tone (A17), -100..100: highlights/shadows recover or push the bright/dark ranges; whites/blacks move the end points. */
    highlights?: number;
    shadows?: number;
    whites?: number;
    blacks?: number;
    /** Detail (A17): sharpen 0..100 (fine edges), clarity -100..100 (local contrast), fade 0..100 (matte lift). */
    sharpen?: number;
    clarity?: number;
    fade?: number;
    /** Pixelate 0..100 (redaction): blocks of about 6% of the longer side per 100 (crates/oc-render color_matrix). */
    pixelate?: number;
}
export type ImageFilterPreset = 'none' | 'mono' | 'sepia' | 'vivid' | 'fade' | 'cool' | 'warm' | 'noir' | 'retro' | 'chrome' | 'dramatic' | 'vintage' | 'summer' | 'dusk' | 'lomo' | 'crisp' | 'pastel' | 'matte' | 'teal-orange' | 'blush' | 'forest' | 'silver' | 'contrast' | 'selenium' | 'cyanotype' | 'invert' | 'infrared' | 'golden' | 'arctic' | 'rose' | 'mint' | 'lavender' | 'sunset' | 'duo' | 'neon';
export declare const FILTER_PRESETS: readonly ImageFilterPreset[];
/** Duotone (IM-012): luminance mapped from `shadow` to `highlight`, blended by `amount` (0..1). */
export interface Duotone {
    shadow: ColorValue;
    highlight: ColorValue;
    amount: number;
}
export interface ImageEditStack {
    adjust?: ImageAdjust;
    filter?: {
        preset: ImageFilterPreset;
        amount: number;
    };
    duotone?: Duotone;
    version: 1;
}
/** Entrance animation (MV-002). Evaluated deterministically from (element, page time) — see @oc/render-core motion. */
export type AnimPreset = 'fade' | 'rise' | 'drop' | 'pan' | 'pop' | 'zoom' | 'wipe' | 'blur' | 'tumble' | 'breathe' | 'slide' | 'flip' | 'stomp' | 'spin' | 'bounce' | 'float' | 'shrink' | 'typewriter' | 'draw';
export declare const ANIM_PRESETS: readonly AnimPreset[];
export type AnimDirection = 'up' | 'down' | 'left' | 'right';
/** How an entrance moves over its duration (default: ease-out). */
export type AnimEasing = 'linear' | 'ease' | 'ease-in' | 'ease-out' | 'ease-in-out';
/** Text can animate as a whole or one line (paragraph), word or letter at a time. */
export type AnimUnit = 'element' | 'line' | 'word' | 'char';
export interface ElementAnim {
    preset: AnimPreset;
    durationMs: number;
    delayMs: number;
    /** Directional presets (slide, pan, wipe, rise/drop override). */
    direction?: AnimDirection;
    /** 0.25..2 — scales travel, rotation and scale amounts. */
    intensity?: number;
    /** Text only: animate by line/word/letter with `staggerMs` between units. */
    by?: AnimUnit;
    staggerMs?: number;
    /** Presentations (PS-010): appear on the Nth click instead of automatically (1-based build step). */
    onClick?: number;
    easing?: AnimEasing;
    /** Travel of moving presets (rise, drop, pan, slide, float, bounce) in px; default: from the element size. */
    distance?: number;
}
/** Keyframes (MV-100): animate any of these properties over page time, as offsets from the element's own values. */
export type KeyframeProp = 'x' | 'y' | 'scale' | 'rotate' | 'opacity' | 'blur';
export declare const KEYFRAME_PROPS: readonly KeyframeProp[];
export type KeyframeEase = 'linear' | 'ease' | 'ease-in' | 'ease-out' | 'back' | 'hold';
/** `t` = page time (ms); `ease` shapes the segment that ENDS at this key. x/y px, scale ×, rotate °, opacity 0..1 ×, blur px. */
export interface Keyframe {
    t: number;
    v: number;
    ease?: KeyframeEase;
}
export type Keyframes = Partial<Record<KeyframeProp, Keyframe[]>>;
/** Looping emphasis animation (MV-004) while the element is on screen, after its entrance. */
export type EmphasisPreset = 'pulse' | 'wiggle' | 'bounce' | 'flicker' | 'spin' | 'heartbeat' | 'float' | 'swing';
export declare const EMPHASIS_PRESETS: readonly EmphasisPreset[];
export interface ElementEmphasis {
    preset: EmphasisPreset;
    periodMs: number;
    intensity?: number;
}
/** Exit animation: plays at the end of the element's visible window (or the page). */
export interface ElementAnimOut {
    preset: AnimPreset;
    durationMs: number;
}
/** Motion path (MV-011): the element travels along a polyline of offsets (box px) relative to its position. */
export interface MotionPath {
    points: number[];
    durationMs: number;
    delayMs: number;
    easing: 'linear' | 'ease';
}
/** When an element is visible on its page (page time, ms). Omitted = whole page. */
export interface ElementTiming {
    startMs: number;
    endMs?: number;
}
export interface DesignOrigin {
    channel: 'editor' | 'template' | 'ai' | 'api' | 'import';
    templateId?: string;
}
export interface ElementBase {
    id: ElementId;
    type: ElementType;
    /** Flat map + parent pointer. Order within a parent = sort by (z, id). */
    parentId: PageId | ElementId;
    z: string;
    xf: Xf;
    opacity: number;
    blend?: BlendMode;
    visible?: boolean;
    lock?: LockMask;
    name?: string;
    role?: SemanticRole;
    link?: LinkTarget;
    altText?: string;
    decorative?: boolean;
    effects?: Effect[];
    anim?: ElementAnim;
    emphasis?: ElementEmphasis;
    keyframes?: Keyframes;
    animOut?: ElementAnimOut;
    motionPath?: MotionPath;
    timing?: ElementTiming;
    origin?: DesignOrigin;
    /** Bulk create (ED-090): the data column that fills this element (text, or image URL / upload name). */
    dataBinding?: {
        column: string;
    };
    /** Board metadata of older documents (sticky notes, whiteboard sections and mind-map nodes). Stored in `props`. */
    board?: BoardMeta;
    /** Tombstone. Deleted elements stay in the map so undo can restore concurrent edits. */
    deleted?: {
        at: number;
        by: string;
    };
    /** Fields written by a newer client; preserved verbatim. */
    ext?: Record<string, unknown>;
}
/**
 * Board metadata. `sticky` marks a sticky-note group (shape + text) with its colour and author;
 * `section` marks a rect shape as a titled section whose members are the top-level elements inside its box;
 * `mind` marks a mind-map node (`parent` null/absent = root).
 */
export interface BoardMeta {
    sticky?: {
        color: string;
        author?: {
            id: string;
            name: string;
        };
    };
    section?: {
        title: string;
        /** Position in the sections navigator (ascending). */
        order: number;
        /** Collapsed: the expanded height and the members hidden while collapsed. */
        collapsed?: {
            h: number;
            hidden: ElementId[];
        };
    };
    mind?: {
        parent?: ElementId | null;
    };
}
export type ShapeKind = 'rect' | 'ellipse' | 'triangle' | 'polygon' | 'star' | 'arrow' | 'heart' | 'diamond' | 'hexagon' | 'speech' | 'cross' | 'chevron' | 'octagon' | 'pentagon' | 'parallelogram' | 'trapezoid' | 'right-triangle' | 'ring' | 'half-circle' | 'moon' | 'lightning' | 'cloud' | 'seal' | 'tag' | 'ribbon' | 'double-arrow' | 'plus' | 'blob' | 'arch' | 'drop' | 'burst' | 'flag' | 'squircle' | 'pill' | 'snip-corner' | 'snip-two' | 'round-one' | 'folded-corner' | 'plaque' | 'frame' | 'ticket' | 'shield' | 'cylinder' | 'cube' | 'block-arc' | 'pie' | 'star-6' | 'star-8' | 'starburst-12' | 'sparkle' | 'badge' | 'gear' | 'flower' | 'sun' | 'leaf' | 'cross-x' | 'check' | 'chevron-double' | 'curved-arrow' | 'u-turn-arrow' | 'circular-arrow' | 'banner' | 'wave' | 'callout-rect' | 'callout-round' | 'thought-bubble'
/** Flowchart symbols. */
 | 'document' | 'multi-document' | 'predefined-process' | 'manual-input' | 'delay' | 'off-page' | 'internal-storage' | 'display'
/** Free-form outline from `d` (imports: PowerPoint custom geometry, SVG, PDF). */
 | 'custom';
/** Order drives the Elements panel grid: basics, rect variants, polygons, stars & badges, symbols, arrows, banners, callouts. */
export declare const SHAPE_KINDS: readonly ShapeKind[];
export interface ShapeEl extends ElementBase {
    type: 'shape';
    kind: ShapeKind;
    /** cornerRadius, sides, points, innerRatio, … */
    params: Record<string, number>;
    /** For kind 'custom': absolute SVG path (M L C Q Z) in a unit box, x and y in 0..1. */
    d?: string;
    fills: Paint[];
    strokes: Stroke[];
}
export type LineMarker = 'none' | 'arrow' | 'triangle' | 'circle' | 'square' | 'bar' | 'stealth' | 'diamond' | 'triangle-outline' | 'circle-outline' | 'square-outline' | 'diamond-outline';
export declare const LINE_MARKERS: readonly LineMarker[];
/** Where a connector end is glued (WB-010): a side midpoint or the centre of another element. */
export type AnchorSide = 'top' | 'right' | 'bottom' | 'left' | 'center';
export interface LineAttach {
    id: ElementId;
    anchor: AnchorSide;
}
export interface LineEl extends ElementBase {
    type: 'line';
    /** Endpoints in element space (0..w, 0..h of xf). For connectors these are recomputed from the attachments. */
    points: Vec2[];
    stroke: Stroke;
    startMarker: LineMarker;
    endMarker: LineMarker;
    /** Connector glue: when set, that end follows the attached element. */
    startAttach?: LineAttach;
    endAttach?: LineAttach;
    /** Connector routing (default straight). */
    route?: 'straight' | 'elbow' | 'curved';
}
export interface ImageEl extends ElementBase {
    type: 'image';
    asset: AssetId;
    /** Normalized source crop (0..1). */
    crop: Rect;
    edits?: ImageEditStack;
    cornerRadius?: number;
    /** SVG assets only: non-destructive recolouring, original colour (`#rrggbb[aa]` or `currentColor`) → replacement `#rrggbb[aa]`. */
    svgColors?: Record<string, string>;
}
export type FrameClip = 'rect' | 'ellipse' | 'triangle' | 'star' | 'heart' | 'hexagon' | 'diamond' | 'rounded';
export interface FrameEl extends ElementBase {
    type: 'frame';
    /** Built-in outline; also the fallback for clients that do not understand `clipPath`. */
    clip: FrameClip;
    /** Library outline (A13): unit-box SVG path (absolute M/L/C/Q/Z, x and y in 0..1); overrides `clip` when set. */
    clipPath?: string;
    /** Decorative chrome drawn above the content (device bezels, polaroid borders, film perforations). */
    overlay?: FrameOverlay[];
    /** Frame library id the frame was made from (UI only). */
    preset?: string;
    /** Photo or video. `video` set = the asset is a video played with these clip settings. */
    content?: {
        asset: AssetId;
        crop: Rect;
        edits?: ImageEditStack;
        video?: MediaClip;
    };
    placeholderFill: Paint;
}
/** Unit-box path (0..1) filled above a frame's content; holes are counter-wound (nonzero fill). */
export interface FrameOverlay {
    d: string;
    fill: Paint;
}
export interface GroupEl extends ElementBase {
    type: 'group';
}
export interface TextEl extends ElementBase {
    type: 'text';
    body: TextBody;
    frame: {
        mode: 'autoHeight' | 'fixed';
    };
    autoFit: 'none' | 'shrink';
    curve?: number;
    effect?: TextEffect;
    /** Vertical anchor of the text in a fixed-height box (default top). */
    valign?: 'top' | 'middle' | 'bottom';
}
export interface QrEl extends ElementBase {
    type: 'qr';
    data: string;
    ecc: 'L' | 'M' | 'Q' | 'H';
    fg: ColorValue;
    bg: ColorValue;
}
/** Freehand stroke (PT-010). Points are [x, y, pressure] triples in the element box at its creation size (`base`). */
export type Brush = 'pen' | 'marker' | 'highlighter';
export interface DrawEl extends ElementBase {
    type: 'draw';
    points: number[];
    base: {
        w: number;
        h: number;
    };
    brush: Brush;
    color: ColorValue;
    size: number;
}
/** Table (ED-028). Track sizes are fractions of the box; cells are sparse, keyed "r:c". */
/** One side of a cell border; width 0 hides it. */
export interface CellBorder {
    width: number;
    color: ColorValue;
}
export interface TableCell {
    /** Plain-text mirror of the cell (always kept in sync; legacy cells only have this). */
    text: string;
    /**
     * Rich content (A12). When present it is authoritative and `text` is its plain mirror. The body's
     * `defaultRun` is re-derived from the table style when read (see `cellBody`), so table-wide font changes apply.
     */
    body?: TextBody;
    fill?: Paint;
    bold?: boolean;
    align?: 'start' | 'center' | 'end';
    valign?: 'top' | 'middle' | 'bottom';
    /** Inner padding in px (default: the table's `style.padding`). */
    padding?: number;
    /** Per-side borders; a missing side uses the table border. */
    borders?: {
        t?: CellBorder;
        r?: CellBorder;
        b?: CellBorder;
        l?: CellBorder;
    };
    /** Merged cell anchor: covers `rows` × `cols` cells from here (covered cells are hidden). */
    span?: {
        rows: number;
        cols: number;
    };
}
export interface TableEl extends ElementBase {
    type: 'table';
    rows: number[];
    cols: number[];
    cells: Record<string, TableCell>;
    style: {
        family: string;
        size: number;
        color: ColorValue;
        borderColor: ColorValue;
        borderWidth: number;
        headerFill?: Paint;
        headerColor?: ColorValue;
        stripeFill?: Paint;
        padding: number;
        /** Header row text is bold (table styles); per-cell `bold` still applies. */
        headerBold?: boolean;
        /** Last applied table style preset (UI highlight only). */
        preset?: string;
    };
}
/** Static chart (PR-060). Data lives inline; layout is computed deterministically by render-core. */
export type ChartKind = 'bar' | 'column' | 'line' | 'area' | 'pie' | 'donut' | 'stacked-column' | 'stacked-bar' | 'combo' | 'scatter' | 'bubble' | 'radar' | 'funnel' | 'progress-ring' | 'progress-bar' | 'gauge';
export declare const CHART_KINDS: readonly ChartKind[];
/** Value labels: prefix/suffix (currency, %, units), decimals and thousands separators. */
export interface ChartFormat {
    prefix?: string;
    suffix?: string;
    decimals?: number;
    thousands?: boolean;
}
export interface ChartSeries {
    name: string;
    values: number[];
    color: ColorValue;
}
export interface ChartEl extends ElementBase {
    type: 'chart';
    kind: ChartKind;
    labels: string[];
    series: ChartSeries[];
    style: {
        family: string;
        textColor: ColorValue;
        gridColor: ColorValue;
        showLegend: boolean;
        showValues: boolean;
        showGrid: boolean;
        fontSize: number;
        format?: ChartFormat; /** progress/gauge maximum (default 100) */
        max?: number; /** pie/donut/funnel: colour per slice */
        sliceColors?: ColorValue[];
    };
}
/** Media clip (MV-020): a window of a source file played at `speed`. Source times in ms. */
export interface MediaClip {
    trimStartMs: number;
    trimEndMs: number;
    speed: number;
    volume: number;
    muted: boolean;
    loop: boolean;
}
/** Video on the page. Plays from `timing.startMs` (default 0) in page time. */
export interface VideoEl extends ElementBase {
    type: 'video';
    asset: AssetId;
    clip: MediaClip;
    crop: Rect;
    cornerRadius?: number;
    /** Filters/adjustments applied to every frame (A14), same stack as photos. */
    edits?: ImageEditStack;
}
/**
 * Photo grid (IM-030): a layout of cells (normalized rects of the box, before the gap is applied).
 * Each cell is an image placeholder; dropping an image fills it with a cover crop.
 */
/** `video` set = the cell holds a video asset played with these clip settings. */
export interface GridCell {
    rect: Rect;
    asset?: AssetId;
    crop?: Rect;
    edits?: ImageEditStack;
    video?: MediaClip;
}
export interface GridEl extends ElementBase {
    type: 'grid';
    cells: GridCell[];
    /** Spacing between cells and at the outer edge, in px. */
    gap: number;
    radius: number;
    placeholderFill: Paint;
}
/** Forms (FM-001): collect responses on published websites and public views. */
export type FormFieldKind = 'text' | 'longtext' | 'email' | 'number' | 'date' | 'choice' | 'checkboxes';
export interface FormField {
    id: string;
    kind: FormFieldKind;
    label: string;
    required?: boolean;
    options?: string[];
    placeholder?: string;
}
export interface FormEl extends ElementBase {
    type: 'form';
    title: string;
    fields: FormField[];
    submitLabel: string;
    thanks: string;
    style: {
        family: string;
        size: number;
        color: ColorValue;
        accent: ColorValue;
        fieldFill: ColorValue;
        borderColor: ColorValue;
        radius: number;
    };
}
/**
 * Embedded media (A11): YouTube, Vimeo, Spotify, SoundCloud, Google Maps, Loom, Figma, CodePen.
 * On the canvas and in static exports it is a poster with a play badge (and a link to `url`); Present
 * mode, view links and published websites play it live in an iframe of `src` (allow-listed hosts only).
 */
export interface EmbedEl extends ElementBase {
    type: 'embed';
    url: string;
    provider: EmbedProvider;
    src: string;
    title: string;
    poster?: AssetId;
}
export type EmbedProvider = 'youtube' | 'vimeo' | 'spotify' | 'soundcloud' | 'maps' | 'loom' | 'figma' | 'codepen';
export type Element = ShapeEl | LineEl | ImageEl | FrameEl | GroupEl | TextEl | QrEl | DrawEl | TableEl | ChartEl | VideoEl | GridEl | FormEl | EmbedEl;
export type ElementOf<T extends ElementType> = Extract<Element, {
    type: T;
}>;
/** Fields stored at the top level of an element map (everything else is type-specific `props`). */
export declare const BASE_KEYS: readonly ["id", "type", "parentId", "z", "xf", "opacity", "blend", "visible", "lock", "name", "role", "link", "altText", "decorative", "effects", "anim", "emphasis", "keyframes", "animOut", "motionPath", "timing", "origin", "dataBinding", "deleted", "ext"];
export declare function isContainer(el: Element): el is GroupEl;
