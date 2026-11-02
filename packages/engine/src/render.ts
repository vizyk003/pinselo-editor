// SPDX-License-Identifier: Apache-2.0
/**
 * The renderer (crates/oc-render through crates/oc-wasm `render`): what apps/web used from `@oc/render-core` and
 * `@oc/render-canvaskit`, without CanvasKit.
 *
 * * `resolvePage(doc, pageId, opts)` → a {@link DisplayList} held in Rust (its ops cross the boundary only when
 *   read: `items`, `background`, `assets`).
 * * {@link RenderTarget}: a rectangle of a device (a tile of the editor canvas, a thumbnail…) painted by
 *   tiny-skia; `end()` returns `ImageData` for `putImageData`. Coordinates are the device's, so the same scene
 *   painted tile by tile is bit-identical to painting it at once, and a tile repaints exactly.
 * * {@link Painter}: draws display lists onto a target with the fonts of a {@link FontRegistry} (the editor's
 *   own text engine: caret geometry and drawing share one layout), loads image assets on demand
 *   (`onInvalidate` when one arrives) and video frames from `videoFrames`.
 * * The pure helpers: `smoothPath`, `sliceColor`, `pageSettleMs`, `clickStepCount`, `transitionProgress`,
 *   `transitionFrame`, `tableGeometry` (schema), and {@link drawTransition}.
 */
import type { AnimDirection, AssetId, BlendMode, ChartEl, ColorValue, DesignDoc, Effect, Element, ElementId, ImageEditStack, LineMarker, Mat2D, Page, PageId, Rect, Stroke, TextBody, TextEffect, TransitionKind } from './types/schema';
import { docRef } from './docref';
import type { FontRegistry } from './text';
import type { TextOutline } from './vector';
import { raw } from './wasm';

// ------------------------------------------------------------------------------------------------ types

export type ResolvedPaint =
  | { kind: 'solid'; color: ColorValue }
  | { kind: 'linear'; stops: Array<{ t: number; color: ColorValue }>; angle: number; box: Rect }
  | { kind: 'radial'; stops: Array<{ t: number; color: ColorValue }>; box: Rect }
  | { kind: 'image'; asset: AssetId; fit: 'fill' | 'fit' | 'tile'; box: Rect };

export type DLItem =
  | { op: 'save'; matrix: Mat2D }
  | { op: 'restore' }
  | { op: 'layer'; opacity: number; blend: BlendMode; effects: Effect[]; srcEl: ElementId }
  | { op: 'clip'; path: string }
  | { op: 'mask'; mode: 'alpha' | 'luminance'; srcEl: ElementId }
  | { op: 'maskContent' }
  | { op: 'path'; path: string; fills: ResolvedPaint[]; stroke?: { stroke: Stroke; paint: ResolvedPaint }; srcEl: ElementId }
  | { op: 'line'; points: Array<[number, number]>; stroke: Stroke; paint: ResolvedPaint; startMarker: LineMarker; endMarker: LineMarker; srcEl: ElementId }
  | { op: 'image'; asset: AssetId; dst: Rect; crop: Rect; edits?: ImageEditStack; clip?: string; srcEl: ElementId }
  | { op: 'text'; key: string; body: TextBody; box: Rect; effect?: TextEffect; valign?: 'top' | 'middle' | 'bottom'; curve?: number; fit?: 'shrink'; srcEl: ElementId }
  | { op: 'qr'; modules: number[]; size: number; box: Rect; fg: ColorValue; bg: ColorValue; srcEl: ElementId }
  | { op: 'placeholder'; box: Rect; label: string; srcEl: ElementId }
  | { op: 'video'; asset: AssetId; dst: Rect; crop: Rect; sourceTimeMs: number | null; clip?: string; key?: string; srcEl: ElementId; edits?: ImageEditStack };

export interface ResolveOptions {
  /** Transient per-element overrides (drag previews) merged over the document. */
  overrides?: ReadonlyMap<ElementId, Partial<Element>>;
  /** Elements to skip (e.g. hidden while being cropped). */
  skip?: ReadonlySet<ElementId>;
  /** Skip the page background (transparent PNG export). */
  transparent?: boolean;
  /** Page time in ms for animated output (Present, video export, preview). Omitted = settled state. */
  timeMs?: number;
  /** Design time (ms) for the caption overlay; omitted = no captions. */
  captionTimeMs?: number;
  /** Present mode: page time at which each click build step happened (steps not yet clicked stay hidden). */
  clickSteps?: number[];
  /** Text being edited: drawn flat (not curved) so the caret lines up. */
  editingText?: ElementId | null;
}

