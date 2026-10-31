/**
 * Template builder: a small fluent API over the schema factories for authoring original templates.
 * Text uses any bundled or Google Fonts family; photos are pinned CC0 images (photos.ts); illustrations
 * can use Noto emoji artwork. Templates are plain design documents and validate like any design.
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { Resvg } from '@resvg/resvg-js';
import {
  emptyDesign, GRID_LAYOUTS, hex, makeChart, makeFrame, makeGrid, makeImage, makeLine, makeQr, makeShape, makeStroke, makeTable, makeText, SIZE_PRESETS, sortedPages,
  type AssetId, type AssetRef, type ChartEl, type ChartKind, type DesignDoc, type Element, type FrameClip, type ImageEditStack, type ImageFilterPreset, type Paint, type RunStyle, type ShapeKind, type TextEffect,
} from '@oc/engine';
import { preparedPhoto } from './photos';

const ROOT = resolve(import.meta.dirname, '../../..');
export type Align = 'start' | 'center' | 'end' | 'justify';
export const lin = (angle: number, ...stops: string[]): Paint => ({ k: 'linear', angle, stops: stops.map((c, i) => ({ t: i / (stops.length - 1), color: hex(c) })) });
export const rad = (...stops: string[]): Paint => ({ k: 'radial', stops: stops.map((c, i) => ({ t: i / (stops.length - 1), color: hex(c) })) });
export const solid = (c: string): Paint => ({ k: 'solid', color: hex(c) });
const paint = (p: Paint | string) => (typeof p === 'string' ? solid(p) : p);

/** Library assets (photos, emoji art) used by templates; the seed uploads them. */
/** `file`: where the bytes already are (pinned photos); generated art (emoji PNGs) only has `bytes`. */
export interface LibAsset { ref: AssetRef; bytes: () => Buffer; file?: string; source: 'stock' | 'emoji'; attribution?: Record<string, unknown> }
export const LIB_ASSETS = new Map<string, LibAsset>();
const B62 = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
const stableId = (prefix: string, key: string) => {
  const h = createHash('sha256').update(key).digest();
  let s = prefix;
  for (let i = 0; s.length < 16; i++) s += B62[h[i]! % 62];
  return s as AssetId;
};

const provider = (p: { source: string }) => (p.source === 'unsplash' ? 'Unsplash' : `Openverse · ${p.source}`);
const license = (p: { license: string }) => (p.license === 'unsplash' ? 'Unsplash License' : p.license === 'pdm' ? 'Public Domain' : 'CC0');

function photoAsset(id: string): AssetRef {
  const p = preparedPhoto(id);
  if (!p) throw new Error(`photo ${id} not prepared — templates must be built with buildTemplates()`);
  const aid = stableId('P', id);
  if (!LIB_ASSETS.has(aid)) {
    LIB_ASSETS.set(aid, {
      ref: { id: aid, kind: 'image', sha256: p.sha256, mime: 'image/jpeg', w: p.w, h: p.h, name: p.pin.title || 'Photo',
        attribution: { provider: provider(p.pin), author: p.pin.creator, authorUrl: p.pin.creatorUrl, license: license(p.pin), licenseUrl: p.pin.licenseUrl, sourceUrl: p.pin.landing } },
      bytes: () => readFileSync(p.file), file: p.file, source: 'stock',
      attribution: { provider: provider(p.pin), author: p.pin.creator, license: license(p.pin), licenseUrl: p.pin.licenseUrl, sourceUrl: p.pin.landing },
    });
  }
  return LIB_ASSETS.get(aid)!.ref;
}

function emojiAsset(ch: string): AssetRef {
  const hexes = [...ch].map((c) => c.codePointAt(0)!.toString(16).toUpperCase().padStart(4, '0'));
  const candidates = [hexes.join('-'), hexes.filter((h) => h !== 'FE0F').join('-')];
  const file = candidates.map((h) => join(ROOT, 'fixtures/emoji/svg', `${h}.svg`)).find((f) => { try { readFileSync(f); return true; } catch { return false; } });
  if (!file) throw new Error(`no emoji artwork for ${ch} (run pnpm emoji:sync)`);
  const aid = stableId('E', file);
  if (!LIB_ASSETS.has(aid)) {
    const png = Buffer.from(new Resvg(readFileSync(file, 'utf8'), { fitTo: { mode: 'width', value: 512 }, font: { loadSystemFonts: false } }).render().asPng());
    LIB_ASSETS.set(aid, { ref: { id: aid, kind: 'image', sha256: createHash('sha256').update(png).digest('hex'), mime: 'image/png', w: 512, h: 512, name: `emoji ${ch}` }, bytes: () => png, source: 'emoji' });
  }
  return LIB_ASSETS.get(aid)!.ref;
}

