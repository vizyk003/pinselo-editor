// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
/**
 * Editor engine (docs/plan/03 §8). DOM-free: the React shell feeds it pointer/keyboard input in scene
 * coordinates and renders `state`. All persistent changes go through DocStore.apply (DocOps).
 * Gestures write transient overrides (previews) and commit once on pointer-up, so a drag is one undo
 * step and peers never see half-finished moves.
 */
import { type FontSet } from './styles';
import { type KeyframeEase, type KeyframeProp, type Keyframes } from '../schema';
import { type DocOp, type DocStore } from '../crdt';
import { worldMatrix, type AlignEdge, type Guide, type SpacingMark } from '../geometry';
import { type AudioClip, type CaptionCue, type CaptionTrack, type Brush, type DesignDoc, type Element, type ElementId, type ImageEl, type Page, type PageId, type ParagraphAttrs, type Rect, type AnchorSide, type AssetId, type RunStyle, type TableEl, type TextEl, type Vec2, type Xf, type BorderPreset, type CellBorder, type CellPos, type CellRange, type TableCell, type TableStylePreset } from '../schema';
import { type Handle } from './transform';
import { pageSlots, type Viewport } from './viewport';
/** Text geometry, injected by the UI layer (backed by @oc/text / SkParagraph). Box-space coordinates. */
export interface TextService {
    height(el: TextEl, width: number): number;
    offsetAt(el: TextEl, x: number, y: number): number;
    verticalMove(el: TextEl, offset: number, dir: -1 | 1): number;
    wordAt(el: TextEl, offset: number): [number, number];
    length(el: TextEl): number;
    /** Natural box size of curved text (ED-041); null while fonts load. */
    curvedSize?(el: TextEl): {
        w: number;
        h: number;
    } | null;
}
/** Style picked up by "Copy style" (ED-070), applied to compatible elements. */
export interface CopiedStyle {
    from: Element['type'];
    base: Partial<Pick<Element, 'opacity' | 'blend' | 'effects'>>;
    text?: {
        run: Partial<RunStyle>;
        paragraph: Partial<ParagraphAttrs>;
        effect?: TextEl['effect'];
    };
    shape?: {
        fills: unknown;
        strokes: unknown;
        cornerRadius?: number;
    };
    image?: {
        edits?: ImageEl['edits'];
        cornerRadius?: number;
    };
    line?: {
        stroke: unknown;
        startMarker: unknown;
        endMarker: unknown;
        route?: unknown;
    };
}
export interface FindMatch {
    id: ElementId;
    pageId: PageId;
    offset: number;
    length: number;
    context: string;
}
export interface Mods {
    shift: boolean;
    alt: boolean;
    meta: boolean;
}
export interface TextEditState {
    id: ElementId;
    anchor: number;
    focus: number;
}
export interface EditorState {
    activePageId: PageId;
    selection: ElementId[];
    hover: ElementId | null;
    enteredGroup: ElementId | null;
    editingText: TextEditState | null;
    croppingId: ElementId | null;
    viewport: Viewport;
    guides: Guide[];
    /** Scene-space marquee while dragging on empty canvas. */
    marquee: Rect | null;
    /** True while a gesture is in progress (UI hides floating toolbars). */
    gesturing: boolean;
    presenting: boolean;
    /** Active tool: select (default), freehand draw, hand (pan), or the stroke eraser (g55; Rust engine only). */
    tool: 'select' | 'draw' | 'hand' | 'erase';
    brush: {
        kind: Brush;
        color: string;
        size: number;
    };
    /** In-progress freehand stroke in page space (overlay preview). */
    drawing: {
        pageId: PageId;
        points: number[];
    } | null;
    /** Table cell being edited (the caret cell; also the anchor of a cell range). */
    editingCell: {
        id: ElementId;
        r: number;
        c: number;
    } | null;
    /** Cells selected inside a table (shift+click / drag across cells while editing). */
    cellRange: ({
        id: ElementId;
    } & CellRange) | null;
    /** Animation preview: wall-clock start (ms) and the page being previewed. */
    preview: {
        pageId: PageId;
        startedAt: number;
    } | null;
    /** Timeline playhead in design time; null = static editing view. */
    playhead: number | null;
    playing: boolean;
    /** Wall-clock anchor while playing: playhead = anchorTime + (now - anchorAt). */
    playAnchor: {
        at: number;
        time: number;
    } | null;
    /** Equal-spacing measurements shown while dragging. */
    spacing: SpacingMark[];
    /** Connector being dragged: the anchor it will glue to on release. */
    connectTarget: {
        id: ElementId;
        anchor: AnchorSide;
        point: Vec2;
    } | null;
    /** "Copy style" is armed: the next click applies it. */
    styleBrush: CopiedStyle | null;
    /** Keyframe recording (MV-100): moving/rotating at the playhead writes keyframes instead of moving the element. */
    recordKeyframes: boolean;
}
export type HitTarget = {
    kind: 'handle';
    handle: Handle;
} | {
    kind: 'rotate';
} | {
    kind: 'endpoint';
    end: 0 | 1;
} | {
    kind: 'canvas';
}
/** Drag the border after column `index` of a table. */
 | {
    kind: 'tableCol';
    id: ElementId;
    index: number;
};
declare const CLIPBOARD_MIME = "application/x-opencanvas+json";
/**
 * The key a shortcut matches on. Option on a Mac (and many non-US layouts) changes `key` (⌥⌘[ is '“',
 * ⌥⌘L is '¬'), so when `key` isn't a plain ASCII character we fall back to the physical key in `code`.
 */