export interface TransitionLayer {
  alpha: number; dx: number; dy: number; scale: number; rot: number; sx: number; blur: number;
  /** Visible region of this layer, page fractions. */
  clip?: { kind: 'rect'; x: number; y: number; w: number; h: number } | { kind: 'circle'; r: number };
}
export interface TransitionFrame { prev: TransitionLayer | null; next: TransitionLayer; nextFirst?: boolean }

// ------------------------------------------------------------------------------------------------ display lists

/** A resolved page. Ops live in Rust; `items` / `background` / `assets` read them (once) when asked. */
export class DisplayList {
  readonly width: number;
  readonly height: number;
  private _js: { background: ResolvedPaint[]; items: DLItem[]; assets: AssetId[] } | null = null;

  constructor(readonly raw: raw.DisplayList) {
    this.width = raw.width;
    this.height = raw.height;
  }

  private js() {
    return (this._js ??= this.raw.toJs() as { background: ResolvedPaint[]; items: DLItem[]; assets: AssetId[] });
  }

  get background(): ResolvedPaint[] {
    return this.js().background;
  }

  get items(): DLItem[] {
    return this.js().items;
  }

  /** Assets referenced, in first-use order. */
  get assets(): Set<AssetId> {
    return new Set(this.raw.assets as AssetId[]);
  }
}

function resolveArgs(o: ResolveOptions): object | null {
  const out: Record<string, unknown> = {};
  let any = false;
  for (const [k, v] of Object.entries(o)) {
    if (v === undefined || v === null) continue;
    any = true;
    if (k === 'overrides') out[k] = Object.fromEntries(v as ReadonlyMap<string, unknown>);
    else if (k === 'skip') out[k] = [...(v as ReadonlySet<string>)];
    else out[k] = v;
  }
  return any ? out : null;
}

/** `resolvePage`: a page of a design as a display list. */
export function resolvePage(doc: DesignDoc, pageId: PageId, opts: ResolveOptions = {}): DisplayList {
  return new DisplayList(raw.resolvePage(docRef(doc), pageId, resolveArgs(opts)));
}

// ------------------------------------------------------------------------------------------------ pure helpers

/** Smooth a polyline through midpoints with quadratic segments. */
export const smoothPath = (pts: Array<[number, number]>): string => raw.smoothPath(pts);

/** SVG import as editable vectors (DS-100): the SVG's paths as path elements in the pixel space of a `w × h` image
 *  element showing it through `crop` (normalised), or null when nothing converts. */
export const svgToElements = (svg: string, w: number, h: number, crop?: Rect): Element[] | null => raw.svgToElements(svg, w, h, crop) as Element[] | null;

/** Colour of slice/bar `i` of a chart. */
export const sliceColor = (el: ChartEl, i: number): ColorValue => raw.sliceColor(el, i) as ColorValue;

/** When the page's entrance/emphasis animations have settled (ms). */
export const pageSettleMs = (page: Page, els: readonly Element[]): number => raw.pageSettleMs(page, els);

/** Number of click build steps on a page (0 = none). */
export const clickStepCount = (els: readonly Element[]): number => raw.clickStepCount(els);

/** Transition overlay progress for a page that started `t` ms ago (1 = fully visible). */
export const transitionProgress = (page: Page, t: number): number => raw.transitionProgress(page, t);

/** Layers at eased progress `p` (0..1) of a transition. */
export const transitionFrame = (kind: TransitionKind, p: number, direction?: AnimDirection): TransitionFrame => raw.transitionFrame(kind, p, direction ?? null) as TransitionFrame;

// ------------------------------------------------------------------------------------------------ render target

/** The canvas calls {@link drawTransition} needs (a {@link RenderTarget}, or {@link canvas2dTransitionTarget}). */
export interface TransitionCanvas {
  save(): void;
  restore(): void;
  translate(x: number, y: number): void;
  rotate(deg: number): void;
  scale(sx: number, sy: number): void;
  clipRect(x: number, y: number, w: number, h: number): void;
  clipCircle(cx: number, cy: number, r: number): void;
  /** Push a layer composited with `alpha` and a Gaussian blur of σ = `blur` (local units); `restore` pops it. */
  saveLayer(alpha: number, blur: number): void;
}

