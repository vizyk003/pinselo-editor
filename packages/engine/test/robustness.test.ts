// SPDX-License-Identifier: Apache-2.0
/**
 * Robustness of the facade and the Rust editor against the exploratory-QA findings (docs/review/qa-explore-2026-10-22.md):
 *
 * * TXT-01/02: caret steps and deletes next to emoji, ZWJ sequences and combining marks never split a cluster
 *   (a lone surrogate used to make yrs panic and kill the module);
 * * a Rust panic surfaces as an `EnginePanicError` and the editor keeps working on a rebuilt mirror;
 * * SYNC-01: a burst of keystrokes with throwing and re-entrant listeners and yjs observers still forwards every
 *   update (persistence) and keeps notifying.
 */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import { Editor, EnginePanicError, decodeDesign, encodeDesign, makeText, type ElementId } from '../src/index';
import { fixture, goldText } from './helpers';

const VP = { x: 0, y: 0, zoom: 1, width: 1200, height: 900 };

function browser(seed: Uint8Array | null = encodeDesign(fixture('card-business-clean.json'))) {
  const ydoc = new Y.Doc();
  if (seed) Y.applyUpdate(ydoc, seed, 'seed');
  const ed = new Editor({ ydoc, actor: 'a' }, VP);
  ed.textService = goldText;
  return { ydoc, ed };
}

const textOf = (ed: Editor, id: ElementId) => ed.plainText(ed.doc.elements[id] as never);
const ydocText = (ydoc: Y.Doc, id: ElementId) => {
  const el = decodeDesign(Y.encodeStateAsUpdate(ydoc)).elements[id] as { body: { paragraphs: Array<{ runs: Array<{ text: string }> }> } };
  return el.body.paragraphs.map((p) => p.runs.map((r) => r.text).join('')).join('\n');
};

function editText(ed: Editor, text: string): ElementId {
  const [id] = ed.insert([makeText({ xf: { x: 0, y: 0, w: 400, h: 40, rot: 0 }, text })]);
  ed.startTextEdit(id!);
  ed.moveCaret('end', false);
  return id!;
}

describe('TXT-01/02 grapheme clusters', () => {
  test('backspace after an emoji deletes the whole emoji', () => {
    const { ydoc, ed } = browser();
    const id = editText(ed, 'ab');
    ed.typeText('😀');
    ed.deleteText(-1);
    expect(textOf(ed, id)).toBe('ab');
    expect(ydocText(ydoc, id)).toBe('ab');
    expect(ed.state.editingText).toMatchObject({ anchor: 2, focus: 2 });
  });

  test('shift+left selects a whole emoji, typing replaces it', () => {
    const { ydoc, ed } = browser();
    const id = editText(ed, 'ab😀');
    ed.moveCaret('left', true);
    expect(ed.selectedText()).toBe('😀');
    ed.typeText('c');
    expect(textOf(ed, id)).toBe('abc');
    expect(ydocText(ydoc, id)).toBe('abc');
  });

  test('ZWJ sequences, flags and combining marks are single steps', () => {
    const { ed } = browser();
    const family = '👨‍👩‍👧';
    const id = editText(ed, `x${family}é🇵🇱`);
    ed.moveCaret('left', false); // before the flag
    ed.moveCaret('left', false); // before é
    ed.typeText('Y');
    expect(textOf(ed, id)).toBe(`x${family}Yé🇵🇱`);
    ed.moveCaret('left', false);
    ed.deleteText(-1); // the whole family
    expect(textOf(ed, id)).toBe('xYé🇵🇱');
    ed.moveCaret('right', false);
    ed.deleteText(1); // é with its combining mark
    expect(textOf(ed, id)).toBe('xY🇵🇱');
    ed.deleteText(1); // the flag
    expect(textOf(ed, id)).toBe('xY');
  });

  test('a DocOp delete that splits a surrogate pair widens to the whole code point instead of panicking', () => {
    const { ydoc, ed } = browser();
    const id = editText(ed, 'a😀b');
    ed.stopTextEdit();
    const r = ed.store.apply([{ op: 'textDelete', id, offset: 2, len: 1 } as never]);
    expect(r.ok).toBe(true);
    expect(textOf(ed, id)).toBe('ab');
    expect(ydocText(ydoc, id)).toBe('ab');
  });
});

describe('panic recovery', () => {
  test('a Rust panic throws EnginePanicError and the editor keeps working', () => {
    const { ydoc, ed } = browser();
    const id = editText(ed, 'Hello');
    ed.stopTextEdit();
    const v = ed.version;
    let notified = 0;
    ed.subscribe(() => notified++);
    const prevRaw = ed.raw;
    expect(() => ed.dispatch('$panic')).toThrow(EnginePanicError);
    expect(ed.panics).toBe(1);
    expect(ed.raw).not.toBe(prevRaw);
    expect(ed.version).toBeGreaterThan(v);
    expect(notified).toBeGreaterThan(0);
    // The rebuilt editor has the document and still syncs both ways.
    expect(textOf(ed, id)).toBe('Hello');
    ed.startTextEdit(id);
    ed.moveCaret('end', false);
    ed.typeText(' again');
    expect(ydocText(ydoc, id)).toBe('Hello again');
    ydoc.transact(() => ydoc.getMap('meta').set('title', 'From yjs'), 'tiptap');
    expect(ed.doc.meta.title).toBe('From yjs');
    ed.destroy();
  });
});

