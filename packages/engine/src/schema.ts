// SPDX-License-Identifier: Apache-2.0
/**
 * `@oc/schema` exports used by apps/web, backed by crates/oc-schema. Every function is typed as the TS original
 * (`typeof S.fn`), so a signature drift fails the build. Documents go through `docRef` (no per-call
 * serialisation); helpers that return elements/pages return the caller's own objects, like TS.
 */
import type * as S from './types/schema';
import type { Element, ElementBase, Page } from './types/schema';
import { docRef } from './docref';
import { onEngineReady, raw } from './wasm';

type Doc = Pick<S.DesignDoc, 'elements'>;

// ------------------------------------------------------------------ constants (filled when the module loads)

export let ANIM_PRESETS: typeof S.ANIM_PRESETS;
export let BORDER_PRESETS: typeof S.BORDER_PRESETS;
export let CALLOUT_TONES: typeof S.CALLOUT_TONES;
export let CHART_COLORS: typeof S.CHART_COLORS;
export let CHART_KINDS: typeof S.CHART_KINDS;
export let CURRENT_COLOR: typeof S.CURRENT_COLOR;
export let DEFAULT_PRINT_CATALOG: typeof S.DEFAULT_PRINT_CATALOG;
export let DOC_CONTENT_WIDTH: typeof S.DOC_CONTENT_WIDTH;
export let EMBED_FRAME_HOSTS: typeof S.EMBED_FRAME_HOSTS;
export let EMPHASIS_PRESETS: typeof S.EMPHASIS_PRESETS;
export let FILTER_PRESETS: typeof S.FILTER_PRESETS;
export let FRAME_CATEGORIES: typeof S.FRAME_CATEGORIES;
export let FRAME_LIBRARY: typeof S.FRAME_LIBRARY;
export let GRID_LAYOUTS: typeof S.GRID_LAYOUTS;
export let KEYFRAME_NEUTRAL: typeof S.KEYFRAME_NEUTRAL;
export let KEYFRAME_PROPS: typeof S.KEYFRAME_PROPS;
export let LINE_MARKERS: typeof S.LINE_MARKERS;
export let POPULAR_LAYOUTS: typeof S.POPULAR_LAYOUTS;
export let PRESET_CATEGORIES: typeof S.PRESET_CATEGORIES;
export let SHAPE_KINDS: typeof S.SHAPE_KINDS;
export let SIZE_PRESETS: typeof S.SIZE_PRESETS;
export let TABLE_STYLE_PRESETS: typeof S.TABLE_STYLE_PRESETS;
export let TRANSITION_KINDS: typeof S.TRANSITION_KINDS;

function deepFreeze<T>(v: T): T {
  if (v && typeof v === 'object' && !Object.isFrozen(v)) {
    Object.freeze(v);
    for (const x of Object.values(v)) deepFreeze(x);
  }
  return v;
}

onEngineReady(() => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- shapes are the TS constants' (checked by the tests)
  const c = deepFreeze(raw.schemaConstants()) as any;
  ({
    ANIM_PRESETS, BORDER_PRESETS, CALLOUT_TONES, CHART_COLORS, CHART_KINDS, CURRENT_COLOR, DEFAULT_PRINT_CATALOG, DOC_CONTENT_WIDTH, EMBED_FRAME_HOSTS,
    EMPHASIS_PRESETS, FILTER_PRESETS, FRAME_CATEGORIES, FRAME_LIBRARY, GRID_LAYOUTS, KEYFRAME_NEUTRAL, KEYFRAME_PROPS, LINE_MARKERS,
    POPULAR_LAYOUTS, PRESET_CATEGORIES, SHAPE_KINDS, SIZE_PRESETS, TABLE_STYLE_PRESETS, TRANSITION_KINDS,
  } = c);
});

// ------------------------------------------------------------------ text, colour, ids

export const bodyPlainText: typeof S.bodyPlainText = (body) => raw.bodyPlainText(body);
export const textBody: typeof S.textBody = (text, style = {}, align = 'start') => raw.textBody(text, style, align);
export const hex: typeof S.hex = (s) => raw.hex(s);
export const solid: typeof S.solid = (c) => raw.solid(c);
export const toHex: typeof S.toHex = (c, withAlpha = false) => raw.toHex(c, withAlpha);
export const toCss: typeof S.toCss = (c) => raw.toCss(c);
export const newId: typeof S.newId = () => raw.newId() as never;

