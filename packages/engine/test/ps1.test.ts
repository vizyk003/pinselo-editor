// SPDX-License-Identifier: Apache-2.0
/**
 * PS-1 across builds (AD-18, plan 13 stage 4.3): the WebAssembly renderer and text layout against the native
 * baselines in tests/golden/ps1/baselines.json (written by `crates/oc-render/tests/ps1.rs`, which documents the
 * corpus and the fingerprints).
 *
 * - PS-1/L: every text box's caret geometry (all UTF-16 offsets, 1/64 px) hashes to the native value.
 * - PS-1/A, wasm vs native (one backend): the page's RGBA pixels hash to the native value. A page whose hash
 *   differs falls back to the stored 16 × 16 cell means (CIEDE2000 ≤ 3.0 per cell) and is reported.
 *
 * `OC_PS1_FILTER=<substring>` limits the pages (the whole corpus takes a minute in vitest's workers).
 */
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import { FontRegistry, Painter, TextEngine, resolvePage, type AssetId, type DesignDoc, type FontManifest, type PageId } from '../src/index';
import { ROOT, fixture, golden, readJson } from './helpers';

interface Baseline { name: string; doc: string; page: string; scale: number; w: number; h: number; sha256: string; layout: string; cells: string }
const BASE = readJson<{ grid: number; pages: Baseline[] }>('tests/golden/ps1/baselines.json');
const FILTER = process.env.OC_PS1_FILTER;

const SCENE_ASSETS: Record<string, string> = { photo: 'photo.jpg', tile: 'tile.png', anim: 'anim.gif', pic: 'pic.webp', icon: 'icon.svg', 'clip/poster': 'poster.png' };
const scenes = new Map((golden<Array<{ name: string; doc: DesignDoc }>>('render', 'scenes')).map((s) => [`scenes:${s.name}`, s.doc]));
const docOf = (key: string): DesignDoc => scenes.get(key) ?? fixture(key.replace('fixtures/templates-seed/', ''));

const sha = (b: Uint8Array | Uint8ClampedArray | string) => createHash('sha256').update(b).digest('hex');
const q = (v: number) => Math.round(v * 64);

function lab(r: number, g: number, b: number): [number, number, number] {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  [r, g, b] = [lin(r), lin(g), lin(b)];
  const x = (0.4360747 * r + 0.3850649 * g + 0.1430804 * b) / 0.96422;
  const y = 0.2225045 * r + 0.7168786 * g + 0.0606169 * b;
  const z = (0.0139322 * r + 0.0971045 * g + 0.7141733 * b) / 0.82521;
  const f = (t: number) => (t > 216 / 24389 ? Math.cbrt(t) : ((24389 / 27) * t + 16) / 116);
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}

/** CIEDE2000 (same formula as ps1.rs). */
function de00([l1, a1, b1]: number[], [l2, a2, b2]: number[]): number {
  const rad = Math.PI / 180;
  const c1 = Math.hypot(a1!, b1!), c2 = Math.hypot(a2!, b2!), cm = (c1 + c2) / 2;
  const g = 0.5 * (1 - Math.sqrt(cm ** 7 / (cm ** 7 + 25 ** 7)));
  const a1p = (1 + g) * a1!, a2p = (1 + g) * a2!;
  const c1p = Math.hypot(a1p, b1!), c2p = Math.hypot(a2p, b2!);
  const hue = (b: number, a: number) => (b === 0 && a === 0 ? 0 : (Math.atan2(b, a) / rad + 360) % 360);
  const h1p = hue(b1!, a1p), h2p = hue(b2!, a2p);
  const dl = l2! - l1!, dc = c2p - c1p;
  const dh = c1p * c2p === 0 ? 0 : Math.abs(h2p - h1p) <= 180 ? h2p - h1p : h2p - h1p > 180 ? h2p - h1p - 360 : h2p - h1p + 360;
  const dH = 2 * Math.sqrt(c1p * c2p) * Math.sin((dh * rad) / 2);
  const lm = (l1! + l2!) / 2, cmp = (c1p + c2p) / 2;
  const hm = c1p * c2p === 0 ? h1p + h2p : Math.abs(h1p - h2p) <= 180 ? (h1p + h2p) / 2 : h1p + h2p < 360 ? (h1p + h2p + 360) / 2 : (h1p + h2p - 360) / 2;
  const t = 1 - 0.17 * Math.cos((hm - 30) * rad) + 0.24 * Math.cos(2 * hm * rad) + 0.32 * Math.cos((3 * hm + 6) * rad) - 0.2 * Math.cos((4 * hm - 63) * rad);
  const dtheta = 30 * Math.exp(-(((hm - 275) / 25) ** 2));
  const rc = 2 * Math.sqrt(cmp ** 7 / (cmp ** 7 + 25 ** 7));
  const sl = 1 + (0.015 * (lm - 50) ** 2) / Math.sqrt(20 + (lm - 50) ** 2), sc = 1 + 0.045 * cmp, sh = 1 + 0.015 * cmp * t;
  const rt = -Math.sin(2 * dtheta * rad) * rc;
  return Math.sqrt((dl / sl) ** 2 + (dc / sc) ** 2 + (dH / sh) ** 2 + rt * (dc / sc) * (dH / sh));
}

