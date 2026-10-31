// SPDX-License-Identifier: Apache-2.0
/**
 * The renderer facade (`resolvePage`, `Painter`, `RenderTarget`, the `@oc/render-core` helpers) and the font
 * loader:
 *
 * - display lists and helpers against tests/golden/render (written from the TS `@oc/render-core`);
 * - painting: a page painted tile by tile equals the page painted at once; a partial repaint of the dirty tiles
 *   equals the full repaint; images load on demand through the asset source (once) and repaint; video frames fed
 *   from the host replace the poster; the TS `drawTransition` layering equals oc-render's;
 * - fonts: drawing and measuring report missing faces, the registry fetches each once, registers it and
 *   notifies; text re-flows (the editor re-measures auto-height text) exactly as with the face preloaded;
 * - caret geometry comes from the layout that is drawn: the caret at the end of a line sits on the drawn glyphs'
 *   right edge.
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import {
  Editor, FontRegistry, Painter, RenderTarget, TextEngine, clickStepCount, drawTransition, emptyDesign, encodeDesign, makeImage, makeText, makeVideo, pageSettleMs, resolvePage,
  sliceColor, smoothPath, sortedPages, transitionFrame, transitionProgress, type AssetId, type DesignDoc, type FontManifest, type PageId, type TextBody, type TextEl,
  type TransitionKind,
} from '../src/index';
import { ROOT, diff, fixture, golden, readJson } from './helpers';

const FILES = join(ROOT, 'fixtures/fonts/files');
const manifest = (): FontManifest => readJson<FontManifest>('fixtures/fonts/manifest.json');
const ASSETS = join(ROOT, 'crates/oc-render/tests/assets');

/** A registry loading from fixtures/fonts/files, counting requests per file. */
function registry(delay = 0) {
  const calls = new Map<string, number>();
  const fonts = new FontRegistry(manifest(), async (file) => {
    calls.set(file, (calls.get(file) ?? 0) + 1);
    if (delay) await new Promise((r) => setTimeout(r, delay));
    return readFileSync(join(FILES, file));
  });
  return { fonts, calls };
}

const settle = (fonts: FontRegistry) => fonts.idle();

/** A new document object with `el` added (documents are immutable to the facade: one parse per object). */
const withEl = <D extends DesignDoc>(doc: D, el: { id: string }): D => ({ ...doc, elements: { ...doc.elements, [el.id]: el } });

/**
 * The goldens store SVG path data with coordinates rounded to 1e-6 (tools/golden/render.ts `canonPath`: the last
 * digit of Math results differs between Node versions); the output is compared the same way (like the Rust tests).
 */
const PATH_KEYS = new Set(['path', 'clip', 'clipPath', 'd', '']);
const PATH_DATA = /^\s*[Mm][MmLlHhVvCcSsQqTtAaZz0-9.eE+\-,\s]*$/;
const PATH_NUM = /-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?/g;
const canonPath = (d: string) => d.replace(PATH_NUM, (m) => { const r = Math.round(Number(m) * 1e6) / 1e6; return String(r === 0 ? 0 : r); });
const canon = (v: unknown): unknown => JSON.parse(JSON.stringify(v, (k, x) => (typeof x === 'string' && PATH_KEYS.has(k) && PATH_DATA.test(x) ? canonPath(x) : x)));

const jsDl = (dl: ReturnType<typeof resolvePage>) => canon({ width: dl.width, height: dl.height, background: dl.background, items: dl.items, assets: [...dl.assets] });