/** Cover crop of an image (aspect ia) into a box (aspect ba), with a focus point 0..1. */
function cover(ba: number, ia: number, focus: [number, number] = [0.5, 0.5]) {
  if (ia > ba) { const w = ba / ia; return { x: Math.min(1 - w, Math.max(0, focus[0] - w / 2)), y: 0, w, h: 1 }; }
  const h = ia / ba;
  return { x: 0, y: Math.min(1 - h, Math.max(0, focus[1] - h / 2)), w: 1, h };
}

export interface PhotoOpts { r?: number; clip?: FrameClip; focus?: [number, number]; filter?: ImageFilterPreset; filterAmount?: number; duotone?: [string, string]; opacity?: number; rot?: number; adjust?: NonNullable<ImageEditStack['adjust']> }
export interface TextOpts { lh?: number; role?: Element['role']; effect?: TextEffect; curve?: number; opacity?: number; rot?: number; fit?: boolean; h?: number; spaceAfter?: number }

export class B {
  readonly doc: DesignDoc;
  private pageIdx = 0;
  private z = 0;
  constructor(preset: string, title: string, pages = 1) {
    const p = SIZE_PRESETS.find((x) => x.id === preset);
    if (!p) throw new Error(`unknown preset ${preset}`);
    this.doc = emptyDesign({ w: p.w, h: p.h, unit: p.unit, title, surface: p.surface, pages });
  }
  get W() { return this.doc.meta.dims.w; }
  get H() { return this.doc.meta.dims.h; }
  page(i: number) { this.pageIdx = i; this.z = 0; return this; }
  notes(s: string) { sortedPages(this.doc)[this.pageIdx]!.notes = s; return this; }
  bg(p: Paint | string) { sortedPages(this.doc)[this.pageIdx]!.background = [paint(p)]; return this; }
  add(e: Element) {
    const pid = sortedPages(this.doc)[this.pageIdx]!.id;
    const z = `a${String(this.z++).padStart(4, '0')}`;
    this.doc.elements[e.id] = { ...e, parentId: pid, z } as Element;
    return this;
  }
  rect(x: number, y: number, w: number, h: number, fill: Paint | string, o: { r?: number; rot?: number; opacity?: number; stroke?: [string, number]; shadow?: [number, number, number, string] } = {}) {
    const e = makeShape({ kind: 'rect', xf: { x, y, w, h, rot: o.rot ?? 0 }, fill: fill === 'none' ? solid('#000000') : paint(fill), params: { cornerRadius: o.r ?? 0 }, ...(o.stroke ? { stroke: makeStroke(o.stroke[0], o.stroke[1]) } : {}) });
    if (fill === 'none') e.fills = [];
    return this.add({ ...e, opacity: o.opacity ?? 1, ...(o.shadow ? { effects: [{ k: 'dropShadow', dx: o.shadow[0], dy: o.shadow[1], blur: o.shadow[2], color: hex(o.shadow[3]) }] } : {}) });
  }
  /** Outline-only rectangle. */
  box(x: number, y: number, w: number, h: number, color: string, width = 4, r = 0) {
    const e = makeShape({ kind: 'rect', xf: { x, y, w, h, rot: 0 }, fill: solid('#000000'), params: { cornerRadius: r }, stroke: makeStroke(color, width) });
    return this.add({ ...e, fills: [] });
  }
  shape(kind: ShapeKind, x: number, y: number, w: number, h: number, fill: Paint | string, o: { rot?: number; opacity?: number; params?: Record<string, number>; stroke?: [string, number] } = {}) {
    const e = makeShape({ kind, xf: { x, y, w, h, rot: o.rot ?? 0 }, fill: paint(fill), params: o.params ?? {}, ...(o.stroke ? { stroke: makeStroke(o.stroke[0], o.stroke[1]) } : {}) });
    return this.add({ ...e, opacity: o.opacity ?? 1 });
  }
  circle(cx: number, cy: number, r: number, fill: Paint | string, o: { opacity?: number } = {}) {
    return this.shape('ellipse', cx - r, cy - r, r * 2, r * 2, fill, o);
  }
  line(x: number, y: number, w: number, color: string, width = 4, o: { dash?: number[]; rot?: number } = {}) {
    const e = makeLine({ xf: { x, y: y - 10, w, h: 20, rot: o.rot ?? 0 }, color, width, dash: o.dash });
    return this.add(e);
  }
  text(s: string, x: number, y: number, w: number, style: Partial<RunStyle> & { color?: string }, align: Align = 'start', o: TextOpts = {}) {
    const { color, ...rest } = style;
    const lines = s.split('\n').length;
    const e = makeText({ xf: { x, y, w, h: o.h ?? (rest.size ?? 32) * (o.lh ?? 1.3) * lines, rot: o.rot ?? 0 }, text: s, align, style: { ...rest, ...(color ? { fill: solid(color) } : {}) } });
    if (o.lh || o.spaceAfter) e.body.paragraphs = e.body.paragraphs.map((p) => ({ ...p, ...(o.lh ? { lineHeight: o.lh } : {}), ...(o.spaceAfter ? { spaceAfter: o.spaceAfter } : {}) }));
    const el = { ...e, ...(o.role ? { role: o.role } : {}), ...(o.effect ? { effect: o.effect } : {}), ...(o.curve ? { curve: o.curve } : {}), ...(o.fit ? { autoFit: 'shrink' as const, frame: { mode: 'fixed' as const } } : {}), opacity: o.opacity ?? 1 };
    return this.add(el);
  }
  /** A photo (pinned CC0 Openverse id), cover-cropped into the box. */
  photo(id: string, x: number, y: number, w: number, h: number, o: PhotoOpts = {}) {
    const a = photoAsset(id);
    this.doc.assets = { ...this.doc.assets, [a.id]: a };
    const crop = cover(w / h, a.w / a.h, o.focus);
    const edits: ImageEditStack | undefined = o.filter || o.duotone || o.adjust
      ? { version: 1, ...(o.filter ? { filter: { preset: o.filter, amount: o.filterAmount ?? 1 } } : {}), ...(o.duotone ? { duotone: { shadow: hex(o.duotone[0]), highlight: hex(o.duotone[1]), amount: 1 } } : {}), ...(o.adjust ? { adjust: o.adjust } : {}) }
      : undefined;
    if (o.clip && o.clip !== 'rect') {
      const f = makeFrame({ xf: { x, y, w, h, rot: o.rot ?? 0 }, clip: o.clip, asset: a.id });
      return this.add({ ...f, content: { asset: a.id, crop, ...(edits ? { edits } : {}) }, opacity: o.opacity ?? 1 });
    }
    const img = makeImage({ xf: { x, y, w, h, rot: o.rot ?? 0 }, asset: a.id });
    return this.add({ ...img, crop, ...(edits ? { edits } : {}), ...(o.r ? { cornerRadius: o.r } : {}), opacity: o.opacity ?? 1, altText: a.name });
  }
  /** Full-bleed background photo for the current page. */
  bgPhoto(id: string, o: PhotoOpts = {}) {
    return this.photo(id, 0, 0, this.W, this.H, o);
  }
  /** A photo grid: one pinned photo per cell (ids may be fewer than cells; the rest stay placeholders). */
  grid(ids: string[], x: number, y: number, w: number, h: number, layout = 'grid-2x2', o: { gap?: number; radius?: number } = {}) {
    const g = makeGrid({ xf: { x, y, w, h, rot: 0 }, layout, gap: o.gap ?? 12 });
    g.radius = o.radius ?? 0;
    const cells = GRID_LAYOUTS[layout]!.cells;
    g.cells = cells.map((rect, i) => {
      const id = ids[i];
      if (!id) return { rect };
      const a = photoAsset(id);
      this.doc.assets = { ...this.doc.assets, [a.id]: a };
      return { rect, asset: a.id, crop: cover((rect.w * w) / (rect.h * h), a.w / a.h) };
    });
    return this.add(g);
  }
  /** Noto emoji artwork as an illustration (e.g. '🌿', '☕'). */
  emoji(ch: string, x: number, y: number, size: number, o: { rot?: number; opacity?: number } = {}) {
    const a = emojiAsset(ch);
    this.doc.assets = { ...this.doc.assets, [a.id]: a };
    const img = makeImage({ xf: { x, y, w: size, h: size, rot: o.rot ?? 0 }, asset: a.id });
    return this.add({ ...img, opacity: o.opacity ?? 1, decorative: true });
  }
  /** Empty image frame for the user's own photo. */
  frame(x: number, y: number, w: number, h: number, clip: FrameClip = 'rect', fill = '#d0d7de') {
    const f = makeFrame({ xf: { x, y, w, h, rot: 0 }, clip });
    return this.add({ ...f, placeholderFill: solid(fill) });
  }
  chart(kind: ChartKind, x: number, y: number, w: number, h: number, labels: string[], series: Array<{ name: string; values: number[]; color?: string }>,
    o: { family?: string; fontSize?: number; textColor?: string; gridColor?: string; legend?: boolean; values?: boolean; grid?: boolean; slices?: string[]; format?: ChartEl['style']['format']; max?: number } = {}) {
    const c = makeChart({ xf: { x, y, w, h, rot: 0 }, kind, labels, series });
    c.style = {
      ...c.style,
      ...(o.family ? { family: o.family } : {}), ...(o.fontSize ? { fontSize: o.fontSize } : {}),
      ...(o.textColor ? { textColor: hex(o.textColor) } : {}), ...(o.gridColor ? { gridColor: hex(o.gridColor) } : {}),
      ...(o.legend !== undefined ? { showLegend: o.legend } : {}), ...(o.values !== undefined ? { showValues: o.values } : {}), ...(o.grid !== undefined ? { showGrid: o.grid } : {}),
      ...(o.slices ? { sliceColors: o.slices.map((x) => hex(x)) } : {}), ...(o.format ? { format: o.format } : {}), ...(o.max ? { max: o.max } : {}),
    };
    return this.add(c);
  }
  table(x: number, y: number, w: number, h: number, data: string[][]) {
    return this.add(makeTable({ xf: { x, y, w, h, rot: 0 }, rows: data.length, cols: data[0]!.length, data }));
  }
  qr(data: string, x: number, y: number, s: number) {
    return this.add(makeQr({ xf: { x, y, w: s, h: s, rot: 0 }, data }));
  }
}