// ------------------------------------------------------------------ factory

export const makeText: typeof S.makeText = (init) => raw.makeText(init);
export const makeShape: typeof S.makeShape = (init) => raw.makeShape(init);
export const makeLine: typeof S.makeLine = (init) => raw.makeLine(init);
export const makeStroke: typeof S.makeStroke = (color, width, dash) => raw.makeStroke(color, width, dash);
export const makeImage: typeof S.makeImage = (init) => raw.makeImage(init);
export const makeFrame: typeof S.makeFrame = (init) => raw.makeFrame(init);
export const gridCellBox: typeof S.gridCellBox = (el, cell) => raw.gridCellBox({ gap: el.gap, xf: el.xf }, cell);
export const makeGrid: typeof S.makeGrid = (init) => raw.makeGrid(init);
export const makeQr: typeof S.makeQr = (init) => raw.makeQr(init);
export const makeTable: typeof S.makeTable = (init) => raw.makeTable(init);
export const makeChart: typeof S.makeChart = (init) => raw.makeChart(init);
export const makeVideo: typeof S.makeVideo = (init) => raw.makeVideo(init);
export const makeForm: typeof S.makeForm = (init) => raw.makeForm(init);
export const makeEmbed: typeof S.makeEmbed = (init) => raw.makeEmbed(init);
export const emptyDesign: typeof S.emptyDesign = (init) => raw.emptyDesign(init);
/** A transform box (`rot` in degrees). */
export const xf: typeof S.xf = (x, y, w, h, rot = 0) => ({ x, y, w, h, rot });
/** An empty group (children point at it with `parentId`), like oc-schema's `make_group`. */
export const makeGroup: typeof S.makeGroup = (init) => ({ id: newId(), parentId: '' as never, z: 'a0', opacity: 1, ...init, type: 'group' });
export const clipLengthMs: typeof S.clipLengthMs = (c) => raw.clipLengthMs(c);
export const formLayout: typeof S.formLayout = (el) => raw.formLayout(el);

/** `placeAll(parentId, els, after)`: ordered z keys from Rust, spread over the caller's objects (extra fields kept). */
export const placeAll: typeof S.placeAll = <T extends ElementBase>(parentId: string, els: T[], after: string | null = null): T[] => {
  const keys = raw.placeAllKeys(after ?? undefined, els.length);
  return els.map((e, i) => ({ ...e, parentId, z: keys[i]! }));
};

// ------------------------------------------------------------------ tree, timeline

const byIds = <T>(map: Record<string, T>, ids: string[]): T[] => ids.map((id) => map[id]!);

export const sortedPages: typeof S.sortedPages = (doc) => byIds(doc.pages, raw.sortedPages(docRef(doc))) as Page[];
export const childrenOf: typeof S.childrenOf = (doc, parentId) => byIds(doc.elements, raw.childrenOf(docRef(doc), parentId)) as Element[];
export const descendants: typeof S.descendants = (doc, id) => byIds(doc.elements, raw.descendants(docRef(doc), id)) as Element[];
export const pageOf: typeof S.pageOf = (doc, id) => raw.pageOf(docRef(doc), id) as never;
export const pageDurationMs: typeof S.pageDurationMs = (doc: Doc, page) => raw.pageDurationMs(docRef(doc), page);

type Span = { pageId: string; index: number; startMs: number; durationMs: number };
const span = (doc: S.DesignDoc, s: Span) => ({ page: doc.pages[s.pageId as S.PageId]!, index: s.index, startMs: s.startMs, durationMs: s.durationMs });

export const timelineOf: typeof S.timelineOf = (doc, includeHidden = false) => {
  const t = raw.timelineOf(docRef(doc), includeHidden) as { spans: Span[]; totalMs: number };
  return { spans: t.spans.map((s) => span(doc, s)), totalMs: t.totalMs } as ReturnType<typeof S.timelineOf>;
};

export const locateTime: typeof S.locateTime = (doc, t) => {
  const r = raw.locateTime(docRef(doc), t) as { span: Span; pageTimeMs: number } | null;
  return r ? ({ span: span(doc, r.span), pageTimeMs: r.pageTimeMs } as ReturnType<typeof S.locateTime>) : null;
};

