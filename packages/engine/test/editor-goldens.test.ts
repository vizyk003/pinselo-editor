// SPDX-License-Identifier: Apache-2.0
/**
 * Editor golden replay through the facade (ADR-0001 parity method): every session in tests/golden/editor/*.json
 * (recorded from the TS `Editor` by tools/golden/editor.ts, and replayed natively by
 * crates/oc-editor-core/tests/golden_sessions.rs) runs through `@oc/engine`'s `Editor` — the typed facade methods
 * (pointer hot paths, `updateSelected(fn)`, `bulkCreate(Map)`, …), the yjs sync (`$remote` is applied to the
 * browser-side Y.Doc and reaches the Rust mirror through the update handler) and a JS text service (`goldText`, the
 * TS object the goldens were recorded with). After each step the return value, state, document, overrides,
 * undo/redo availability, version and random draws must match, and listeners must have run iff the version moved.
 * Finally the browser Y.Doc (fed only by the facade) must hold the same document as the editor.
 *
 * OC_GOLDEN_CASE=<name> limits the replay to one session.
 */
import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import { Editor, childrenOf, decodeDesign, sortedPages, type DesignDoc } from '../src/index';
import { ROOT, diff, fixture, golden, goldText, json, short, unb64, withoutNullList } from './helpers';

type Step = { m: string; a: unknown[]; r: unknown; s: unknown; v: number; n: number; d?: Delta; o?: unknown; u: [boolean, boolean] };
type Delta = { e?: Record<string, unknown>; p?: Record<string, unknown>; a?: Record<string, unknown>; m?: unknown };

const files = readdirSync(join(ROOT, 'tests/golden/editor'))
  .filter((f) => f.endsWith('.json') && f !== 'modules.json')
  .filter((f) => !process.env.OC_GOLDEN_CASE || f === `${process.env.OC_GOLDEN_CASE}.json`)
  .sort();

function applyDelta(doc: Record<string, any>, d: Delta) {
  for (const [k, field] of [['e', 'elements'], ['p', 'pages'], ['a', 'assets']] as const) {
    const m = d[k];
    if (!m) continue;
    for (const [id, v] of Object.entries(m)) {
      if (v === null) delete doc[field][id];
      else doc[field][id] = v;
    }
  }
  if (d.m) doc.meta = d.m;
}

/** The web TemplatesPanel apply flow (tools/golden/editor.ts `applyTemplate`), through the facade helpers. */
function applyTemplate(ed: Editor, name: string) {
  const tpl = fixture(name) as DesignDoc;
  const { w, h } = ed.doc.meta.dims;
  const imported = ed.templatePages(tpl, { w, h }) as Array<{ page: { id: string }; elements: Array<{ parentId: string }> }>;
  const pages = sortedPages(ed.doc);
  const active = ed.state.activePageId;
  const activeEmpty = childrenOf(ed.doc, active).length === 0;
  const ops: unknown[] = [];
  let after: string = active;
  for (const ip of imported) {
    ops.push({ op: 'page', action: 'add', page: { ...ip.page }, after });
    after = ip.page.id;
    for (const el of ip.elements) ops.push({ op: 'add', el, parent: el.parentId });
  }
  if (activeEmpty && pages.length >= 1) ops.push({ op: 'page', action: 'delete', pageId: active });
  for (const a of Object.values(tpl.assets ?? {})) ops.unshift({ op: 'asset', asset: a });
  const r = ed.apply(ops as never);
  if (r.ok) ed.setActivePage(imported[0]!.page.id as never, true);
  return { ok: r.ok, pages: imported.map((p) => p.page.id) };
}