export interface TemplateDef { id: string; title: string; category: string; preset: string; tags: string[]; doc: DesignDoc }
interface Pending { id: string; title: string; category: string; preset: string; tags: string[]; pages: number; build: (b: B) => void }
const PENDING: Pending[] = [];

/** Register a template (built later, once its photos are downloaded). */
export function tpl(id: string, title: string, category: string, preset: string, tags: string[], build: (b: B) => void, pages = 1) {
  if (PENDING.some((p) => p.id === id)) throw new Error(`duplicate template id ${id}`);
  PENDING.push({ id, title, category, preset, tags, pages, build });
}

/**
 * Run `fn` with the random source the engine draws ids and fractional-index jitter from (Web Crypto's
 * `getRandomValues`, which the WebAssembly calls through `globalThis.crypto`) replaced by a stream seeded from `key`.
 * Building a template twice therefore gives the same document, ids included, so `pnpm seed --fixtures` only
 * rewrites the fixtures whose templates changed.
 */
function withSeededIds<T>(key: string, fn: () => T): T {
  const c = globalThis.crypto;
  const own = Object.getOwnPropertyDescriptor(c, 'getRandomValues');
  let s = createHash('sha256').update(`template:${key}`).digest().readUInt32LE(0);
  // Mulberry32.
  const next = () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) & 0xff;
  };
  Object.defineProperty(c, 'getRandomValues', {
    configurable: true,
    value: <A extends ArrayBufferView | null>(a: A): A => {
      if (a) {
        const u8 = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
        for (let i = 0; i < u8.length; i++) u8[i] = next();
      }
      return a;
    },
  });
  try {
    return fn();
  } finally {
    if (own) Object.defineProperty(c, 'getRandomValues', own);
    else delete (c as { getRandomValues?: unknown }).getRandomValues;
  }
}

export function buildRegistered(filter?: (p: { id: string; category: string; preset: string }) => boolean): TemplateDef[] {
  return PENDING.filter((p) => !filter || filter(p)).map((p) => withSeededIds(p.id, () => {
    const b = new B(p.preset, p.title, p.pages);
    p.build(b);
    return { id: p.id, title: p.title, category: p.category, preset: p.preset, tags: p.tags, doc: b.doc };
  }));
}