describe('display lists and helpers (TS goldens)', () => {
  test('settled template pages', () => {
    const cases = golden<Array<{ file: string; page: string; dl: unknown }>>('render', 'templates');
    let n = 0;
    for (const c of cases) {
      const d = diff(jsDl(resolvePage(fixture(c.file) as DesignDoc, c.page as PageId)), c.dl);
      expect(d, `${c.file} ${c.page}`).toBeNull();
      n++;
    }
    expect(n).toBeGreaterThanOrEqual(300);
  });

  test('animated pages (times, click steps)', () => {
    const cases = golden<Array<{ file: string; page: string; patch: { page: object; elements: Record<string, object> }; clickSteps: number[] | null; cases: Array<{ timeMs: number; dl: unknown }> }>>('render', 'animated');
    for (const c of cases) {
      const doc = structuredClone(fixture(c.file)) as any;
      Object.assign(doc.pages[c.page], c.patch.page);
      for (const [id, p] of Object.entries(c.patch.elements)) Object.assign(doc.elements[id], p);
      for (const k of c.cases) {
        const dl = resolvePage(doc, c.page as PageId, { timeMs: k.timeMs, ...(c.clickSteps ? { clickSteps: c.clickSteps } : {}) });
        expect(diff(jsDl(dl), k.dl), `${c.file} t=${k.timeMs}`).toBeNull();
      }
    }
  });

  test('helpers', () => {
    const cases = golden<Array<{ fn: string; args: any[]; out: unknown }>>('render', 'units');
    const fns: Record<string, (...a: any[]) => unknown> = {
      smoothPath, sliceColor, pageSettleMs, clickStepCount, transitionProgress,
      transitionFrame: (k: TransitionKind, p: number, d?: any) => transitionFrame(k, p, d ?? undefined),
    };
    let n = 0;
    for (const c of cases) {
      const f = Object.hasOwn(fns, c.fn) ? fns[c.fn] : undefined;
      if (!f) continue;
      expect(diff(canon(f(...c.args)), c.out), `${c.fn}(${JSON.stringify(c.args).slice(0, 200)})`).toBeNull();
      n++;
    }
    expect(n).toBeGreaterThan(100);
  });
});

/** Paint `dl` tile by tile (the editor's way) into one RGBA buffer. */
function paintTiles(p: Painter, dl: ReturnType<typeof resolvePage>, scale: number, tile = 64, only?: { x: number; y: number; w: number; h: number }, into?: Uint8ClampedArray) {
  const W = Math.round(dl.width * scale);
  const H = Math.round(dl.height * scale);
  const out = into ?? new Uint8ClampedArray(W * H * 4);
  const m = Math.ceil((p.filterReach(dl) * scale + 2) / 16) * 16;
  const t = new RenderTarget();
  for (let ty = 0; ty < H; ty += tile) {
    for (let tx = 0; tx < W; tx += tile) {
      const tw = Math.min(tile, W - tx);
      const th = Math.min(tile, H - ty);
      if (only) {
        const x0 = Math.floor(only.x * scale) - 1, y0 = Math.floor(only.y * scale) - 1;
        const x1 = Math.ceil((only.x + only.w) * scale) + 1, y1 = Math.ceil((only.y + only.h) * scale) + 1;
        if (x1 <= tx || y1 <= ty || x0 >= tx + tw || y0 >= ty + th) continue;
      }
      t.begin(tx - m, ty - m, tile + 2 * m, tile + 2 * m);
      t.clear(1, 1, 1, 1);
      t.scale(scale);
      p.draw(t, dl);
      const img = t.endCrop(m, m, tw, th);
      for (let y = 0; y < th; y++) out.set(img.data.subarray(y * tw * 4, (y + 1) * tw * 4), ((ty + y) * W + tx) * 4);
    }
  }
  return out;
}

const countDiff = (a: Uint8ClampedArray, b: Uint8ClampedArray, tol = 0) => {
  let n = 0;
  for (let i = 0; i < a.length; i += 4) if (Math.abs(a[i]! - b[i]!) > tol || Math.abs(a[i + 1]! - b[i + 1]!) > tol || Math.abs(a[i + 2]! - b[i + 2]!) > tol || Math.abs(a[i + 3]! - b[i + 3]!) > tol) n++;
  return n;
};