describe('SYNC-01 burst sync', () => {
  test('200 keystrokes in a burst persist although listeners and yjs observers throw and re-enter', () => {
    const { ydoc, ed } = browser();
    // The "server": a second yjs doc that receives every update the browser doc emits, like a provider.
    const server = new Y.Doc();
    Y.applyUpdate(server, Y.encodeStateAsUpdate(ydoc));
    let n = 0;
    // An observer that throws now and then (a React setState over its limit).
    // It comes before the provider-like observer: plain yjs would skip the provider for that update and then
    // never emit `update` again (its cleanup queue stays stuck); the engine isolates the doc's observers.
    ydoc.on('update', () => {
      if (++n % 7 === 0) throw new Error('observer over its limit');
    });
    ydoc.on('update', (u: Uint8Array) => Y.applyUpdate(server, u, 'provider'));
    const id = editText(ed, '');
    // Listeners: one throws, one re-enters the editor (reads state, sets hover), one counts.
    let calls = 0;
    let reentered = 0;
    ed.subscribe(() => {
      if (calls % 5 === 0) throw new Error('listener threw');
    });
    ed.subscribe(() => {
      if (ed.state.editingText && reentered++ % 3 === 0) ed.setHover(null);
    });
    ed.subscribe(() => calls++);
    const errors: unknown[] = [];
    const prev = console.error;
    console.error = (...a: unknown[]) => errors.push(a);
    try {
      const want = 'abcd '.repeat(40);
      for (const ch of want) ed.typeText(ch);
      expect(want.length).toBe(200);
      expect(textOf(ed, id)).toBe(want);
      expect(ydocText(ydoc, id)).toBe(want);
      expect(ydocText(server, id)).toBe(want);
      // Nothing is left pending in the server doc (every update arrived and integrated).
      expect(Y.encodeStateVector(server)).toEqual(Y.encodeStateVector(ydoc));
      expect(calls).toBeGreaterThanOrEqual(200);
      // Later, slow typing still syncs.
      ed.typeText('MORE');
      expect(ydocText(server, id)).toBe(`${want}MORE`);
    } finally {
      console.error = prev;
    }
    expect(errors.length).toBeGreaterThan(0);
    ed.destroy();
  });

  test('a listener that calls back into the editor does not nest notifications', () => {
    const { ed } = browser();
    const id = editText(ed, 'x');
    let depth = 0;
    let maxDepth = 0;
    ed.subscribe(() => {
      depth++;
      maxDepth = Math.max(maxDepth, depth);
      if (ed.state.hover !== id) ed.setHover(id);
      depth--;
    });
    ed.typeText('y');
    expect(ed.state.hover).toBe(id);
    expect(maxDepth).toBe(1);
  });
});

describe('COL-02 concurrent typing in one text box', () => {
  /** Link two yjs docs like a provider; `manual` queues updates until `flush` (concurrent edits). */
  function link(a: Y.Doc, b: Y.Doc) {
    const queue: Array<[Y.Doc, Uint8Array]> = [];
    const origin = Symbol('provider');
    const send = (to: Y.Doc) => (u: Uint8Array, o: unknown) => {
      if (o !== origin) queue.push([to, u]);
    };
    a.on('update', send(b));
    b.on('update', send(a));
    return { flush: () => { for (const [to, u] of queue.splice(0)) Y.applyUpdate(to, u, origin); } };
  }

  function pair() {
    const A = browser();
    const ids = A.ed.insert([makeText({ xf: { x: 0, y: 0, w: 400, h: 40, rot: 0 }, text: 'Hello world' })]);
    const B = browser(Y.encodeStateAsUpdate(A.ydoc));
    return { A, B, id: ids[0]!, net: link(A.ydoc, B.ydoc) };
  }

  for (const burst of [1, 3]) {
    test(`typing at the start and the end at once keeps both inserts intact (burst ${burst})`, () => {
      const { A, B, id, net } = pair();
      A.ed.startTextEdit(id);
      A.ed.moveCaret('end', false);
      B.ed.startTextEdit(id);
      B.ed.moveCaret('home', false);
      const ta = [...'12345'];
      const tb = [...'ab'];
      for (let i = 0; i < Math.max(ta.length, tb.length); i += burst) {
        for (let k = i; k < i + burst; k++) {
          if (ta[k]) A.ed.typeText(ta[k]!);
          if (tb[k]) B.ed.typeText(tb[k]!);
        }
        net.flush();
      }
      expect(textOf(A.ed, id)).toBe('abHello world12345');
      expect(textOf(B.ed, id)).toBe('abHello world12345');
      expect(A.ed.state.editingText).toMatchObject({ anchor: 18, focus: 18 });
      expect(B.ed.state.editingText).toMatchObject({ anchor: 2, focus: 2 });
    });
  }

  test('typing at the same spot keeps each run contiguous, and a selection follows remote inserts', () => {
    const { A, B, id, net } = pair();
    A.ed.startTextEdit(id);
    A.ed.moveCaret('end', false);
    B.ed.startTextEdit(id);
    B.ed.moveCaret('end', false);
    for (const [x, y] of [['a1', 'b1'], ['a2', 'b2'], ['a3', 'b3']] as const) {
      A.ed.typeText(x);
      B.ed.typeText(y);
      net.flush();
    }
    const t = textOf(A.ed, id);
    expect(textOf(B.ed, id)).toBe(t);
    expect(t).toContain('a1a2a3');
    expect(t).toContain('b1b2b3');
    // B selects "world"; A types in front of it: the selection still covers "world".
    B.ed.startTextEdit(id);
    B.ed.moveCaret('home', false);
    for (let i = 0; i < 6; i++) B.ed.moveCaret('right', false);
    for (let i = 0; i < 5; i++) B.ed.moveCaret('right', true);
    expect(B.ed.selectedText()).toBe('world');
    A.ed.startTextEdit(id);
    A.ed.moveCaret('home', false);
    A.ed.typeText('>> ');
    net.flush();
    expect(B.ed.selectedText()).toBe('world');
  });
});
