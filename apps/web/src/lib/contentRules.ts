/**
 * Pure rules behind the plan 11 batch 3 content tools (no engine or DOM imports, so they are unit-tested in
 * tests/content-rules.test.ts): background-image crop, document fonts, text reading order and heading levels.
 */
import type { DesignDoc, Rect, SemanticRole } from '@oc/engine';

/** The part of `crop` (normalized, of an image showing at aspect `a` = w/h) that covers a box of aspect `p`, centred. */
export function coverCrop(crop: Rect, a: number, p: number): Rect {
  if (!(a > 0 && p > 0)) return crop;
  if (p > a) {
    const h = (crop.h * a) / p;
    return { x: crop.x, y: crop.y + (crop.h - h) / 2, w: crop.w, h };
  }
  const w = (crop.w * p) / a;
  return { x: crop.x + (crop.w - w) / 2, y: crop.y, w, h: crop.h };
}

/** Font families the design uses: text (default run and every run), tables and charts, in first-use order. */
export function documentFonts(doc: Pick<DesignDoc, 'elements'>): string[] {
  const out = new Set<string>();
  for (const el of Object.values(doc.elements)) {
    if (el.deleted) continue;
    if (el.type === 'text') {
      out.add(el.body.defaultRun.family);
      for (const p of el.body.paragraphs) for (const r of p.runs) if (r.style.family) out.add(r.style.family);
    } else if (el.type === 'table' || el.type === 'chart') {
      out.add(el.style.family);
    }
  }
  return [...out].filter(Boolean);
}

/** Reading order of two boxes: top to bottom, then left to right (boxes within half a height count as one row). */
export function readingOrder(a: { x: number; y: number; h: number }, b: { x: number; y: number; h: number }): number {
  return Math.abs(a.y - b.y) < Math.min(a.h, b.h) / 2 ? a.x - b.x : a.y - b.y;
}

/** Heading level of a text role: 1 for headlines, 2 for subheads, none for the rest. */
export function headingLevel(role: SemanticRole | undefined): 1 | 2 | null {
  return role === 'headline' ? 1 : role === 'subhead' ? 2 : null;
}