describe.skipIf(!existsSync(FILES))('painter', () => {
  test('tiles paint the page; dirty tiles repaint it exactly', async () => {
    const { fonts } = registry();
    const p = new Painter(new TextEngine(fonts), async () => { throw new Error('no assets'); });
    for (const file of ['deck-education-warm.json', 'card-business-clean.json', 'ig-product-launch.json']) {
      const doc = fixture(file) as DesignDoc;
      const page = sortedPages(doc)[0]!;
      // Fonts load on first draw (the painter reports them), then the page is stable.
      p.renderPage(resolvePage(doc, page.id), 0.5);
      await settle(fonts);
      const dl = resolvePage(doc, page.id);
      const scale = Math.min(1, 400 / Math.max(dl.width, dl.height));
      const whole = p.renderPage(dl, scale, [1, 1, 1, 1]).data;
      const tiled = paintTiles(p, dl, scale);
      // Same pixels up to edge rasterisation where curves cross a tile (a few levels on a few pixels).
      expect(countDiff(whole, tiled, 8)).toBeLessThan(whole.length / 4 / 200);

      const moved = structuredClone(doc) as any;
      const id = Object.values(moved.elements as Record<string, any>).find((e) => e.parentId === page.id && e.type !== 'group')!.id;
      moved.elements[id].xf.x += 41;
      const dl2 = resolvePage(moved, page.id);
      const region = p.dirtyRegion(dl, dl2);
      expect(region).not.toBeNull();
      expect(region).not.toBe('full');
      const patched = paintTiles(p, dl2, scale, 64, region as { x: number; y: number; w: number; h: number }, tiled.slice());
      expect(countDiff(patched, paintTiles(p, dl2, scale))).toBe(0);
      expect(p.dirtyRegion(dl2, resolvePage(moved, page.id))).toBeNull();
    }
  });

  test('images load once through the asset source and repaint', async () => {
    const { fonts } = registry();
    const loads: string[] = [];
    const p = new Painter(new TextEngine(fonts), async (a) => {
      loads.push(a);
      return readFileSync(join(ASSETS, 'photo.jpg'));
    });
    let invalidated = 0;
    p.onInvalidate(() => invalidated++);
    let doc = emptyDesign({ w: 200, h: 100 });
    const page = sortedPages(doc)[0]!.id;
    const img = makeImage({ parentId: page, asset: 'photo1' as AssetId, xf: { x: 0, y: 0, w: 200, h: 100, rot: 0 } });
    doc = withEl(doc, img);
    const dl = resolvePage(doc, page);
    const before = p.renderPage(dl, 1, [1, 1, 1, 1]).data;
    p.renderPage(dl, 1, [1, 1, 1, 1]);
    expect(loads).toEqual(['photo1']);
    await new Promise((r) => setTimeout(r, 5));
    expect(invalidated).toBe(1);
    const after = p.renderPage(dl, 1, [1, 1, 1, 1]).data;
    expect(countDiff(before, after, 8)).toBeGreaterThan(200 * 100 * 0.5);
    expect(p.imageSize('photo1' as AssetId)).not.toBeNull();
    expect(loads).toEqual(['photo1']);
  });

  test('video frames from the host replace the poster', () => {
    const { fonts } = registry();
    const p = new Painter(new TextEngine(fonts), async () => { throw new Error('none'); });
    const red = { data: new Uint8ClampedArray(16 * 9 * 4).map((_, i) => (i % 4 === 0 || i % 4 === 3 ? 255 : 0)), width: 16, height: 9 } as unknown as ImageData;
    const seen: Array<[string, number, string]> = [];
    let doc = emptyDesign({ w: 160, h: 90 });
    const page = sortedPages(doc)[0]!.id;
    const v = makeVideo({ parentId: page, asset: 'clip1' as AssetId, durationMs: 4000, xf: { x: 0, y: 0, w: 160, h: 90, rot: 0 } });
    doc = withEl(doc, v);
    const at = (t?: number) => resolvePage(doc, page, t === undefined ? {} : { timeMs: t });
    const center = (img: ImageData) => [...img.data.subarray((45 * 160 + 80) * 4, (45 * 160 + 80) * 4 + 4)];
    // Without a source: the poster (a dark placeholder while it does not load).
    const poster = center(p.renderPage(at(1000), 1, [1, 1, 1, 1]));
    p.videoFrames = (asset, t, key) => {
      seen.push([asset, t, key]);
      return red;
    };
    expect(center(p.renderPage(at(1000), 1, [1, 1, 1, 1]))).toEqual([255, 0, 0, 255]);
    expect(seen).toEqual([['clip1', 1000, v.id]]);
    // Static renders (no time) never ask for frames.
    p.renderPage(at(), 1, [1, 1, 1, 1]);
    expect(seen.length).toBe(1);
    // No frame: back to the poster.
    p.videoFrames = () => null;
    expect(center(p.renderPage(at(1500), 1, [1, 1, 1, 1]))).toEqual(poster);
  });

  test('drawTransition (TS layering) equals oc-render draw_transition', async () => {
    const { fonts } = registry();
    const p = new Painter(new TextEngine(fonts), async () => { throw new Error('none'); });
    const doc = fixture('deck-education-warm.json') as DesignDoc;
    const [a, b] = sortedPages(doc);
    const prev = resolvePage(doc, a!.id);
    const next = resolvePage(doc, b!.id);
    p.renderPage(prev, 0.25);
    p.renderPage(next, 0.25);
    await settle(fonts);
    const { w, h } = doc.meta.dims;
    const s = 160 / w;
    for (const kind of ['fade', 'dissolve', 'slide', 'push', 'cover', 'reveal', 'zoom', 'wipe', 'iris', 'split', 'flip', 'blur', 'spin', 'drop'] as TransitionKind[]) {
      const f = transitionFrame(kind, 0.4, 'left');
      const t1 = new RenderTarget().begin(0, 0, w * s, h * s);
      t1.clear(0, 0, 0, 1);
      t1.scale(s);
      p.drawTransitionFrame(t1, w, h, f, prev, next);
      const t2 = new RenderTarget().begin(0, 0, w * s, h * s);
      t2.clear(0, 0, 0, 1);
      t2.scale(s);
      drawTransition(t2, w, h, f, (which, c) => p.draw(c, which === 'prev' ? prev : next));
      expect(countDiff(t1.end().data, t2.end().data), kind).toBe(0);
    }
  });
});

