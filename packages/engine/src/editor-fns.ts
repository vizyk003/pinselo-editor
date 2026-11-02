// SPDX-License-Identifier: Apache-2.0
/** `@oc/editor-core` module functions used by apps/web, backed by crates/oc-editor-core (signatures checked). */
import type * as EC from './types/editor-core';
import type { Element } from './types/schema';
import { docRef } from './docref';
import { onEngineReady, raw } from './wasm';

type Slot = { id: string; index: number; rect: EC.PageSlot['rect'] };

/** Page rectangles in scene space; `page` is the document's own page object. */
export const pageSlots: typeof EC.pageSlots = (doc) =>
  (raw.pageSlots(docRef(doc)) as Slot[]).map((s) => ({ page: doc.pages[s.id as keyof typeof doc.pages]!, index: s.index, rect: s.rect }));

export const screenToScene: typeof EC.screenToScene = (vp, p) => {
  const r = raw.screenToScene(vp, p[0], p[1]);
  return [r[0]!, r[1]!];
};

export const designColors: typeof EC.designColors = (doc) => raw.designColors(docRef(doc));
export const fontSetOps: typeof EC.fontSetOps = (doc, set) => raw.fontSetOps(docRef(doc), set);
export const auditDesign: typeof EC.auditDesign = (doc) => raw.auditDesign(docRef(doc));
export const describeElement: typeof EC.describeElement = (doc, el) => raw.describeElement(docRef(doc), el);
export const coverCrop: typeof EC.coverCrop = (boxAspect, imgAspect) => raw.coverCrop(boxAspect, imgAspect);
export const refitCrop: typeof EC.refitCrop = (crop, from, to) => raw.refitCrop(crop, from, to);
export const scalePatches: typeof EC.scalePatches = (doc, el, sx, sy) =>
  new Map(Object.entries(raw.scalePatches(docRef(doc as Parameters<typeof docRef>[0]), el, sx, sy) as Record<string, Partial<Element>>)) as ReturnType<typeof EC.scalePatches>;
/** Ids come from Web Crypto, like the TS `newId`; `editor.templatePages` draws from the editor's stream instead. */
export const templatePages: typeof EC.templatePages = (tpl, target) => raw.templatePages(docRef(tpl), target.w, target.h);

