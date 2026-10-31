// SPDX-License-Identifier: Apache-2.0
/**
 * The helpers apps/web's drop targets use (editor/dropTarget.ts): `coverCrop`, `invert`, `frameOutline`,
 * `hitElement`, `gridCellBox`, `brandLockError` — facade (Rust) against tests/golden/engine/drop-helpers.json, the
 * TS originals' outputs on the seed templates (recorded before the TS packages were deleted, plan 13 4.1).
 */
import { describe, expect, test } from 'vitest';
import { apply, brandLockError, coverCrop, frameOutline, gridCellBox, hitElement, invert, worldMatrix, type DesignDoc, type Element } from '../src/index';
import { diff, fixture, golden } from './helpers';

const close = (a: readonly number[], b: readonly number[]) => a.every((v, i) => Math.abs(v - b[i]!) <= 1e-9 * Math.max(1, Math.abs(v)));

interface Recorded {
  invert: number[];
  frameOutline?: string;
  gridCellBox?: unknown[];
  hits: Array<{ p: [number, number]; hit: boolean }>;
}
const G = golden<{
  coverCrop: Array<{ args: [number, number]; out: unknown }>;
  docs: Record<string, Record<string, Recorded>>;
  brandLock: Array<{ lock: unknown; action: unknown; out: string | null }>;
}>('engine', 'drop-helpers');

describe('drop target helpers', () => {
  test('coverCrop', () => {
    for (const { args, out } of G.coverCrop) expect(diff(coverCrop(...args), out)).toBeNull();
  });

  test('frames, cells, hits and matrices on the seed templates', () => {
    let frames = 0;
    let grids = 0;
    let hits = 0;
    for (const [name, els] of Object.entries(G.docs)) {
      const doc = fixture(name) as DesignDoc;
      expect(Object.keys(doc.elements).sort(), name).toEqual(Object.keys(els).sort());
      for (const el of Object.values(doc.elements) as Element[]) {
        const want = els[el.id]!;
        const m = worldMatrix(doc, el.id);
        expect(close([...invert(m)], want.invert), `${name} ${el.id}`).toBe(true);
        if (el.type === 'frame') {
          frames++;
          expect(diff(frameOutline(el, el.xf.w, el.xf.h), want.frameOutline), `${name} ${el.id}`).toBeNull();
        }
        if (el.type === 'grid') {
          (el as Extract<Element, { type: 'grid' }>).cells.forEach((c, i) => {
            grids++;
            expect(diff(gridCellBox(el as never, c.rect), want.gridCellBox![i]), `${name} ${el.id}`).toBeNull();
          });
        }
        // The centre and a point outside the box.
        const probes = [apply(m, [el.xf.w / 2, el.xf.h / 2]), apply(m, [-5, -5])];
        probes.forEach((p, i) => {
          hits++;
          expect(close(p, want.hits[i]!.p)).toBe(true);
          expect(hitElement(doc, el, p, 2), `${name} ${el.id}`).toBe(want.hits[i]!.hit);
        });
      }
    }
    expect(frames + grids).toBeGreaterThan(0);
    expect(hits).toBeGreaterThan(1000);
  });

  test('brandLockError', () => {
    for (const { lock, action, out } of G.brandLock) expect(brandLockError(lock as never, action as never), JSON.stringify([lock, action])).toBe(out);
  });
});
