// SPDX-License-Identifier: Apache-2.0
/**
 * `EngineTextService` (oc-text in WebAssembly) with fonts fed from JS, against tests/golden/text/hand.json (written
 * from the TS SkParagraph engine). Acceptance mirrors crates/oc-text/tests/golden.rs: total heights within 0.01 px for
 * ≥ 99% of the cases and within 2% for all, `wordAt` exact, at most 5% of `offsetAt` / `verticalMove` probes
 * different, curved boxes within 2%. Then the service drives an editor (auto-height text measured in Rust).
 *
 * Needs the pinned font files (`pnpm fonts:sync` → fixtures/fonts/files/).
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import { Editor, EngineTextService, encodeDesign, emptyDesign, makeText, type FontManifest, type TextBody, type TextEl } from '../src/index';
import { ROOT, golden, readJson } from './helpers';

const FILES = join(ROOT, 'fixtures/fonts/files');
const manifest = readJson<FontManifest>('fixtures/fonts/manifest.json');

function service(): EngineTextService {
  return new EngineTextService(manifest);
}

/** Load every face `body` needs (the host's job: fetch the files the service reports missing). */
function ensure(ts: EngineTextService, body: TextBody) {
  for (const file of ts.missingFonts(body)) {
    const got = ts.registerFont(readFileSync(join(FILES, file)));
    expect(got).toBe(file);
  }
}

const virtualEl = (name: string, body: TextBody, width: number, curve?: number): TextEl =>
  ({ id: name, type: 'text', parentId: '', z: 'a0', opacity: 1, xf: { x: 0, y: 0, w: width, h: 10, rot: 0 }, body, frame: { mode: 'autoHeight' }, autoFit: 'none', ...(curve ? { curve } : {}) }) as TextEl;

describe.skipIf(!existsSync(FILES))('EngineTextService', () => {
  test('registers faces by content hash', () => {
    const ts = service();
    const inter = manifest.families.find((f) => f.family === 'Inter')!.faces.find((f) => f.weight === 400 && !f.italic)!;
    expect(ts.registerFont(readFileSync(join(FILES, inter.file)))).toBe(inter.file);
    expect(ts.generation).toBe(1);
    expect(() => ts.registerFont(new Uint8Array([1, 2, 3]))).toThrow(/no manifest face/);
    expect(ts.hasFamily('Inter')).toBe(true);
  });

  test('hand cases match the TS layout (oc-text acceptance)', () => {
    const ts = service();
    const cases = golden('text', 'hand.json') as any[];
    let exact = 0;
    let checked = 0;
    const far: string[] = [];
    let probes = 0;
    let probeDiff = 0;
    let vTotal = 0;
    let vDiff = 0;
    for (const c of cases) {
      if (c.fitHeight !== undefined) continue; // shrink-to-fit is not part of the TextService interface
      ensure(ts, c.body);
      if (c.curve !== undefined) {
        const s = ts.curvedSize(virtualEl(c.name, c.body, c.width, c.curve));
        const w = c.out.curved;
        if (!w) expect(s).toBeNull();
        else {
          expect(Math.abs(s!.w - w.width) / Math.max(1, w.width), `${c.name} curved w`).toBeLessThanOrEqual(0.02);
          expect(Math.abs(s!.h - w.height) / Math.max(1, w.height), `${c.name} curved h`).toBeLessThanOrEqual(0.02);
        }
        continue;
      }
      const el = virtualEl(c.name, c.body, c.width);
      const h = ts.height(el, c.width);
      checked++;
      if (Math.abs(h - c.out.height) < 0.01) exact++;
      else if (Math.abs(h - c.out.height) / Math.max(1, Math.abs(c.out.height)) > 0.02) far.push(`${c.name}: ${h} vs ${c.out.height}`);
      expect(ts.length(el)).toBe(c.out.length);
      const want = c.out.caret;
      if (!want) continue;
      want.wordAt.forEach((w: [number, number], o: number) => expect(ts.wordAt(el, o), `${c.name} wordAt(${o})`).toEqual(w));
      want.verticalDown.forEach((w: number, o: number) => { vTotal++; if (ts.verticalMove(el, o, 1) !== w) vDiff++; });
      want.verticalUp.forEach((w: number, o: number) => { vTotal++; if (ts.verticalMove(el, o, -1) !== w) vDiff++; });
      for (const p of want.offsetAt) { probes++; if (ts.offsetAt(el, p.x, p.y) !== p.out) probeDiff++; }
    }
    console.log(`text: ${checked} cases, ${exact} exact heights; offsetAt ${probeDiff}/${probes} differ, verticalMove ${vDiff}/${vTotal} differ`);
    expect(far).toEqual([]);
    expect(exact * 100).toBeGreaterThanOrEqual(checked * 99);
    expect(probeDiff * 20).toBeLessThanOrEqual(probes);
    expect(vDiff * 20).toBeLessThanOrEqual(vTotal);
  });

  test('drives the editor: auto-height text is measured in Rust', () => {
    const ts = service();
    const design = emptyDesign({ w: 800, h: 600 });
    const ydoc = new Y.Doc();
    Y.applyUpdate(ydoc, encodeDesign(design));
    const ed = new Editor(ydoc);
    ed.textService = ts;
    expect(ed.textService).toBe(ts);
    expect(ed.raw.hasTextService).toBe(true);
    const el = makeText({ xf: { x: 0, y: 0, w: 120, h: 10, rot: 0 }, text: 'Short', style: { size: 20 } });
    ensure(ts, el.body);
    const [id] = ed.insert([el]);
    const h1 = ed.doc.elements[id!]!.xf.h;
    expect(h1).toBeCloseTo(ts.height(ed.doc.elements[id!] as TextEl, 120), 6);
    ed.startTextEdit(id!);
    ed.moveCaret('end', false);
    ed.typeText(' and a much longer line that has to wrap several times in a narrow box');
    ed.stopTextEdit();
    const after = ed.doc.elements[id!] as TextEl;
    expect(after.xf.h).toBeGreaterThan(h1 * 2);
    expect(after.xf.h).toBeCloseTo(ts.height(after, 120), 6);
    // Fonts arriving late: remeasure (the host calls it from `onLoad`).
    let loaded = 0;
    const off = ts.onLoad(() => loaded++);
    const bold = manifest.families.find((f) => f.family === 'Inter')!.faces.find((f) => f.weight === 700 && !f.italic)!;
    ts.registerFont(readFileSync(join(FILES, bold.file)));
    off();
    expect(loaded).toBe(1);
    ed.remeasureText();
    ed.destroy();
  });
});
