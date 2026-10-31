// SPDX-License-Identifier: Apache-2.0
/** `@oc/editor-core` module functions used by apps/web, backed by crates/oc-editor-core (signatures checked). */
import type * as EC from './types/editor-core';
import type { Element } from './types/schema';
import { docRef } from './docref';
import { onEngineReady, raw } from './wasm';

export let STICKY_COLORS: typeof EC.STICKY_COLORS;

onEngineReady(() => {
  ({ STICKY_COLORS } = raw.editorConstants() as { STICKY_COLORS: typeof EC.STICKY_COLORS });
  Object.freeze(STICKY_COLORS);
});

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

export const isSticky: typeof EC.isSticky = (el) => raw.isSticky(el);
export const isSection: typeof EC.isSection = (el) => raw.isSection(el);
export const isMindNode: typeof EC.isMindNode = (el) => raw.isMindNode(el);
export const sectionsOf: typeof EC.sectionsOf = (doc, pageId) => raw.sectionsOf(docRef(doc), pageId).map((id) => (doc.elements as Record<string, Element>)[id]!);
export const mindNodeOf: typeof EC.mindNodeOf = (doc, id) => {
  const n = raw.mindNodeOf(docRef(doc), id);
  return n === undefined ? null : ((doc.elements as Record<string, Element>)[n] ?? null);
};
export const voteScope: typeof EC.voteScope = (doc, pageId, selection) => raw.voteScope(docRef(doc), pageId, [...selection]) as never;
export const tallyVotes: typeof EC.tallyVotes = (session, entries) => raw.tallyVotes(session.id, [...session.scope], entries);
export const topVoted: typeof EC.topVoted = (tally, n = 3) => raw.topVoted(tally, n) as never;
export const voteAllowance: typeof EC.voteAllowance = (session, entries, voter) => raw.voteAllowance(session.id, session.perPerson, session.status, entries, voter);
export const formatClock: typeof EC.formatClock = (ms) => raw.formatClock(ms);
export const makeTimer: typeof EC.makeTimer = (durationMs, sound = true) => raw.makeTimer(durationMs, sound);

// `{ ...t, … }` in TS keeps fields the timer type does not declare; the Rust state keeps them in `extra` too, and
// spreading the input first keeps them even if they are not JSON (functions, undefined).
export const timerRemaining: typeof EC.timerRemaining = (t, now) => raw.timerRemaining(t, now);
export const timerRunning: typeof EC.timerRunning = (t, now) => raw.timerRunning(t, now);
export const timerStart: typeof EC.timerStart = (t, now) => ({ ...t, ...raw.timerStart(t, now) });
export const timerPause: typeof EC.timerPause = (t, now) => ({ ...t, ...raw.timerPause(t, now) });
export const timerReset: typeof EC.timerReset = (t) => ({ ...t, ...raw.timerReset(t) });
export const timerAdd: typeof EC.timerAdd = (t, ms, now) => ({ ...t, ...raw.timerAdd(t, ms, now) });

/** `@oc/crdt` `brandLockError`: why an action would break a brand lock (`null` = allowed). */
export const brandLockError: typeof import('./types/crdt').brandLockError = (lock, action) => raw.brandLockError(lock ?? null, action) ?? null;