const body = (text: string, family: string, size = 28): TextBody => ({
  defaultRun: { family, weight: 400, italic: false, size, fill: { k: 'solid', color: { space: 'srgb', r: 0, g: 0, b: 0, a: 1 } }, tracking: 0 },
  paragraphs: [{ runs: [{ text, style: {} }], align: 'start', lineHeight: 1.3, spaceAfter: 0 }],
} as TextBody);

describe.skipIf(!existsSync(FILES))('font loader', () => {
  test('missing faces are fetched once, registered, and text re-flows', async () => {
    const { fonts, calls } = registry(3);
    let loaded = 0;
    fonts.onLoad(() => loaded++);
    const text = new TextEngine(fonts);
    const b = body('Lobster is a display face that wraps differently', 'Lobster');
    const lobster = fonts.pickFace('Lobster', 400, false).face.file;
    expect(fonts.isLoaded('Lobster', 400, false)).toBe(false);
    const first = text.layout('el1', b, 260);
    // Laying out again (other keys, other widths) while the face loads does not fetch again.
    text.layout('el2', b, 300);
    text.layout('el1', b, 200);
    await settle(fonts);
    expect(calls.get(lobster)).toBe(1);
    expect(fonts.isLoaded('Lobster', 400, false)).toBe(true);
    expect(loaded).toBeGreaterThanOrEqual(1);
    const after = text.layout('el1', b, 260);
    // Exactly the layout of a registry that had the face from the start.
    const pre = registry();
    await pre.fonts.ensureAll([{ family: 'Lobster', weight: 400, italic: false }, { family: 'Inter', weight: 400, italic: false }]);
    const want = new TextEngine(pre.fonts).layout('x', b, 260);
    expect([after.width, after.height]).toEqual([want.width, want.height]);
    expect(text.caretRect(after, b.paragraphs[0]!.runs[0]!.text.length)).toEqual(new TextEngine(pre.fonts).caretRect(want, b.paragraphs[0]!.runs[0]!.text.length));
    expect(text.caretRect(first, 10).x).not.toBe(text.caretRect(after, 10).x);
    // Nothing else is pending; nothing is fetched twice.
    await settle(fonts);
    expect([...calls.values()].every((n) => n === 1)).toBe(true);
  });

  test('drawing requests fonts; uploaded families load through the same loader', async () => {
    const { fonts, calls } = registry();
    const p = new Painter(new TextEngine(fonts), async () => { throw new Error('none'); });
    let repaints = 0;
    p.onInvalidate(() => repaints++);
    // An uploaded brand font: a runtime family whose face is served under its own path.
    const roboto = manifest().families.find((f) => f.family === 'Roboto')!.faces.find((f) => f.weight === 700 && !f.italic)!;
    fonts.addFamily({ family: 'Acme Brand', slug: 'custom-1', category: 'uploaded', license: 'Uploaded', faces: [{ ...roboto }] });
    let doc = emptyDesign({ w: 300, h: 80 });
    const page = sortedPages(doc)[0]!.id;
    const el = makeText({ parentId: page, xf: { x: 10, y: 10, w: 280, h: 40, rot: 0 }, body: body('Brand headline', 'Acme Brand', 30) });
    doc = withEl(doc, el);
    p.renderPage(resolvePage(doc, page), 1);
    await settle(fonts);
    expect(calls.get(roboto.file)).toBe(1);
    expect(repaints).toBeGreaterThanOrEqual(1);
    expect(fonts.isLoaded('Acme Brand', 700, false)).toBe(true);
  });

  test('the editor re-measures auto-height text when a face arrives', async () => {
    const { fonts } = registry(2);
    let doc = emptyDesign({ w: 400, h: 400 });
    const page = sortedPages(doc)[0]!.id;
    const el = makeText({ parentId: page, xf: { x: 0, y: 0, w: 180, h: 40, rot: 0 }, body: body('A display face changes the wrapping of this long paragraph quite a lot', 'Lobster', 30) });
    doc = withEl(doc, el);
    const ydoc = new Y.Doc();
    Y.applyUpdate(ydoc, encodeDesign(doc));
    const editor = new Editor({ ydoc, actor: 'u1' });
    editor.textService = fonts.service;
    fonts.onLoad(() => editor.remeasureText());
    editor.remeasureText();
    await settle(fonts);
    const h = (editor.doc.elements[el.id] as TextEl).xf.h;
    const pre = registry();
    await pre.fonts.ensureAll([{ family: 'Lobster', weight: 400, italic: false }, { family: 'Inter', weight: 400, italic: false }]);
    expect(h).toBe(pre.fonts.service.height(editor.doc.elements[el.id] as TextEl, 180));
  });
});

