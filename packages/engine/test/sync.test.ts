// SPDX-License-Identifier: Apache-2.0
/**
 * Sync between the browser's yjs `Y.Doc` and the Rust editor's yrs mirror, in both directions:
 *
 * * engine → yjs: editor commands reach the yjs doc (and from there a provider);
 * * yjs → engine: writes made directly on the yjs doc (TipTap, the board) and updates from peers reach the editor;
 * * two browsers (yjs doc + editor each) linked like `@hocuspocus/provider` converge after sequential and
 *   concurrent edits, undo included;
 * * a late joiner starting from the encoded state converges too.
 */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import { Editor, decodeDesign, encodeDesign, makeShape, makeText, sortedPages, type DesignDoc, type ElementId } from '../src/index';
import { diff, fixture, json, withoutNullList } from './helpers';

/** The design a yjs doc holds (decoded by the Rust codec from a full state update). */
const designOf = (d: Y.Doc): DesignDoc => decodeDesign(Y.encodeStateAsUpdate(d));

const same = (a: unknown, b: unknown, label: string) => expect(diff(withoutNullList(json(a)), withoutNullList(json(b)), label), label).toBeNull();

/** A browser: a yjs doc seeded with `seed` (or empty) and an editor on it. */
function browser(seed: Uint8Array | null, name: string) {
  const ydoc = new Y.Doc();
  if (seed) Y.applyUpdate(ydoc, seed, 'seed');
  const ed = new Editor({ ydoc, actor: name }, { x: 0, y: 0, zoom: 1, width: 1200, height: 900 });
  return { ydoc, ed };
}

/** Link two yjs docs like a sync provider: every update made on one is applied to the other (now or on flush). */
function link(a: Y.Doc, b: Y.Doc, opts: { manual?: boolean } = {}) {
  const queue: Array<[Y.Doc, Uint8Array]> = [];
  const origin = Symbol('provider');
  const send = (to: Y.Doc) => (u: Uint8Array, o: unknown) => {
    if (o === origin) return;
    if (opts.manual) queue.push([to, u]);
    else Y.applyUpdate(to, u, origin);
  };
  a.on('update', send(b));
  b.on('update', send(a));
  return { flush: () => { for (const [to, u] of queue.splice(0)) Y.applyUpdate(to, u, origin); } };
}

const seedDesign = () => encodeDesign(fixture('card-business-clean.json'));