/**
 * A rectangle of a device painted by tiny-skia. `begin(x, y, w, h)` starts it (device coordinates, identity
 * transform); drawing calls use the device's coordinates; `end()` returns the rectangle's pixels.
 */
export class RenderTarget implements TransitionCanvas {
  readonly raw = new raw.RenderTarget();
  private rect = { x: 0, y: 0, w: 0, h: 0 };

  begin(x: number, y: number, w: number, h: number): this {
    this.rect = { x: Math.round(x), y: Math.round(y), w: Math.max(1, Math.round(w)), h: Math.max(1, Math.round(h)) };
    this.raw.begin(this.rect.x, this.rect.y, this.rect.w, this.rect.h);
    return this;
  }

  /** The painted pixels (unpremultiplied RGBA). */
  end(): ImageData {
    const px = this.raw.end();
    return imageData(px, this.rect.w, this.rect.h);
  }

  /** The inner `(x, y, w, h)` (relative to the begun rectangle) of what was painted. */
  endCrop(x: number, y: number, w: number, h: number): ImageData {
    return imageData(this.raw.endCrop(x, y, w, h), w, h);
  }

  get active(): boolean {
    return this.raw.active;
  }

  /** A copy of the painted pixels `(x, y, w, h)` (relative to the begun rectangle), to {@link compose} later. */
  snapshot(x: number, y: number, w: number, h: number): raw.Snapshot {
    return this.raw.snapshot(x, y, w, h);
  }

  /** Composite a snapshot at `(x, y)` (relative to the begun rectangle). No save or clip may be in effect. */
  compose(snap: raw.Snapshot, x: number, y: number): void {
    this.raw.compose(snap, x, y);
  }

  save(): void {
    this.raw.save();
  }

  restore(): void {
    this.raw.restore();
  }

  translate(x: number, y: number): void {
    this.raw.translate(x, y);
  }

  scale(sx: number, sy = sx): void {
    this.raw.scale(sx, sy);
  }

  rotate(deg: number): void {
    this.raw.rotate(deg);
  }

  clipRect(x: number, y: number, w: number, h: number): void {
    this.raw.clipRect(x, y, w, h);
  }

  clipCircle(cx: number, cy: number, r: number): void {
    this.raw.clipCircle(cx, cy, r);
  }

  saveLayer(alpha: number, blur = 0): void {
    this.raw.saveLayer(alpha, blur);
  }

  /** Fill everything (inside the clip) with a colour, replacing what is there. Components 0..1. */
  clear(r: number, g: number, b: number, a = 1): void {
    this.raw.clear(r, g, b, a);
  }

  fillRect(x: number, y: number, w: number, h: number, [r, g, b, a]: readonly [number, number, number, number]): void {
    this.raw.fillRect(x, y, w, h, r, g, b, a);
  }

  /**
   * A blurred rectangle (soft shadow), σ = `sigma` local units. Pixels inside `hole` (an opaque page drawn over
   * it) are skipped.
   */
  shadowRect(x: number, y: number, w: number, h: number, sigma: number, [r, g, b, a]: readonly [number, number, number, number], hole?: { x: number; y: number; w: number; h: number }): void {
    this.raw.shadowRect(x, y, w, h, sigma, r, g, b, a, hole?.x ?? 0, hole?.y ?? 0, hole?.w ?? 0, hole?.h ?? 0);
  }

}

function imageData(px: Uint8ClampedArray, w: number, h: number): ImageData {
  if (typeof ImageData !== 'undefined') return new ImageData(px as Uint8ClampedArray<ArrayBuffer>, w, h);
  // Node (tests): a structural stand-in.
  return { data: px, width: w, height: h, colorSpace: 'srgb' } as unknown as ImageData;
}

// ------------------------------------------------------------------------------------------------ transitions

/**
 * Draw one transition frame (`transitionFrame`) on a `w × h` page: offsets, scale, flip, rotation, clip, blur
 * and alpha around `drawPage(which, c)`, which draws a page's content in page coordinates. Same layering as
 * oc-render's `draw_transition` (and the CanvasKit `drawTransition` before it).
 */
