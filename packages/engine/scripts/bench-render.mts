// SPDX-License-Identifier: Apache-2.0
/**
 * Editor paint benchmark (`pnpm --filter @oc/engine bench:render [template.json …]`, release build, plain Node —
 * vitest's workers run WebAssembly several times slower). Drives apps/web's tiled scene painter
 * (`apps/web/src/editor/scene.ts`, 256 px tiles, 10 ms tile budget per frame) with a stub 2D context, like the
 * editor: a 1440 × 900 CSS px viewport at device pixel ratio 2 (2880 × 1800 device px), the page fitted at 90 %.
 *
 * - first paint (every tile; frames until no tile is pending);
 * - drag: a pointer drag through the editor (pointerDown on a large element, pointerMove back and forth by up to
 *   90 px for 60 frames) → per-frame cost (pointerMove + resolvePage of the view + dirty region + layered tiles);
 * - typing: a character appended per frame to the largest text element through `editor.apply` (30 frames, drawn
 *   as the text being edited);
 * - settle: the frames after the gesture (layered tiles repainted plainly);
 * - thumbnail: the page at 160 px wide.
 *
 * `OC_ENGINE_WASM=path/to/oc_wasm_bg.wasm` loads another build (e.g. one with a name section, for profiling).
 * Photos come from fixtures/template-photos (gitignored; `OC_TEMPLATE_PHOTOS`, else the main checkout's copy).
 */
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import './bench-wasm.mts';
import * as Y from 'yjs';
import { Editor, FontRegistry, Painter, TextEngine, encodeDesign, hitTest, resolvePage, sortedPages, type AssetId, type DesignDoc, type DisplayList, type FontManifest, type PageId, type TextEl } from '../src/index';
import { ScenePainter, scenePages } from '../../../apps/web/src/editor/scene';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');
const readJson = <T,>(p: string): T => JSON.parse(readFileSync(join(ROOT, p), 'utf8')) as T;
const FILES = join(ROOT, 'fixtures/fonts/files');
const PHOTOS = [process.env.OC_TEMPLATE_PHOTOS, join(ROOT, 'fixtures/template-photos'), join(ROOT, '../opencanvas/fixtures/template-photos')].find((d) => d && existsSync(d));
if (!existsSync(FILES)) throw new Error('fixtures/fonts/files missing (pnpm fonts:sync)');
if (!PHOTOS) console.warn('no template photos: images draw placeholders');

const CSS = { w: 1440, h: 900 };
const DPR = 2;

const photos = new Map<string, string>();
if (PHOTOS) for (const f of readdirSync(PHOTOS)) photos.set(createHash('sha256').update(readFileSync(join(PHOTOS, f))).digest('hex'), join(PHOTOS, f));

const stats = (xs: number[]) => {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  const q = (p: number) => s[Math.min(s.length - 1, Math.floor(p * s.length))]!;
  return { median: +q(0.5).toFixed(1), p95: +q(0.95).toFixed(1), max: +s[s.length - 1]!.toFixed(1) };
};

/** A canvas whose 2D context accepts the scene painter's calls and does nothing (putImageData is the browser's). */
function stubCanvas(width: number, height: number) {
  const ctx = new Proxy({}, { get: () => () => {} });
  return { width, height, getContext: () => ctx } as unknown as HTMLCanvasElement;
}