export declare function shortcutKey(key: string, code?: string): string;
export interface ClipboardPayload {
    kind: typeof CLIPBOARD_MIME;
    elements: Element[];
    roots: ElementId[];
    assets: DesignDoc['assets'];
}
/** Keyframes with a key set at `t` (replacing one within 20 ms); a first key after 0 also pins the start at neutral. */
export declare function withKeyframe(k: Keyframes, prop: KeyframeProp, t: number, v: number, ease?: KeyframeEase): Keyframes;
/** Extended grapheme cluster boundaries of `s` (UTF-16 offsets, from 0 to `s.length`). */
export declare function graphemeBounds(s: string): number[];
/** The grapheme boundary before `offset` (the start of the cluster it is in or follows; 0 at the start). */
export declare function prevGrapheme(s: string, offset: number): number;
/** The grapheme boundary after `offset` (`s.length` at the end). */
export declare function nextGrapheme(s: string, offset: number): number;
export declare class Editor {
    readonly store: DocStore;
    state: EditorState;
    textService: TextService | null;
    /** Transient overrides merged over the document while a gesture is running. */
    overrides: Map<ElementId, Partial<Element>>;
    private gesture;
    private readonly listeners;
    private clipboard;
    /** Styles to apply to the next typed characters (toggled with no selection). */
    private pendingMarks;
    constructor(store: DocStore, viewport?: Viewport);
    private caretMark;
    /** The text body fragment behind the caret (a text element; table cells are laid out from the table). */
    private caretFragment;
    private stickyCaret;
    private restoreCaret;
    get doc(): DesignDoc;
    subscribe(fn: () => void): () => void;
    /** Bumped on every notification; UI caches derived values on it. */
    version: number;
    private emit;
    private set;
    /** Document view including transient overrides (what the renderer and overlay should show). */
    get view(): DesignDoc;
    /**
     * Apply ops as the local user. Discrete commands are their own undo step; `merge` lets rapid
     * repeats (typing, nudges, colour-picker drags) coalesce within the undo capture window.
     */
    apply(ops: DocOp[], merge?: boolean): import("@oc/crdt").DocOpResult;
    /** Drop selection entries that no longer exist (deleted remotely) and keep the active page valid. */
    private sanitize;
    /** Auto-height text: keep xf.h equal to the laid-out height after local edits. */
    private fixTextHeights;
    /** Connectors follow the elements they are glued to (persisted with the same local change). */
    private fixConnectors;
    /**
     * Shape labels (WB-004): text that lives inside a shape. The label is a centred text element grouped
     * with the shape, so moving or resizing the shape carries it along.
     */
    editShapeLabel(shapeId: ElementId): void;
    /**
     * A shape that gains a label is wrapped in a group; the group takes over the board metadata of older
     * documents (mind-map node, section, …): the metadata, glued connectors and mind-map children now point at the group.
     */
    private adoptBoardRole;
    /** Re-measure every text element (after fonts load). */
    remeasureText(): void;
    pageOrigin(pageId: PageId): Vec2;
    /** Scene point → page-local point. */
    toPage(pageId: PageId, p: Vec2): [number, number];
    selected(): Element[];
    selectionBox(): Rect | null;
    /** The single selected element's world quad + rotation (for rotated selection chrome). */
    selectionFrame(): {
        xf: Xf;
        matrix: ReturnType<typeof worldMatrix>;
    } | null;
    private topLevelSiblings;
    select(ids: ElementId[], opts?: {
        keepText?: boolean;
    }): void;
    clearSelection(): void;
    selectAll(): void;
    setActivePage(pageId: PageId, scroll?: boolean): void;
    setHover(id: ElementId | null): void;
    setViewportSize(width: number, height: number): void;
    setViewport(vp: Viewport): void;
    zoomTo(zoom: number, screenPt?: Vec2): void;
    zoomBy(factor: number, screenPt?: Vec2): void;
    panBy(dx: number, dy: number): void;
    zoomToFit(): void;
    scrollToPage(pageId: PageId): void;
    /** Page whose centre is closest to the viewport centre (tracks scrolling). */
    updateActivePageFromScroll(): void;
    pointerDown(p: Vec2, mods: Mods, target?: HitTarget, button?: number): void;
    pointerMove(p: Vec2, mods: Mods, pressure?: number): void;
    private resizeMove;
    private originDoc;
    /** Abandon the gesture in progress without committing it (a second finger turned a drag into a pinch). */
    cancelGesture(): void;
    pointerUp(): void;
    /** Line patch whose box wraps two page-space ends (top-level or inside a group). */
    private lineFromEnds;
    doubleClick(p: Vec2): void;
    /** Centre of the visible canvas in page coordinates. */
    viewCenter(pageId?: PageId): Vec2;
    /** Fit a section (or any element) in the view. */
    zoomToElement(id: ElementId, pad?: number): void;
    setTool(tool: EditorState['tool']): void;
    setBrush(b: Partial<EditorState['brush']>): void;
    private finishStroke;
    /**
     * Edit a cell with the same text engine as text boxes: the caret lives in `editingText` (id = the table) and
     * `editingCell` says which cell. Without a point the cell's text is selected (spreadsheet-style Tab entry).
     */
    startCellEdit(id: ElementId, r: number, c: number, pagePt?: Vec2, selectWord?: boolean): void;
    stopCellEdit(): void;
    /** Select the cells between two cells (grown to whole merged cells). A single cell goes back to typing. */
    selectCellRange(id: ElementId, from: CellPos, to: CellPos): void;
    /** Move the caret cell by Tab / Shift+Tab (wrapping across rows, skipping merged cells). */
    cellTab(dir: 1 | -1): void;
    /** Virtual text element the text engine lays out for a body at a width (id doubles as the layout cache key). */
    private virtualText;
    /**
     * A cell as a text element in the cell's content box: `box` and the vertical-alignment offset `dy` are in
     * table-local px. The id is the same layout key the renderer uses, so measurement and drawing agree.
     */
    cellTextEl(id: ElementId, r: number, c: number, doc?: DesignDoc): {
        el: TextEl;
        box: Rect;
        dy: number;
    } | null;
    /** Text being edited and where its layout origin sits inside the host element (for caret/selection chrome). */
    editingTextFrame(): {
        el: TextEl;
        hostId: ElementId;
        ox: number;
        oy: number;
    } | null;
    /** The text element (real, or a cell's virtual one) the caret is in. */
    private editTarget;
    private cellOffsetAt;
    private tableSlotAt;
    /** Text ops on the edited text: written to the text element, or folded into the edited cell's body. */
    private applyTextOps;
    /** Rewrite cell bodies (rich text) in one update; legacy cells are migrated as they are touched. */
    private editCellBodies;
    /** Cell under a page point for a table element (the merged cell's anchor), or null. */
    cellAt(id: ElementId, pagePt: Vec2): {
        r: number;
        c: number;
    } | null;
    /** Patch one cell. Setting plain `text` replaces any rich body. */
    setCell(id: ElementId, r: number, c: number, patch: Partial<TableEl['cells'][string]>): void;
    /** The range table commands act on: the selected cells, the caret cell, or (neither) the whole table. */
    tableTargetRange(id: ElementId): CellRange | null;
    /** Anchor cells table commands act on (see `tableTargetRange`). */
    tableTargets(id: ElementId): CellPos[];
    /** Patch every targeted cell (fill, valign, padding, bold…); `null` clears a field. */
    setCells(id: ElementId, patch: {
        [K in keyof TableCell]?: TableCell[K] | null;
    }): void;
    /** Horizontal alignment of the targeted cells (written to their paragraphs too, so rich bodies follow). */
    setCellAlign(id: ElementId, align: 'start' | 'center' | 'end'): void;
    /** Border preset over the targeted range (whole table when nothing is picked). */
    setCellBorders(id: ElementId, preset: BorderPreset, border?: Partial<CellBorder>): void;
    /** Merge the selected cell range into one cell. */
    mergeCellRange(id: ElementId): void;
    /** Split merged cells in the targeted range back into single cells. */
    unmergeCellRange(id: ElementId): void;
    /** Clear the text of the targeted cells (Delete with a cell range selected). */
    clearCells(id: ElementId): void;
    /** One of the table style presets (TableControls). */
    setTableStyle(id: ElementId, preset: TableStylePreset): void;
    /** Column widths: even distribution, or one column set to `px` (neighbour to the right absorbs the change). */
    setColumnWidth(id: ElementId, index: number | 'even', px?: number): undefined;
    /** Insert/remove rows or columns, keeping cell contents (and merged cells) aligned. */
    tableStructure(id: ElementId, action: 'addRow' | 'addCol' | 'delRow' | 'delCol', at: number): void;
    previewAnimation(pageId?: PageId): void;
    stopPreview(): void;
    /** Current design time, advancing with the wall clock while playing. */
    currentTime(now?: number): number;
    play(): void;
    pause(): void;
    togglePlay(): void;
    /** Move the playhead (design time). Pass null to leave timeline mode. */
    seek(t: number | null): void;
    /** Called every animation frame by the UI while playing: stops at the end and follows pages. */
    tick(): void;
    pageStart(pageId: PageId): number;
    setPageDuration(pageId: PageId, ms: number): void;
    setTiming(id: ElementId, timing: {
        startMs: number;
        endMs?: number;
    } | null): void;
    /** Insert a video sized to fit ~70% of the page. */
    insertVideo(asset: {
        id: ImageEl['asset'];
        w: number;
        h: number;
        durationMs?: number;
        animated?: boolean;
    }, at?: Vec2): ElementId[];
    /** Split: the selected video at the playhead, or else the whole page into two pages. */
    splitAtPlayhead(): void;
    audioClips(): AudioClip[];
    private setAudio;
    addAudio(asset: {
        id: ImageEl['asset'];
        durationMs?: number;
        name?: string;
    }, at?: number): string;
    updateAudio(id: string, patch: Partial<AudioClip>, merge?: boolean): void;
    removeAudio(id: string): void;
    splitAudio(id: string, t?: number): void;
    captions(): CaptionTrack;
    setCaptions(track: CaptionTrack, merge?: boolean): void;
    upsertCue(cue: Partial<CaptionCue> & {
        id?: string;
    }): string;
    startTextEdit(id: ElementId, pagePt?: Vec2, selectWord?: boolean): void;
    stopTextEdit(): void;
    private textRange;
    typeText(str: string): void;
    private mergedMarks;
    deleteText(dir: -1 | 1, word?: boolean): void;
    moveCaret(key: 'left' | 'right' | 'up' | 'down' | 'home' | 'end', extend: boolean, word?: boolean): void;
    plainText(el: TextEl): string;
    selectedText(): string;
    /** Apply run marks: to the text selection while editing, else to whole text elements in the selection. */
    setTextStyle(marks: Partial<RunStyle> & Record<string, unknown>): void;
    setParagraphStyle(attrs: Partial<ParagraphAttrs>): void;
    /** Effective run style at the caret / for the first selected text element (toolbar state). */
    currentTextStyle(): RunStyle | null;
    currentParagraph(): ParagraphAttrs | null;
    /** First cell of a selected cell range as text (toolbar state while cells are selected). */
    private rangeTextEl;
    /** True while the text toolbar applies to table cells (caret in a cell or a cell range). */
    get editingTableText(): boolean;
    /** Selection plus descendants of selected groups (for styling a group's contents). */
    selectedDeep(): Element[];
    /** Patch every selected element (type-filtered). */
    updateSelected(patch: Record<string, unknown> | ((el: Element) => Record<string, unknown> | null), deep?: boolean, merge?: boolean): void;
    /** Insert elements on the active page, centred (or at a page point), and select them. */
    /**
     * Page numbers (ED-095): a `{{page}}` text box at the same spot on every page (skipping pages that
     * already have one). One undo step.
     */
    addPageNumbers(o?: {
        text?: string;
        position?: 'bottom-right' | 'bottom-center' | 'bottom-left' | 'top-right';
        size?: number;
    }): number;
    insert(els: Element[], at?: Vec2, opts?: {
        select?: boolean;
        edit?: boolean;
    }): ElementId[];
    deleteSelection(): void;
    /** Deep copy of the selection with fresh ids; roots keep their parent. */
    private cloneSubtrees;
    duplicate(): void;
    copy(): ClipboardPayload | null;
    cut(): ClipboardPayload | null;
    /** Paste a payload (from the system clipboard or the internal one). Ids are refreshed every time. */
    paste(payload?: ClipboardPayload | null, offset?: number): void;
    static readonly CLIPBOARD_MIME = "application/x-opencanvas+json";
    nudge(dx: number, dy: number): void;
    arrange(where: 'forward' | 'backward' | 'front' | 'back'): void;
    /** Move an element to a z-position among its siblings (layers panel drag). */
    reorderTo(id: ElementId, after: ElementId | 'top' | 'bottom'): void;
    align(edge: AlignEdge): void;
    distribute(axis: 'x' | 'y'): void;
    /** Tidy: arrange the selection in a grid with equal gaps. */
    tidy(): void;
    private applyDeltas;
    flip(axis: 'x' | 'y'): void;
    toggleLock(): void;
    group(): void;
    ungroup(): void;
    startCrop(id: ElementId): void;
    stopCrop(): void;
    /** Zoom the image inside its crop (crop-mode slider). 1 = the image just covers the box. */
    setCropZoom(id: ElementId, zoom: number): void;
    cropZoom(id: ElementId): number;
    /**
     * The part of the photo shown in an image, a filled frame or a shape filled with a photo (its topmost image
     * fill). A shape's photo never stretches: its stored crop is refit to the box's aspect (render-core `shapePhoto`
     * does the same), so resizing by any means — W/H fields, group scaling, a collaborator, pasted styles — keeps it
     * in proportion. Without a stored crop: the cover crop of a `fill`-fit photo of known size. Null when there is none.
     */
    mediaCrop(el: Element): Rect | null;
    /** Patch that sets the crop `mediaCrop` reads. */
    private cropPatch;
    /** Put an image into a frame (drop onto frame) or replace an image's asset, keeping the box. */
    replaceImage(id: ElementId, asset: ImageEl['asset'], aspect: number): void;
    /** Pick up the style of the single selected element; the next click (or `pasteStyle`) applies it. */
    copyStyle(): CopiedStyle | null;
    cancelStyleBrush(): void;
    /** Apply a copied style to elements (defaults: the selection). Incompatible parts are skipped. */
    pasteStyle(ids?: ElementId[], style?: CopiedStyle | null): void;
    /** Case-insensitive (by default) search across every text element and table cell of the design. */
    find(query: string, opts?: {
        matchCase?: boolean;
        wholeWord?: boolean;
    }): FindMatch[];
    /** Select and reveal a match (selects the text range in edit mode). */
    revealMatch(m: FindMatch): void;
    /** Replace matches (all when `only` is omitted). One undo step. Returns the number replaced. */
    replace(query: string, replacement: string, opts?: {
        matchCase?: boolean;
        wholeWord?: boolean;
        only?: FindMatch;
    }): number;
    /** Set page-space position / size / rotation of the single selection (or move a multi-selection). */
    setGeometry(g: Partial<{
        x: number;
        y: number;
        w: number;
        h: number;
        rot: number;
    }>): void;
    setGuides(pageId: PageId, guides: Array<{
        axis: 'x' | 'y';
        pos: number;
    }>): void;
    addGuide(pageId: PageId, axis: 'x' | 'y', pos: number): void;
    /** Index of the grid cell under a page point, or -1. */
    gridCellAt(id: ElementId, pagePoint: Vec2): number;
    setGridCell(id: ElementId, index: number, asset: AssetId | null): void;
    /** Default clip settings when a video asset fills a frame or grid cell; undefined for images. */
    private videoClipFor;
    /** Put an image or video into a frame, cover-cropped to the frame box. */
    fillFrame(id: ElementId, asset: AssetId, aspect?: number): void;
    /** Drop an asset at a page point: fills a grid cell or a frame under the point. Returns true when used. */
    dropIntoContainer(pageId: PageId, local: Vec2, asset: AssetId): boolean;
    /** Elements on a page that can take data: text, images, frames (with their current binding). */
    bindableElements(pageId?: PageId): Element[];
    setDataBinding(id: ElementId, column: string | null): void;
    /**
     * One new page per data row, cloned from `pageId` with bound elements filled in. Image values are
     * resolved by the caller (uploads or imported URLs) into assets beforehand. Returns the new page ids.
     */
    bulkCreate(pageId: PageId, rows: Array<Record<string, string>>, images?: ReadonlyMap<string, {
        id: AssetId;
        w: number;
        h: number;
    }>): PageId[];
    addPage(after?: PageId | null): PageId | null;
    duplicatePage(pageId?: PageId): void;
    deletePage(pageId?: PageId): void;
    movePage(pageId: PageId, toIndex: number): void;
    setRecordKeyframes(on: boolean): void;
    /** Page time of the playhead on an element's page (null when the timeline isn't open). */
    keyframeTime(id: ElementId): number | null;
    setKeyframe(id: ElementId, prop: KeyframeProp, t: number, v: number, ease?: KeyframeEase): void;
    removeKeyframe(id: ElementId, prop: KeyframeProp, t: number): void;
    /** Move a keyframe in time (all properties keyed at `from` move together when `prop` is omitted). */
    moveKeyframe(id: ElementId, from: number, to: number, prop?: KeyframeProp): void;
    clearKeyframes(id: ElementId): void;
    /** Replace colours everywhere (hex → hex). One undo step. Returns the number of changes. */
    replaceColors(map: Record<string, string>): number;
    /** Recolour the design with a palette (luminance-ranked mapping). */
    applyPalette(colors: string[]): number;
    /** Apply a heading/body font pair to every text. */
    applyFonts(set: FontSet): number;
    /** Document language (BCP-47) for screen readers, hyphenation and spell-check. */
    setLanguage(lang: string): void;
    updatePage(pageId: PageId, patch: Partial<Page>): void;
    /** Change the design's page size; elements keep their position (Canva's free "Resize"). */
    resizeDesign(w: number, h: number, unit?: import("@oc/schema").Unit, scaleContent?: boolean): void;
    undo(): void;
    redo(): void;
    private stopTextEditIfGone;
    setPresenting(on: boolean): void;
    /** Returns true when the key was handled. Text-editing keys are handled by the UI's IME proxy. */
    /**
     * Text shortcuts (Canva set): ⌘B/I/U, ⇧⌘> / ⇧⌘< size, ⇧⌘L/C/R/J alignment, ⇧⌘K uppercase,
     * ⇧⌘7 / ⇧⌘8 numbered / bulleted list. Work while editing text and on selected text boxes.
     */
    textShortcut(key: string, mods: Mods & {
        ctrl?: boolean;
    }, code?: string): boolean;
    handleKey(key: string, mods: Mods & {
        ctrl?: boolean;
    }, code?: string): boolean;
}
/** Crop rect (normalized) that makes an image of `imgAspect` cover a box of `boxAspect`. */
export declare function coverCrop(boxAspect: number, imgAspect: number): Rect;
export { pageSlots };