describe.skipIf(!existsSync(FILES))('caret geometry', () => {
  test('the caret at the end of a line sits on the drawn glyphs', async () => {
    const { fonts } = registry();
    const text = new TextEngine(fonts);
    const p = new Painter(text, async () => { throw new Error('none'); });
    for (const [family, s] of [['Inter', 'Hamburgefonstiv'], ['Lobster', 'Display words'], ['Playfair Display', 'Serif caret test']] as const) {
      let doc = emptyDesign({ w: 600, h: 120 });
      const page = sortedPages(doc)[0]!.id;
      const el = makeText({ parentId: page, xf: { x: 20, y: 20, w: 560, h: 60, rot: 0 }, body: body(s, family, 40) });
      doc = withEl(doc, el);
      p.renderPage(resolvePage(doc, page), 1);
      await settle(fonts);
      const img = p.renderPage(resolvePage(doc, page), 1, [1, 1, 1, 1]);
      let right = 0;
      for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) if (img.data[(y * img.width + x) * 4]! < 128) right = Math.max(right, x + 1);
      const caret = text.caretRect(text.layout(el.id, el.body, el.xf.w, el.effect), s.length);
      // The caret is at the advance end (side bearing included): within a few px right of the ink.
      expect(20 + caret.x - right, family).toBeGreaterThanOrEqual(-1.5);
      expect(20 + caret.x - right, family).toBeLessThan(6);
    }
  });
});