for (const file of process.argv.slice(2).length ? process.argv.slice(2) : ['vd-deck-lesson.json', 'mw-web-services-greenleaf.json', 'pr-flyer-open-house.json']) {
  const doc0 = readJson<DesignDoc>(`fixtures/templates-seed/${file}`);
  const fonts = new FontRegistry(readJson<FontManifest>('fixtures/fonts/manifest.json'), async (f) => readFileSync(join(FILES, f)));
  const text = new TextEngine(fonts);
  const painter = new Painter(text, async (key) => {
    const ref = doc0.assets?.[key.replace(/\/poster$/, '') as AssetId];
    const p = ref && photos.get(ref.sha256);
    if (!p) throw new Error(`no ${key}`);
    return readFileSync(p);
  });
  const engine = { fonts, text, painter };
  const page = sortedPages(doc0)[0]!;
  const { w, h } = doc0.meta.dims;
  const zoom = Math.min(CSS.w / w, CSS.h / h) * 0.9;
  const port = { x: -(CSS.w / zoom - w) / 2, y: -(CSS.h / zoom - h) / 2, zoom, width: CSS.w, height: CSS.h };
  const canvas = stubCanvas(CSS.w * DPR, CSS.h * DPR);
  const scene = new ScenePainter(canvas, engine as never);
  const cache = new Map<PageId, { doc: DesignDoc; dl: DisplayList }>();
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, encodeDesign(doc0));
  const editor = new Editor({ ydoc, actor: 'bench' });
  editor.textService = fonts.service;
  let doc = editor.view;
  let editing: string | null = null;
  const frame = () => {
    const t0 = performance.now();
    const { pages, layout } = scenePages(doc, port, (id) => {
      const hit = cache.get(id);
      if (hit && hit.doc === doc) return hit.dl;
      const dl = resolvePage(doc, id, { editingText: editing as never });
      cache.set(id, { doc, dl });
      return dl;
    });
    const more = scene.frame({ view: doc, port, dpr: DPR, workspace: [0.91, 0.91, 0.93, 1], whiteboard: false, pages, layout });
    return { ms: performance.now() - t0, more };
  };
  const drain = () => {
    const out: number[] = [];
    for (let i = 0; i < 400; i++) {
      const f = frame();
      out.push(f.ms);
      if (!f.more) break;
    }
    return out;
  };
  // Warm up: fonts and photos load, then paint again.
  drain();
  await fonts.idle();
  await painter.preload(resolvePage(doc, page.id));
  await new Promise((r) => setTimeout(r, 30));
  scene.invalidate();
  const first = drain();
  scene.invalidate();
  const t0 = performance.now();
  drain();
  const fullMs = performance.now() - t0;

  // Drag the second largest element below 60 % of the page area.
  const els = Object.values(doc0.elements).filter((e) => e.parentId === page.id && e.xf.w * e.xf.h < w * h * 0.6).sort((a, b) => b.xf.w * b.xf.h - a.xf.w * a.xf.h);
  const mover = els[Math.min(1, els.length - 1)]!;
  const drag: number[] = [];
  const tiles: number[] = [];
  const backlog: number[] = [];
  const layeredTiles: number[] = [];
  const at: [number, number] = [mover.xf.x + mover.xf.w / 2, mover.xf.y + mover.xf.h / 2];
  const dragged = hitTest(doc, page.id, at, {})?.id ?? null;
  const mods = { shift: false, alt: false, meta: false };
  editor.pointerDown(at, mods, { kind: 'canvas' }, 0);
  for (let i = 0; i < 60; i++) {
    const t1 = performance.now();
    // Back and forth (±90 px, 3 px per frame), staying on the page.
    const x = 90 * Math.sin(((i + 1) / 60) * Math.PI * 2);
    editor.pointerMove([at[0] + x, at[1] + x / 3], mods, 0.5);
    doc = editor.view;
    frame();
    drag.push(performance.now() - t1);
    tiles.push(scene.last.tiles);
    backlog.push(scene.last.pending);
    layeredTiles.push(scene.last.layered);
  }
  editor.pointerUp(at, mods);
  doc = editor.view;
  await new Promise((r) => setTimeout(r, 300));
  const settle = drain();

  // Typing into the largest text element (through the editor: the view is the editor's snapshot).
  const tx = Object.values(doc.elements).filter((e): e is TextEl => e.type === 'text' && e.parentId === page.id).sort((a, b) => b.xf.w * b.xf.h - a.xf.w * a.xf.h)[0];
  const typing: number[] = [];
  const typingEdit: number[] = [];
  const typingTiles: number[] = [];
  if (tx) {
    editing = tx.id;
    for (let i = 0; i < 30; i++) {
      const t1 = performance.now();
      const el = editor.doc.elements[tx.id] as TextEl;
      const p0 = el.body.paragraphs[0]!;
      const run = p0.runs[0] ?? { text: '', style: {} };
      const body = { ...el.body, paragraphs: [{ ...p0, runs: [{ ...run, text: run.text + 'x' }, ...p0.runs.slice(1)] }, ...el.body.paragraphs.slice(1)] };
      editor.apply([{ op: 'update', id: tx.id, patch: { body } }] as never, true);
      doc = editor.view;
      const t2 = performance.now();
      typingEdit.push(t2 - t1);
      frame();
      typing.push(performance.now() - t1);
      typingTiles.push(scene.last.tiles);
    }
  }

  const thumbs: number[] = [];
  for (let i = 0; i < 5; i++) {
    const t1 = performance.now();
    painter.renderPage(resolvePage(doc, page.id), 160 / w, [1, 1, 1, 1]);
    thumbs.push(performance.now() - t1);
  }
  console.log(JSON.stringify({
    file, elements: Object.keys(doc0.elements).length, dragged, devicePx: `${CSS.w * DPR}x${CSS.h * DPR}`, pagePx: `${Math.round(w * zoom * DPR)}x${Math.round(h * zoom * DPR)}`,
    firstPaint: { frames: first.length, totalMs: +first.reduce((a, b) => a + b, 0).toFixed(0), perFrame: stats(first) }, fullRepaintMs: +fullMs.toFixed(0),
    dragFrameMs: stats(drag), dragTilesPerFrame: stats(tiles), dragBacklogTiles: stats(backlog), dragLayeredTiles: stats(layeredTiles), dragged: [mover.type, Math.round(mover.xf.w), Math.round(mover.xf.h)], settleFrames: settle.length, typingFrameMs: stats(typing), typingEditMs: stats(typingEdit), typingTiles: stats(typingTiles), thumbnailMs: stats(thumbs),
  }));
}