/** Worst cell CIEDE2000 between the pixels and the stored cell means. */
function worstCell(px: Uint8ClampedArray, w: number, h: number, cells: string, grid: number): number {
  const want = Buffer.from(cells, 'base64');
  const sum = Array.from({ length: grid * grid }, () => [0, 0, 0, 0]);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4, a = px[i + 3]! / 255;
      const c = (v: number) => (v / 255) * a + (1 - a);
      const l = lab(c(px[i]!), c(px[i + 1]!), c(px[i + 2]!));
      const s = sum[Math.floor((y * grid) / h) * grid + Math.floor((x * grid) / w)]!;
      s[0]! += l[0]; s[1]! += l[1]; s[2]! += l[2]; s[3]! += 1;
    }
  }
  let worst = 0;
  sum.forEach((s, i) => {
    const n = Math.max(1, s[3]!);
    const ours = [Math.round((s[0]! / n) * 2) / 2, Math.round(s[1]! / n), Math.round(s[2]! / n)];
    worst = Math.max(worst, de00([want[i * 3]! / 2, want[i * 3 + 1]! - 128, want[i * 3 + 2]! - 128], ours));
  });
  return worst;
}

describe('PS-1: WebAssembly vs native baselines', () => {
  test('pixels (PS-1/A) and text layout (PS-1/L)', async () => {
    const fonts = new FontRegistry(readJson<FontManifest>('fixtures/fonts/manifest.json'), async (file) => readFileSync(join(ROOT, 'fixtures/fonts/files', file)));
    const text = new TextEngine(fonts);
    const assets = async (a: AssetId) => {
      const f = SCENE_ASSETS[a];
      if (!f) throw new Error(`no asset ${a}`);
      return readFileSync(join(ROOT, 'crates/oc-render/tests/assets', f));
    };
    // Scenes and templates use different asset sets (templates draw without their library photos).
    const painters = { scenes: new Painter(text, assets), templates: new Painter(text, async (a) => { throw new Error(`no asset ${a}`); }) };
    const pages = BASE.pages.filter((p) => !FILTER || p.name.includes(FILTER));
    expect(pages.length).toBeGreaterThan(0);
    const layoutDiffs: string[] = [];
    const pixelDiffs: string[] = [];
    let exact = 0;
    for (const b of pages) {
      const doc = docOf(b.doc);
      const dl = resolvePage(doc, b.page as PageId);
      const painter = b.doc.startsWith('scenes:') ? painters.scenes : painters.templates;
      await painter.preload(dl);
      // PS-1/A
      const img = painter.renderPage(dl, b.scale);
      expect([img.width, img.height], b.name).toEqual([b.w, b.h]);
      if (sha(new Uint8Array(img.data.buffer, img.data.byteOffset, img.data.byteLength)) === b.sha256) exact++;
      else {
        const worst = worstCell(img.data, img.width, img.height, b.cells, BASE.grid);
        pixelDiffs.push(`${b.name} (cell ΔE00 ${worst.toFixed(2)})`);
        expect(worst, b.name).toBeLessThanOrEqual(3);
      }
      // PS-1/L (after drawing, like the native test: both lay out through the same caches)
      let s = '';
      for (const it of dl.items) {
        if (it.op !== 'text') continue;
        const l = text.layout(it.key, it.body, it.box.w, it.effect);
        s += `${it.key}:${l.length}:`;
        for (let o = 0; o <= l.length; o++) {
          const c = text.caretRect(l, o);
          s += `${q(c.x)},${q(c.y)},${q(c.w)},${q(c.h)};`;
        }
        s += '\n';
      }
      if (sha(s) !== b.layout) layoutDiffs.push(b.name);
    }
    console.log(`PS-1 wasm: ${pages.length} pages, ${exact} pixel-identical to native${pixelDiffs.length ? `; within thresholds: ${pixelDiffs.slice(0, 10).join(', ')}${pixelDiffs.length > 10 ? ' …' : ''}` : ''}`);
    expect(layoutDiffs, 'PS-1/L: text layout differs from native').toEqual([]);
  }, 600_000);
});