describe('yjs ⇄ wasm sync', () => {
  test('editor commands reach the yjs doc', () => {
    const { ydoc, ed } = browser(seedDesign(), 'a');
    same(ed.doc, designOf(ydoc), 'initial');
    const page = ed.state.activePageId;
    const [id] = ed.insert([makeShape({ xf: { x: 10, y: 10, w: 80, h: 40, rot: 0 } })], [100, 100]);
    expect(designOf(ydoc).elements[id!]).toBeTruthy();
    // A drag: overrides only while moving (nothing reaches the yjs doc), one commit on pointer-up.
    const before = Y.encodeStateVector(ydoc);
    const o = ed.pageOrigin(page);
    const c = [o[0] + 100, o[1] + 100] as [number, number];
    ed.pointerDown(c, { shift: false, alt: false, meta: false });
    for (let i = 1; i <= 10; i++) ed.pointerMove([c[0] + i * 5, c[1] + i * 3], { shift: false, alt: false, meta: false });
    expect(Y.encodeStateVector(ydoc)).toEqual(before);
    expect(ed.view.elements[id!]!.xf.x).not.toBe(ed.doc.elements[id!]!.xf.x);
    ed.pointerUp();
    expect(Y.encodeStateVector(ydoc)).not.toEqual(before);
    same(ed.doc, designOf(ydoc), 'after drag');
    // Text editing (YText formatting) and undo.
    const [tid] = ed.insert([makeText({ xf: { x: 0, y: 0, w: 300, h: 40, rot: 0 }, text: 'Hello' })]);
    ed.startTextEdit(tid!);
    ed.moveCaret('end', false);
    ed.typeText(' world');
    ed.stopTextEdit();
    same(ed.doc, designOf(ydoc), 'after typing');
    ed.undo();
    same(ed.doc, designOf(ydoc), 'after undo');
    ed.destroy();
  });

  test('writes made on the yjs doc reach the editor and notify', () => {
    const { ydoc, ed } = browser(seedDesign(), 'a');
    let notified = 0;
    ed.subscribe(() => notified++);
    const v = ed.version;
    ydoc.transact(() => ydoc.getMap('meta').set('title', 'Renamed in TipTap'), 'tiptap');
    expect(ed.doc.meta.title).toBe('Renamed in TipTap');
    expect(ed.version).toBeGreaterThan(v);
    expect(notified).toBeGreaterThan(0);
    // An element deleted remotely leaves the selection (the editor sanitises its state).
    const id = Object.keys(ed.doc.elements)[0] as ElementId;
    ed.select([id]);
    expect(ed.state.selection).toEqual([id]);
    ydoc.transact(() => ydoc.getMap('elements').delete(id), 'peer');
    expect(ed.doc.elements[id]).toBeUndefined();
    expect(ed.state.selection).toEqual([]);
    ed.destroy();
  });

  test('two browsers converge (sequential, concurrent, undo, late joiner)', () => {
    const seed = seedDesign();
    const A = browser(seed, 'a');
    const B = browser(seed, 'b');
    const net = link(A.ydoc, B.ydoc, { manual: true });
    const converged = (label: string) => {
      net.flush();
      const a = designOf(A.ydoc);
      same(designOf(B.ydoc), a, `${label}: yjs B vs yjs A`);
      same(A.ed.doc, a, `${label}: editor A vs yjs`);
      same(B.ed.doc, a, `${label}: editor B vs yjs`);
    };
    // Sequential: A inserts, B sees it and moves it.
    const [s1] = A.ed.insert([makeShape({ xf: { x: 0, y: 0, w: 50, h: 50, rot: 0 } })], [60, 60]);
    converged('insert');
    expect(B.ed.doc.elements[s1!]).toBeTruthy();
    B.ed.select([s1!]);
    B.ed.nudge(30, 0);
    converged('nudge');
    // Concurrent: both edit before exchanging updates.
    A.ed.select([s1!]);
    A.ed.updateSelected({ opacity: 0.5 });
    B.ed.addPage(null);
    B.ed.insert([makeText({ xf: { x: 0, y: 0, w: 200, h: 40, rot: 0 }, text: 'from B' })]);
    A.ed.insert([makeText({ xf: { x: 0, y: 0, w: 200, h: 40, rot: 0 }, text: 'from A' })]);
    converged('concurrent');
    expect(sortedPages(A.ed.doc).length).toBe(sortedPages(B.ed.doc).length);
    expect(A.ed.doc.elements[s1!]!.opacity).toBe(0.5);
    // Undo is per editor (local origin only): A's undo removes A's last insert on both sides.
    const before = Object.keys(B.ed.doc.elements).length;
    A.ed.undo();
    converged('undo');
    expect(Object.keys(B.ed.doc.elements).length).toBe(before - 1);
    // Late joiner from the encoded state.
    const C = browser(Y.encodeStateAsUpdate(A.ydoc), 'c');
    same(C.ed.doc, A.ed.doc, 'late joiner');
    for (const b of [A, B, C]) b.ed.destroy();
  });

  test('destroying the yjs doc detaches the editor', () => {
    const { ydoc, ed } = browser(seedDesign(), 'a');
    ydoc.destroy();
    expect(() => ydoc.getMap('meta').set('title', 'x')).not.toThrow();
    expect(() => ed.destroy()).not.toThrow();
  });

  test('replaceContent is one undoable local change', () => {
    const { ydoc, ed } = browser(seedDesign(), 'a');
    const original = json(ed.doc);
    const other = fixture('ig-sale-violet.json');
    ed.store.checkpoint();
    ed.store.replaceContent(other);
    ed.store.checkpoint();
    same(ed.doc, designOf(ydoc), 'restored doc in yjs');
    // Replaced elements stay as tombstones (`deleted`), like `@oc/crdt`'s replaceContent.
    const live = (d: DesignDoc) => Object.values(d.elements).filter((e) => !e.deleted).map((e) => e.id).sort();
    expect(live(ed.doc)).toEqual(live(other));
    expect(ed.store.undo.undoStack.length).toBe(1);
    ed.undo();
    expect(live(ed.doc)).toEqual(live(original));
    same(ed.doc, designOf(ydoc), 'undone in yjs');
    ed.destroy();
  });
});
