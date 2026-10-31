// SPDX-License-Identifier: Apache-2.0
/** `@oc/geometry` exports used by apps/web, backed by crates/oc-geometry (signatures checked against the TS). */
import type * as G from './types/geometry';
import type { Element } from './types/schema';
import { docRef } from './docref';
import { onEngineReady, raw } from './wasm';

export let ANCHORS: typeof G.ANCHORS;

onEngineReady(() => {
  ({ ANCHORS } = Object.freeze(raw.geometryConstants()) as { ANCHORS: typeof G.ANCHORS });
  Object.freeze(ANCHORS);
});

type Mat = ReturnType<typeof G.worldMatrix>;

export const worldMatrix: typeof G.worldMatrix = (doc, id) => Array.from(raw.worldMatrix(docRef(doc), id)) as unknown as Mat;
export const worldBounds: typeof G.worldBounds = (doc, id) => raw.worldBounds(docRef(doc), id);
export const selectionBounds: typeof G.selectionBounds = (doc, ids) => raw.selectionBounds(docRef(doc as Parameters<typeof docRef>[0]), [...ids]) ?? null;

export const worldQuad: typeof G.worldQuad = (doc, id) => {
  const q = raw.worldQuad(docRef(doc), id);
  return [[q[0]!, q[1]!], [q[2]!, q[3]!], [q[4]!, q[5]!], [q[6]!, q[7]!]];
};

export const anchorPoint: typeof G.anchorPoint = (doc, id, anchor) => {
  const p = raw.anchorPoint(docRef(doc), id, anchor);
  return p ? [p[0]!, p[1]!] : null;
};

/** Topmost selectable element under `p` — the document's own object. */
export const hitTest: typeof G.hitTest = (doc, pageId, p, opts = {}) => {
  const id = raw.hitTest(docRef(doc), pageId, p[0], p[1], opts.tolerance ?? 0, opts.enterGroup ?? undefined);
  return id === undefined ? null : ((doc.elements as Record<string, Element>)[id] ?? null);
};

export const apply: typeof G.apply = (m, p) => {
  const r = raw.apply(Float64Array.from(m), p[0], p[1]);
  return [r[0]!, r[1]!];
};

export const invert: typeof G.invert = (m) => Array.from(raw.invert(Float64Array.from(m))) as unknown as Mat;
/** Whether `p` (scene space) is on `el` (lines: near any segment). */
export const hitElement: typeof G.hitElement = (doc, el, p, tolerance = 0) => raw.hitElement(docRef(doc), el, p[0], p[1], tolerance);
/** Unknown clips give `undefined`, like `shapePath`. */
export const frameOutline: typeof G.frameOutline = (el, w, h) => raw.frameOutline({ clip: el.clip, clipPath: el.clipPath }, w, h) || (undefined as never);
export const multiply: typeof G.multiply = (m, n) => Array.from(raw.multiply(Float64Array.from(m), Float64Array.from(n))) as unknown as Mat;
/** Unknown kinds give `undefined`, like the TS switch falling through (the Rust port returns ""). */
export const shapePath: typeof G.shapePath = (kind, w, h, params = {}, d) => raw.shapePath(kind, w, h, params, d) || (undefined as never);
export const scaleUnitPath: typeof G.scaleUnitPath = (d, w, h) => raw.scaleUnitPath(d, w, h);
export const lineMarkerShape: typeof G.lineMarkerShape = (kind, width) => raw.lineMarkerShape(kind, width);

export const trimLineForMarkers: typeof G.trimLineForMarkers = (points, start, end, width) => {
  const flat = new Float64Array(points.length * 2);
  points.forEach((p, i) => {
    flat[i * 2] = p[0];
    flat[i * 2 + 1] = p[1];
  });
  const r = raw.trimLineForMarkers(flat, start, end, width);
  const out: Array<[number, number]> = [];
  for (let i = 0; i + 1 < r.length; i += 2) out.push([r[i]!, r[i + 1]!]);
  return out;
};