export function drawTransition<C extends TransitionCanvas>(c: C, w: number, h: number, f: TransitionFrame, drawPage: (which: 'prev' | 'next', c: C) => void): void {
  const layer = (which: 'prev' | 'next', l: TransitionLayer | null) => {
    if (!l || l.alpha <= 0.001) return;
    c.save();
    c.translate(l.dx * w + w / 2, l.dy * h + h / 2);
    if (l.rot) c.rotate(l.rot);
    c.scale(l.scale * l.sx, l.scale);
    c.translate(-w / 2, -h / 2);
    if (l.clip?.kind === 'rect') c.clipRect(l.clip.x * w, l.clip.y * h, l.clip.w * w, l.clip.h * h);
    if (l.clip?.kind === 'circle') c.clipCircle(w / 2, h / 2, l.clip.r * Math.hypot(w, h));
    const layered = l.alpha < 1 || l.blur > 0.1;
    if (layered) c.saveLayer(Math.max(0, Math.min(1, l.alpha)), l.blur > 0.1 ? l.blur : 0);
    drawPage(which, c);
    if (layered) c.restore();
    c.restore();
  };
  if (f.nextFirst) {
    layer('next', f.next);
    layer('prev', f.prev);
  } else {
    layer('prev', f.prev);
    layer('next', f.next);
  }
}

/**
 * {@link TransitionCanvas} over a 2D context, for compositing pre-rendered page bitmaps on the GPU (Present mode):
 * `saveLayer` becomes the alpha and blur filter of what `drawPage` draws next (one image per layer).
 */
export function canvas2dTransitionTarget(ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, deviceScale: number): TransitionCanvas & { ctx: typeof ctx } {
  let scale = deviceScale;
  const scales: number[] = [];
  return {
    ctx,
    save: () => {
      scales.push(scale);
      ctx.save();
    },
    restore: () => {
      scale = scales.pop() ?? scale;
      ctx.restore();
    },
    translate: (x, y) => ctx.translate(x, y),
    rotate: (deg) => ctx.rotate((deg * Math.PI) / 180),
    scale: (sx, sy) => {
      scale *= Math.max(Math.abs(sx), Math.abs(sy));
      ctx.scale(sx, sy);
    },
    clipRect: (x, y, w, h) => {
      ctx.beginPath();
      ctx.rect(x, y, w, h);
      ctx.clip();
    },
    clipCircle: (cx, cy, r) => {
      ctx.beginPath();
      ctx.arc(cx, cy, Math.max(0, r), 0, Math.PI * 2);
      ctx.clip();
    },
    saveLayer: (alpha, blur) => {
      scales.push(scale);
      ctx.save();
      ctx.globalAlpha *= alpha;
      if (blur > 0) ctx.filter = `blur(${blur * scale}px)`;
    },
  };
}

// ------------------------------------------------------------------------------------------------ painter

/** Fetches an asset's encoded bytes (`<id>`, `<id>/poster`, `<id>/recolor/…`). */
export type AssetSource = (asset: AssetId) => Promise<Uint8Array | ArrayBuffer>;

/** The current frame of a playback: a `<video>` (or any image source), its pixels, or `null` (poster). */
export type VideoFrameSource = (asset: AssetId, sourceTimeMs: number, key: string) => CanvasImageSource | ImageData | null;

/** Longest side video frames are sampled at (they are drawn into a page, rarely larger). */
const VIDEO_MAX = 1280;

export class Painter {
  readonly raw: raw.Painter;
  private readonly listeners = new Set<() => void>();
  private readonly loading = new Set<string>();
  /** Set by the host (browser video pool). Without it videos show their poster. */
  videoFrames: VideoFrameSource | null = null;
  private frameStamp = new Map<string, { at: number; t: number }>();
  private scratch: OffscreenCanvas | HTMLCanvasElement | null = null;

  constructor(readonly text: { fonts: FontRegistry }, private readonly assets: AssetSource) {
    this.raw = new raw.Painter(text.fonts.service.raw);
    text.fonts.onLoad(() => this.invalidate());
  }