export const elementWindow: typeof S.elementWindow = (el, pageMs) => raw.elementWindow(el, pageMs);
export const mediaSourceTime: typeof S.mediaSourceTime = (el, clip, pageTimeMs, pageMs) => raw.mediaSourceTime(el, clip, pageTimeMs, pageMs) ?? null;
export const videoSlots: typeof S.videoSlots = (el) => raw.videoSlots(el);

// ------------------------------------------------------------------ tables

export const tableGeometry: typeof S.tableGeometry = (el) => raw.tableGeometry(el);
export const cellRect: typeof S.cellRect = (el, r, c, geo) => raw.cellRect(el, r, c, geo);
export const canUnmerge: typeof S.canUnmerge = (el, range) => raw.canUnmerge(el, range);
export const tableStyleSwatch: typeof S.tableStyleSwatch = (preset) => raw.tableStyleSwatch(preset);

// ------------------------------------------------------------------ presets, units

export const findPreset: typeof S.findPreset = (id) => raw.findPreset(id ?? undefined);
export const presetsInCategory: typeof S.presetsInCategory = (cat) => raw.presetsInCategory(cat);
export const presetPlatforms: typeof S.presetPlatforms = (list) => raw.presetPlatforms(list);
export const popularPresets: typeof S.popularPresets = () => raw.popularPresets();
export const tryNewPresets: typeof S.tryNewPresets = (recentIds, max = 4) => raw.tryNewPresets(recentIds, max);
export const pushRecentPreset: typeof S.pushRecentPreset = (list, id, max = 8) => raw.pushRecentPreset(list, id, max);
export const searchPresets: typeof S.searchPresets = (q, translate) => raw.searchPresets(q, translate);
export const lockedSide: typeof S.lockedSide = (changed, value, ratio, decimals = 2) => raw.lockedSide(changed, value, ratio, decimals);
export const pxToUnit: typeof S.pxToUnit = (px, unit) => raw.pxToUnit(px, unit);
export const unitToPx: typeof S.unitToPx = (v, unit) => raw.unitToPx(v, unit);
export const formatDims: typeof S.formatDims = (w, h, unit) => raw.formatDims(w, h, unit);

// ------------------------------------------------------------------ print

export const expectedPages: typeof S.expectedPages = (p, picked) => raw.expectedPages(p, picked);
export const preflightTargetFor: typeof S.preflightTargetFor = (p, sizeId, options = {}) => raw.preflightTargetFor(p, sizeId, options);
export const standalonePrintTarget: typeof S.standalonePrintTarget = (doc) => raw.standalonePrintTarget(docRef(doc));
export const preflightPrint: typeof S.preflightPrint = (doc, target, opts = {}) =>
  raw.preflightPrint(docRef(doc), target, opts.availableFonts, opts.outOfGamut);
export const printColors: typeof S.printColors = (doc, max = 512) => raw.printColors(docRef(doc), max);
export const printColorKey: typeof S.printColorKey = (c) => raw.printColorKey(c);
export const quotePrint: typeof S.quotePrint = (catalog, sel, dest = {}, shippingMethod, currency) =>
  raw.quotePrint(catalog, sel, dest, shippingMethod, currency ?? (catalog.currency || 'usd'));
export const validateAddress: typeof S.validateAddress = (a, countries) => raw.validateAddress(a, countries ? [...countries] : undefined);

// ------------------------------------------------------------------ docs, embeds, svg, keyframes

export const calloutTone: typeof S.calloutTone = (v) => raw.calloutTone(v);
export const docBlockWidth: typeof S.docBlockWidth = (w, fallback) => raw.docBlockWidth(w, fallback);
export const parseDocChart: typeof S.parseDocChart = (r) => raw.parseDocChart(r);
export const serializeDocChart: typeof S.serializeDocChart = (d) => raw.serializeDocChart(d);
export const docChartElement: typeof S.docChartElement = (d, box) => raw.docChartElement(d, box);
export const parseEmbedUrl: typeof S.parseEmbedUrl = (u) => raw.parseEmbedUrl(u);
export const extractSvgColors: typeof S.extractSvgColors = (svg) => raw.extractSvgColors(svg);
export const sanitizeSvg: typeof S.sanitizeSvg = (svg) => raw.sanitizeSvg(svg);
export const keyframeValue: typeof S.keyframeValue = (keys, t, neutral = 0) => raw.keyframeValue(keys, t, neutral);