function invoke(ed: Editor, ydoc: Y.Doc, m: string, a: any[]): unknown {
  switch (m) {
    case '$clock':
      ed.setNow(ed.now() + (a[0] as number));
      return null;
    case '$text':
      ed.textService = a[0] === 'gold' ? goldText : null;
      return null;
    case '$remote':
      Y.applyUpdate(ydoc, unb64(a[0]), 'golden-peer');
      return null;
    case '$template':
      return applyTemplate(ed, a[0]);
    case 'user=':
      ed.user = a[0] ?? null;
      return null;
    case 'brandAdmin=':
      ed.brandAdmin = Boolean(a[0]);
      return null;
    case 'checkpoint':
    case 'store.checkpoint':
      ed.store.checkpoint();
      return null;
    case 'store.apply':
      return ed.store.apply(a[0], a[1]);
    case 'store.actor':
      return ed.store.actor;
    case 'store.actor=':
      ed.store.actor = a[0];
      return null;
    case 'flush':
      ed.flush();
      return null;
    case 'state':
    case 'doc':
    case 'view':
    case 'version':
    case 'editingTableText':
    case 'brandAdmin':
    case 'user':
      return (ed as any)[m];
    case 'overrides':
      return Object.fromEntries(ed.overrides);
    case 'canUndo':
      return ed.store.undo.undoStack.length > 0;
    case 'canRedo':
      return ed.store.undo.redoStack.length > 0;
    case 'clipboard':
      return ed.dispatch('clipboard');
    case 'updateSelectedMap': {
      const patches = a[0] as Record<string, Record<string, unknown> | null>;
      ed.updateSelected((el) => patches[el.id] ?? null, a[1], a[2]);
      return null;
    }
    case 'bulkCreate':
      return ed.bulkCreate(a[0], a[1], a[2] ? new Map(Object.entries(a[2])) : undefined);
    case 'isBrandLocked':
      return ed.isBrandLocked(typeof a[0] === 'string' ? (ed.doc.elements as any)[a[0]] : a[0]);
    default: {
      const f = (ed as any)[m];
      if (typeof f === 'function') return f.apply(ed, a);
      return ed.dispatch(m, ...a);
    }
  }
}

let totalSteps = 0;

describe('editor goldens through the facade', () => {
  test.each(files)('%s', (file) => {
    const g = golden('editor', file);
    const ydoc = new Y.Doc();
    ydoc.clientID = 9999; // not 1 (the recorded TS doc) or 2 (the Rust mirror)
    Y.applyUpdate(ydoc, unb64(g.update));
    const ed = new Editor(ydoc, g.viewport, { clock: 'manual', now: g.clock, seed: g.seed, clientId: 2 });
    if (g.text === 'gold') ed.textService = goldText;
    ed.brandAdmin = g.brandAdmin;
    ed.user = g.user;
    let notified = 0;
    ed.subscribe(() => notified++);
    const expected = json(g.doc);
    const d0 = diff(ed.doc, expected, 'doc');
    expect(d0, `${file}: initial document`).toBeNull();
    const steps = g.steps as Step[];
    steps.forEach((st, i) => {
      const fail = (what: string, d: string) => `${file} step ${i} ${st.m}(${short(st.a)}): ${what}: ${d}`;
      const v0 = ed.version;
      const n0 = notified;
      const r = invoke(ed, ydoc, st.m, st.a);
      const checks: Array<[string, unknown, unknown]> = [
        ['random draws', ed.raw.randomBytesDrawn(), st.n],
        ['return value', withoutNullList(json(r)), withoutNullList(json(st.r))],
        ['state', withoutNullList(json(ed.state)), withoutNullList(json(st.s))],
      ];
      for (const [what, actual, want] of checks) {
        const d = diff(actual, want, what);
        if (d) throw new Error(fail(what, d));
      }
      if (st.d) applyDelta(expected, withoutNullList(json(st.d)));
      const dd = diff(withoutNullList(json(ed.doc)), expected, 'doc');
      if (dd) throw new Error(fail('document', dd));
      const od = diff(withoutNullList(json(Object.fromEntries(ed.overrides))), withoutNullList(json(st.o ?? {})), 'overrides');
      if (od) throw new Error(fail('overrides', od));
      const ud = diff([ed.store.undo.undoStack.length > 0, ed.store.undo.redoStack.length > 0], st.u, 'undo');
      if (ud) throw new Error(fail('undo/redo', ud));
      if (ed.version !== st.v) throw new Error(fail('version', `${ed.version} != ${st.v}`));
      if (st.m !== 'version' && (notified > n0) !== (ed.version !== v0)) throw new Error(fail('listeners', `version ${v0}→${ed.version}, ${notified - n0} notifications`));
    });
    // The browser doc saw only what the facade forwarded: it must hold the editor's document.
    const yd = diff(withoutNullList(json(decodeDesign(Y.encodeStateAsUpdate(ydoc)))), withoutNullList(json(ed.doc)), 'ydoc');
    expect(yd, `${file}: Y.Doc vs editor`).toBeNull();
    totalSteps += steps.length;
    ed.destroy();
  });

  test('replayed steps', () => {
    console.log(`editor goldens through the facade: ${totalSteps} steps in ${files.length} sessions`);
    expect(totalSteps).toBeGreaterThan(0);
  });
});