  /** A text element's glyphs as a path in its own box (DS-100), or null when its fonts are not loaded yet. */
  textOutline(doc: DesignDoc, id: ElementId): TextOutline | null {
    return this.raw.textOutline(docRef(doc), id) as TextOutline | null;
  }

  onInvalidate(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private invalidate() {
    for (const l of this.listeners) l();
  }

  /**
   * Draw `dl` on `c` (its transform maps page space to device pixels). `parts` draws only the background or
   * only the items.
   */
  draw(c: RenderTarget, dl: DisplayList, parts: { background?: boolean; items?: boolean } = {}): void {
    if (this.videoFrames) this.feedVideo(dl);
    this.raw.draw(c.raw, dl.raw, parts.background ?? true, parts.items ?? true);
    this.afterDraw();
  }

  /**
   * How `next` can be painted in layers while its groups `from..to` change against `prev` (a drag, typing): the
   * background and the groups below, then the changing groups, then the groups above. `below` / `above` identify
   * the content of the unchanged layers (cache keys); `reach` is the filter reach (page units) of the changing
   * groups. `null` when the change cannot be layered.
   */
  layering(prev: DisplayList, next: DisplayList): { from: number; to: number; below: string; above: string; reach: number } | null {
    return this.raw.layering(prev.raw, next.raw) as { from: number; to: number; below: string; above: string; reach: number } | null;
  }

  /** Draw the groups `from..to` of `dl` (and its background when `background`), see {@link layering}. */
  drawGroups(c: RenderTarget, dl: DisplayList, from: number, to: number, background = false): void {
    if (this.videoFrames) this.feedVideo(dl);
    this.raw.drawGroups(c.raw, dl.raw, from, to, background);
    this.afterDraw();
  }

  /**
   * The page-space offset `[dx, dy]` by which groups `nextRange` of `next` are groups `prevRange` of `prev` moved
   * (a drag: the same ops, translated), `null` otherwise.
   */
  translation(prev: DisplayList, prevRange: [number, number], next: DisplayList, nextRange: [number, number]): [number, number] | null {
    const t = this.raw.translation(prev.raw, prevRange[0], prevRange[1], next.raw, nextRange[0], nextRange[1]);
    return t ? [t[0]!, t[1]!] : null;
  }

  /** Page-space bounds of the groups that draw video. */
  videoBounds(dl: DisplayList): Array<{ x: number; y: number; w: number; h: number }> {
    const v = this.raw.videoBounds(dl.raw);
    const out = [];
    for (let i = 0; i + 3 < v.length; i += 4) out.push({ x: v[i]!, y: v[i + 1]!, w: v[i + 2]!, h: v[i + 3]! });
    return out;
  }

  /** Page-space bounds of groups `from..to` (`null`: they draw nothing). */
  groupsBounds(dl: DisplayList, from: number, to: number): { x: number; y: number; w: number; h: number } | null {
    const b = this.raw.groupsBounds(dl.raw, from, to);
    return b ? { x: b[0]!, y: b[1]!, w: b[2]!, h: b[3]! } : null;
  }

  /** Number of element groups of a list. */
  groupCount(dl: DisplayList): number {
    return this.raw.groupCount(dl.raw);
  }

  /** A transition frame of two pages, drawn by oc-render (what video export draws). */
  drawTransitionFrame(c: RenderTarget, w: number, h: number, f: TransitionFrame, prev: DisplayList | null, next: DisplayList): void {
    if (this.videoFrames) this.feedVideo(next);
    this.raw.drawTransition(c.raw, w, h, f, prev ? prev.raw.share() : undefined, next.raw);
    this.afterDraw();
  }

  /** Render a whole page into an image (`scale` device pixels per px), over `background` (RGBA 0..1) if given. */
  renderPage(dl: DisplayList, scale: number, background?: readonly [number, number, number, number]): ImageData {
    const t = new RenderTarget().begin(0, 0, Math.round(dl.width * scale), Math.round(dl.height * scale));
    if (background) t.clear(...background);
    t.scale(scale);
    this.draw(t, dl);
    return t.end();
  }

  /** Page-space area that differs between two lists of the same page (`null`: none; `'full'`: everything). */
  dirtyRegion(prev: DisplayList, next: DisplayList): Rect | 'full' | null {
    const r = this.raw.dirtyRegion(prev.raw, next.raw) as Rect | 'full' | null;
    this.afterDraw();
    return r;
  }

  /** How far (page units) the list's filters reach (margin for tiled painting). */
  filterReach(dl: DisplayList): number {
    return this.raw.filterReach(dl.raw);
  }

  private afterDraw() {
    this.text.fonts.pump();
    const missing = this.raw.takeMissingAssets();
    for (const key of missing) this.load(key as AssetId);
  }

  private load(key: AssetId): Promise<void> {
    if (this.loading.has(key)) return Promise.resolve();
    this.loading.add(key);
    return this.assets(key)
      .then((buf) => {
        this.raw.addAsset(key, buf instanceof Uint8Array ? buf : new Uint8Array(buf));
      })
      .catch(() => this.raw.assetFailed(key))
      .finally(() => {
        this.loading.delete(key);
        this.invalidate();
      });
  }

  /** Decoded size of an asset whose bytes are in (starts loading it otherwise). */
  imageSize(asset: AssetId): { w: number; h: number } | null {
    const s = this.raw.imageSize(asset);
    if (!s) {
      if (!this.raw.hasAsset(asset)) void this.load(asset);
      return null;
    }
    return { w: s[0]!, h: s[1]! };
  }

  /** Load every image and font a display list needs (export path). */
  async preload(dl: DisplayList): Promise<void> {
    const jobs: Promise<unknown>[] = [];
    for (const a of dl.raw.assets as AssetId[]) if (!this.raw.hasAsset(a)) jobs.push(this.load(a));
    for (const it of dl.items) if (it.op === 'text') for (const f of this.text.fonts.service.missingFonts(it.body)) jobs.push(this.text.fonts.loadFile(f).catch(() => {}));
    await Promise.all(jobs);
  }

  private feedVideo(dl: DisplayList) {
    const vids = dl.raw.videos() as Array<{ asset: AssetId; key: string; sourceTimeMs: number }>;
    if (!vids.length) return;
    const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
    for (const v of vids) {
      const last = this.frameStamp.get(v.key);
      // One sample per animation frame (a frame is drawn tile by tile) unless the requested time moved.
      if (last && now - last.at < 8 && last.t === v.sourceTimeMs) continue;
      this.frameStamp.set(v.key, { at: now, t: v.sourceTimeMs });
      const src = this.videoFrames?.(v.asset, v.sourceTimeMs, v.key) ?? null;
      const px = src ? this.pixels(src) : null;
      if (px) this.raw.setVideoFrame(v.key, px.width, px.height, new Uint8Array(px.data.buffer, px.data.byteOffset, px.data.byteLength));
      else this.raw.clearVideoFrame(v.key);
    }
  }

  private pixels(src: CanvasImageSource | ImageData): ImageData | null {
    if ((typeof ImageData !== 'undefined' && src instanceof ImageData) || ((src as ImageData).data instanceof Uint8ClampedArray && typeof (src as ImageData).width === 'number')) return src as ImageData;
    const s = src as { videoWidth?: number; videoHeight?: number; width?: number | SVGAnimatedLength; height?: number | SVGAnimatedLength };
    const w0 = s.videoWidth || (typeof s.width === 'number' ? s.width : 0);
    const h0 = s.videoHeight || (typeof s.height === 'number' ? s.height : 0);
    if (!w0 || !h0) return null;
    const k = Math.min(1, VIDEO_MAX / Math.max(w0, h0));
    const w = Math.max(1, Math.round(w0 * k));
    const h = Math.max(1, Math.round(h0 * k));
    if (!this.scratch) this.scratch = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(w, h) : Object.assign(document.createElement('canvas'), { width: w, height: h });
    if (this.scratch.width !== w || this.scratch.height !== h) {
      this.scratch.width = w;
      this.scratch.height = h;
    }
    const ctx = this.scratch.getContext('2d', { willReadFrequently: true }) as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
    if (!ctx) return null;
    try {
      ctx.drawImage(src as CanvasImageSource, 0, 0, w, h);
      return ctx.getImageData(0, 0, w, h);
    } catch {
      return null; // tainted or not ready
    }
  }

  /** Forget decoded images and video frames (asset bytes stay). */
  dispose(): void {
    this.raw.clearImages();
    this.raw.clearVideoFrame(undefined);
  }
}
