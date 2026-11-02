// SPDX-License-Identifier: Apache-2.0
/**
 * Micro-benchmarks of the boundary (`pnpm --filter @oc/engine bench`; only meaningful on a release build).
 *
 * Drag scenario: one shape dragged across a page (a template, then the same plus 300 shapes so snapping has
 * candidates). Measured per pointer move:
 *
 * * `pointerMove` through the facade's typed hot path (numbers in, status word out);
 * * a whole frame: `pointerMove` + `state` + `view` (what a render loop reads after each move);
 * * the same move through the generic JSON `dispatch`, for comparison;
 * * the boundary alone: a status read, a trivial `dispatch`, a typed no-op-sized call (`setHover`).
 *
 * Plus snapshot transfer: the full document as JS objects (serde-wasm-bindgen) vs JSON text + `JSON.parse`, and the
 * editor state both ways. Each measurement runs after a warm-up long enough for V8 to tier the wasm up.
 *
 * The assertion is a floor (60 fps with a 2× margin on the busy page), not a target.
 */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import {
  Editor, encodeDesign, makeShape, sortedPages, type DesignDoc, type Mods, type Vec2,
} from '../src/index';
import { fixture } from './helpers';

const NO: Mods = { shift: false, alt: false, meta: false };

function setup(extra: number) {
  const design = structuredClone(fixture('pr-flyer-open-house.json')) as DesignDoc;
  const page = sortedPages(design)[0]!.id;
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, encodeDesign(design));
  const ed = new Editor(ydoc, { x: 0, y: 0, zoom: 1, width: 1600, height: 1200 });
  if (extra) ed.insert(Array.from({ length: extra }, (_, i) => makeShape({ xf: { x: (i % 20) * 37, y: Math.floor(i / 20) * 41, w: 24, h: 24, rot: 0 } })), undefined, { select: false });
  const [id] = ed.insert([makeShape({ xf: { x: 200, y: 200, w: 120, h: 80, rot: 0 } })], [400, 400]);
  ed.clearSelection();
  const [ox, oy] = ed.pageOrigin(page);
  const b = ed.doc.elements[id!]!.xf;
  return { ed, start: [ox + b.x + b.w / 2, oy + b.y + b.h / 2] as Vec2, elements: Object.keys(ed.doc.elements).length };
}

type Rate = { perSec: number; us: number };

function rate(n: number, f: (i: number) => void, warm = Math.min(n, 300)): Rate {
  for (let i = 0; i < warm; i++) f(i);
  const t0 = performance.now();
  for (let i = 0; i < n; i++) f(i);
  const ms = performance.now() - t0;
  return { perSec: Math.round((n / ms) * 1000), us: Math.round((ms / n) * 1000 * 10) / 10 };
}

const path = (start: Vec2, i: number): Vec2 => [start[0] + 150 * Math.sin(i / 40), start[1] + 120 * Math.cos(i / 55)];

/** One drag per measurement (fresh gesture, undone afterwards). */
function drag(ed: Editor, start: Vec2, n: number, move: (p: Vec2) => void): Rate {
  ed.pointerDown(start, NO);
  const r = rate(n, (i) => move(path(start, i)));
  ed.pointerUp();
  ed.undo();
  return r;
}

/** Best of `rounds` round-robin runs of each variant (filters out tier-up and machine-load noise). */
function best(rounds: number, variants: Record<string, () => Rate>): Record<string, Rate> {
  const out: Record<string, Rate> = {};
  for (let i = 0; i < rounds; i++) {
    for (const [k, f] of Object.entries(variants)) {
      const r = f();
      if (!out[k] || r.perSec > out[k]!.perSec) out[k] = r;
    }
  }
  return out;
}

const print = (title: string, rows: Record<string, Rate>) => {
  console.log(title);
  for (const [k, v] of Object.entries(rows)) console.log(`  ${k.padEnd(44)} ${String(v.perSec).padStart(9)}/s ${String(v.us).padStart(9)} µs`);
};

describe('boundary micro-benchmarks', () => {
  test('pointer-move drag', () => {
    const frames: number[] = [];
    for (const extra of [0, 300]) {
      const { ed, start, elements } = setup(extra);
      const raw = ed.raw;
      const someId = Object.keys(ed.doc.elements)[0]!;
      const rows = best(3, {
        'pointerMove (typed hot path)': () => drag(ed, start, 500, (p) => ed.pointerMove(p, NO)),
        'frame: pointerMove + state + view': () => drag(ed, start, 500, (p) => {
          ed.pointerMove(p, NO);
          void ed.state.guides;
          void ed.view.elements;
        }),
        'pointerMove (JSON dispatch)': () => drag(ed, start, 500, (p) => ed.dispatch('pointerMove', p, NO)),
        'boundary: status word': () => rate(50_000, () => raw.status()),
        'boundary: trivial JSON dispatch': () => rate(50_000, () => raw.dispatch('version', '')),
        'boundary: setHover (typed, notifies)': () => rate(20_000, (i) => ed.setHover(i % 2 ? null : (someId as never))),
      });
      frames.push(rows['frame: pointerMove + state + view']!.perSec);
      print(`drag over ${elements} elements:`, rows);
      if (extra) {
        print('snapshots:', best(3, {
          'full doc → JS objects (serde-wasm-bindgen)': () => rate(100, () => { raw.resetMirror(); raw.docDelta(); }, 20),
          'full doc → JSON text + JSON.parse': () => rate(100, () => { raw.resetMirror(); JSON.parse(raw.docDeltaJson()!); }, 20),
          'state → JS object (serde-wasm-bindgen)': () => rate(10_000, () => raw.state()),
          'state → JSON text + JSON.parse': () => rate(10_000, () => JSON.parse(raw.dispatchJson('state', ''))),
        }));
        raw.resetMirror();
      }
      ed.destroy();
    }
    expect(Math.min(...frames)).toBeGreaterThan(120);
  });
});
