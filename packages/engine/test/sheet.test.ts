// SPDX-License-Identifier: AGPL-3.0-only
/**
 * Sheets through the facade against tests/golden/sheet/ (recorded from `@oc/sheet` — HyperFormula, exceljs, yjs —
 * by tools/golden/sheet.ts, replayed natively by crates/oc-sheet/tests):
 *
 * * misc.json: the a1 / format / csv / chart helpers, the model write sequence (`misc.model`), sheet → table
 *   layout, `readRange` over the stored TS state;
 * * recalc.json: every edit sequence (cells, formats, row/column insert/delete with formula rewrites, tab
 *   add/rename/delete/reorder, merges, sizes, frozen panes, writeRows) applied exactly like the generator's
 *   `applyStep` — through the facade's writers inside `ydoc.transact(…, SHEET_LOCAL)` and direct yjs writes —
 *   with the tab state and every value/kind/display/formula compared after each step;
 * * sync: two browsers linked like a provider converge; `Y.UndoManager` on `SHEET_LOCAL` undoes facade writes and
 *   structural edits; XLSX export round-trips through the Rust reader.
 */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import * as E from '../src/index';
import { diff, golden, short } from './helpers';

const misc = golden('sheet', 'misc.json');

type Tab = E.TabSnap;

/** Golden string encoding: `{ utf16: [...] }` holds a JS string with unpaired surrogates, which Rust reads as U+FFFD. */
function lossy(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(lossy);
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (Array.isArray(o.utf16)) {
      const u = o.utf16 as number[];
      const hi = (x: number | undefined) => x !== undefined && x >= 0xd800 && x <= 0xdbff;
      const lo = (x: number | undefined) => x !== undefined && x >= 0xdc00 && x <= 0xdfff;
      // Unpaired surrogates become U+FFFD (what a Rust string holds).
      return String.fromCharCode(...u.map((x, i) => ((hi(x) && !lo(u[i + 1])) || (lo(x) && !hi(u[i - 1])) ? 0xfffd : x)));
    }
    return Object.fromEntries(Object.entries(o).map(([k, x]) => [k, lossy(x)]));
  }
  return v;
}

const check = (failures: string[], label: string, actual: unknown, want: unknown) => {
  const d = diff(JSON.parse(JSON.stringify(actual ?? null)), lossy(want));
  if (d) failures.push(`${label}: ${d}`);
};

/** tools/golden/sheet.ts `tabJson`. */
function tabJson(t: Tab) {
  const byPos = <V>(m: { entries(): Iterable<[string, V]> }) => {
    const out: Array<[number, number, V]> = [];
    for (const [k, v] of m.entries()) {
      const [rid, cid] = E.splitKey(k);
      const r = t.rowIx.get(rid);
      const c = t.colIx.get(cid);
      if (r !== undefined && c !== undefined) out.push([r, c, v]);
    }
    out.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    return Object.fromEntries(out.map(([r, c, v]) => [E.a1(r, c), v]));
  };
  const sizes = (m: { entries(): Iterable<[string, number]> }, ix: Map<string, number>) => {
    const out: Array<[number, number]> = [];
    for (const [id, v] of m.entries()) {
      const i = ix.get(id);
      if (i !== undefined) out.push([i, v]);
    }
    out.sort((a, b) => a[0] - b[0]);
    return Object.fromEntries(out.map(([i, v]) => [String(i), v]));
  };
  return {
    name: t.name, rows: t.rows.length, cols: t.cols.length, cells: byPos(t.cells), fmts: byPos(t.fmts),
    colW: sizes(t.colW, t.colIx), rowH: sizes(t.rowH, t.rowIx), merges: E.mergeRanges(t).map(E.formatRange).sort(),
    frozenRows: t.frozenRows, frozenCols: t.frozenCols,
  };
}

/** tools/golden/sheet.ts `evalJson` (non-finite numbers as `{ num }`). */
function evalJson(engine: E.SheetEngine) {
  const out: Record<string, Record<string, unknown>> = {};
  for (const t of engine.tabs) {
    const addrs: Array<[number, number]> = [];
    for (const k of t.cells.keys()) {
      const [rid, cid] = E.splitKey(k);
      const r = t.rowIx.get(rid);
      const c = t.colIx.get(cid);
      if (r !== undefined && c !== undefined) addrs.push([r, c]);
    }
    addrs.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const cells: Record<string, unknown> = {};
    for (const [r, c] of addrs) {
      const v = engine.value(t.id, r, c);
      const kind = engine.kind(t.id, r, c);
      const formula = engine.formula(t.id, r, c);
      cells[E.a1(r, c)] = {
        value: typeof v === 'number' && !Number.isFinite(v) ? { num: String(v) } : v,
        kind,
        display: E.formatValue(v, E.fmtAt(t, r, c), kind),
        ...(formula !== undefined ? { formula } : {}),
      };
    }
    out[t.name] = cells;
  }
  return out;
}

/** A browser-side sheet doc: one tab (rows × cols), ids from Mulberry32(seed) like the generator's `sheetDoc`. */
function sheetDoc(seed: number, rows: number, cols: number, now = 0) {
  const d = new Y.Doc();
  d.clientID = 9999;
  E.attachSheet(d, { seed, now, clientId: 1 });
  E.initSheet(d, { rows, cols });
  return d;
}

type Step = Record<string, any>;

/** tools/golden/sheet.ts `applyStep`, with the facade in place of `@oc/sheet`. */
function applyStep(d: Y.Doc, engine: E.SheetEngine, s: Step) {
  const root = E.sheetRoot(d);
  const tab = (i: number) => engine.tabs[i]!;
  const cell = (a: string) => E.parseRange(a)!.range;
  switch (s.op) {
    case 'set':
      d.transact(() => { for (const [a, v] of Object.entries(s.cells as Record<string, string>)) E.setRaw(tab(s.tab), cell(a).r0, cell(a).c0, v); }, E.SHEET_LOCAL);
      break;
    case 'fmt':
      d.transact(() => { for (const [a, f] of Object.entries(s.fmts as Record<string, Record<string, unknown> | null>)) E.patchFormat(tab(s.tab), cell(a).r0, cell(a).c0, f as never); }, E.SHEET_LOCAL);
      break;
    case 'insert': engine.insert(tab(s.tab).id, s.axis, s.at, s.n); break;
    case 'remove': engine.remove(tab(s.tab).id, s.axis, s.at, s.n); break;
    case 'rename': engine.renameTab(tab(s.tab).id, s.name); break;
    case 'deleteTab': engine.deleteTab(tab(s.tab).id); break;
    case 'addTab': d.transact(() => E.addTab(root, s.name), E.SHEET_LOCAL); break;
    case 'merge':
      d.transact(() => E.setMerge(tab(s.tab), s.range ? cell(s.range) : null, s.anchor ? { r: cell(s.anchor).r0, c: cell(s.anchor).c0 } : undefined), E.SHEET_LOCAL);
      break;
    case 'frozen': d.transact(() => { tab(s.tab).map.set('frozenRows', s.rows); tab(s.tab).map.set('frozenCols', s.cols); }, E.SHEET_LOCAL); break;
    case 'size':
      d.transact(() => {
        const t = tab(s.tab);
        const m = s.axis === 'rows' ? t.rowH : t.colW;
        const id = (s.axis === 'rows' ? t.rows : t.cols)[s.index]!;
        if (s.px === null) m.delete(id);
        else m.set(id, s.px);
      }, E.SHEET_LOCAL);
      break;
    case 'writeRows':
      d.transact(() => { const t = tab(s.tab); E.writeRows(t, cell(s.at).r0, cell(s.at).c0, s.rows, () => E.snapTab(t.id, t.map)); }, E.SHEET_LOCAL);
      break;
    case 'order':
      d.transact(() => {
        const order = root.get('order') as Y.Array<string>;
        const ids = order.toArray();
        order.delete(0, ids.length);
        order.push((s.ids as number[]).map((i) => ids[i]!));
      }, E.SHEET_LOCAL);
      break;
    default: throw new Error(`unknown op ${s.op}`);
  }
}

describe('sheet helpers (misc.json)', () => {
  test('a1, format, csv, chart', () => {
    const f: string[] = [];
    let n = 0;
    const each = (name: string, run: (c: any[]) => [unknown, unknown]) => {
      for (const c of misc[name] as any[][]) {
        const [got, want] = run(c);
        check(f, `${name}(${short(c.slice(0, -1))})`, got, want);
        n++;
      }
    };
    each('colName', (c) => [E.colName(c[0]), c[1]]);
    each('colIndex', (c) => [E.colIndex(c[0]), c[1]]);
    each('parseA1', (c) => [E.parseA1(c[0]), c[1]]);
    each('parseRange', (c) => [E.parseRange(c[0]), c[1]]);
    each('formatRange', (c) => [E.formatRange(c[0]), c[1]]);
    each('quoteSheet', (c) => [E.quoteSheet(c[0]), c[1]]);
    each('shiftFormula', (c) => [E.shiftFormula(c[0], c[1], c[2]), c[3]]);
    const num = (v: unknown) => (typeof v === 'object' && v && 'num' in v ? Number((v as { num: string }).num) : (v as number));
    each('generalNumber', (c) => [E.generalNumber(num(c[0])), c[1]]);
    each('formatValue', (c) => [E.formatValue(c[0], c[1] ?? undefined, c[2]), c[3]]);
    each('autoAlign', (c) => [E.autoAlign(c[0], c[1] ?? undefined), c[2]]);
    each('toDelimited', (c) => [E.toDelimited(c[0], c[1], c[2]), c[3]]);
    each('parseDelimited', (c) => [E.parseDelimited(c[0], c[1]), c[2]]);
    each('quoteField', (c) => [E.quoteField(c[0], c[1]), c[2]]);
    each('rangeToChartData', (c) => [E.rangeToChartData(c[0]), c[1]]);
    const cp = misc.chartPatch;
    check(f, 'chartPatch', E.chartPatch(cp[0], cp[1]), cp[2]);
    each('chartChanged', (c) => [E.chartChanged(c[0], c[1]), c[2]]);
    console.log(`sheet helpers: ${n + 1} cases through the facade`);
    expect(f.slice(0, 20)).toEqual([]);
  });

  test('model writes (misc.model) on a yjs doc', () => {
    const m = misc.model;
    const d = sheetDoc(m.seed, m.rows, m.cols);
    const root = E.sheetRoot(d);
    const engine = new E.SheetEngine(root);
    const tab = () => engine.tabs[0]!;
    const steps: unknown[] = [];
    const snap = (label: string, extra?: [string, unknown]) => {
      const ids = engine.tabs.map((t) => t.id);
      const tabs = engine.tabs.map((t) => ({ id: t.id, rowIds: t.rows, colIds: t.cols, used: E.usedRange(t), ...tabJson(t) }));
      steps.push({ label, ids, order: (root.get('order') as Y.Array<string>).toArray(), tabs, ...(extra ? { [extra[0]]: extra[1] } : {}) });
    };
    snap('init');
    d.transact(() => { E.setRaw(tab(), 1, 1, 'x'); E.setRaw(tab(), 3, 2, ''); E.setRaw(tab(), 5, 3, '=1'); E.setRaw(tab(), 9, 9, 'outside'); });
    snap('setRaw');
    d.transact(() => {
      E.patchFormat(tab(), 1, 1, { b: true, color: '#ff0000', nf: 'number' });
      E.patchFormat(tab(), 1, 1, { b: false, i: true, color: '' });
      E.patchFormat(tab(), 2, 2, { b: false });
      E.patchFormat(tab(), 0, 0, { size: 0, dp: 0, cur: '' });
    });
    snap('patchFormat');
    const rg = (r0: number, c0: number, r1: number, c1: number) => ({ r0, c0, r1, c1 });
    d.transact(() => { E.setMerge(tab(), rg(0, 0, 1, 1)); E.setMerge(tab(), rg(4, 2, 9, 9)); E.setMerge(tab(), rg(3, 3, 3, 3)); });
    snap('setMerge');
    d.transact(() => E.setMerge(tab(), rg(1, 1, 2, 2)));
    snap('setMerge-overlap');
    d.transact(() => E.setMerge(tab(), null, { r: 5, c: 3 }));
    snap('unmerge');
    d.transact(() => { E.insertTracks(tab(), 'rows', 2, 2); E.insertTracks(tab(), 'cols', 100, 1); });
    snap('insertTracks');
    d.transact(() => E.deleteTracks(tab(), 'rows', 1, 2));
    snap('deleteTracks-rows');
    d.transact(() => E.deleteTracks(tab(), 'cols', 0, 100));
    snap('deleteTracks-all-cols');
    const first = engine.tabs[0]!.id;
    const names: string[] = [];
    d.transact(() => {
      for (const [base, except] of [['Sheet1'], ['sheet1'], ["  a!b'c[d]*e?f/g\\h:i  "], ['!!!'], ['Sheet1', first]] as Array<[string, string?]>) names.push(E.uniqueTabName(root, base, except));
      E.addTab(root);
      E.addTab(root, 'Sheet1');
      E.addTab(root, 'After first', first);
      names.push(E.uniqueTabName(root, 'Sheet1'));
    });
    snap('addTab', ['names', names]);
    d.transact(() => E.deleteTab(root, engine.tabs[1]!.id));
    snap('deleteTab');
    d.transact(() => { for (const t of engine.tabs.slice(1)) E.deleteTab(root, t.id); E.deleteTab(root, engine.tabs[0]!.id); });
    snap('deleteTab-last');
    const f: string[] = [];
    check(f, 'model', steps, m.steps);
    expect(f).toEqual([]);
  });

  test('layout and readRange', () => {
    const d = sheetDoc(700, 200, 5);
    const engine = new E.SheetEngine(E.sheetRoot(d));
    const rows = [['Item', 'Qty', 'Price'], ...Array.from({ length: 99 }, (_, i) => [`Item ${i}`, String(i), `=B${i + 2}*2`])];
    d.transact(() => {
      const t = engine.tabs[0]!;
      E.writeRows(t, 0, 0, rows);
      t.map.set('frozenRows', 1);
      const s = engine.tabs[0]!;
      E.patchFormat(s, 0, 0, { b: true, fill: '#eeeeee' });
      E.patchFormat(s, 1, 2, { nf: 'currency', color: '#ff0000', i: true, align: 'center', valign: 'top' as never, size: 20 });
      E.patchFormat(s, 2, 1, { align: 'justify' as never, valign: 'weird' as never, u: true });
      E.setMerge(s, { r0: 0, c0: 1, r1: 0, c1: 2 });
      E.setMerge(s, { r0: 3, c0: 0, r1: 5, c1: 1 });
      s.colW.set(s.cols[0]!, 300);
      s.rowH.set(s.rows[4]!, 60);
    }, E.SHEET_LOCAL);
    const f: string[] = [];
    (misc.rangeToTable as any[]).forEach((c, i) => {
      const got = { ...E.rangeToTable(engine, engine.tabs[0]!, c[0], c[1]), id: 'table' };
      check(f, `rangeToTable[${i}]`, got, { ...c[2], id: 'table' });
    });
    const normalize = (v: any) => {
      const map = new Map<string, string>();
      const pages = Object.entries(v.pages as Record<string, { order: string }>).sort((a, b) => (a[1].order < b[1].order ? -1 : 1));
      pages.forEach(([id], i) => map.set(id, `page${i}`));
      for (const [id, e] of Object.entries(v.elements as Record<string, { parentId: string; z: string }>)) map.set(id, `${map.get(e.parentId) ?? e.parentId}/${e.z}`);
      const rename = (x: any): any => (typeof x === 'string' ? (map.get(x) ?? x) : Array.isArray(x) ? x.map(rename) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, y]) => [map.get(k) ?? k, rename(y)])) : x);
      return rename(v);
    };
    const layouts = misc.sheetToDesign as any[];
    layouts.slice(0, 2).forEach((c, i) => check(f, `sheetToDesign[${i}]`, normalize(E.sheetToDesign(engine, c[0])), normalize(c[1])));
    d.transact(() => { E.addTab(E.sheetRoot(d), 'Empty'); E.addTab(E.sheetRoot(d), 'Other'); }, E.SHEET_LOCAL);
    d.transact(() => { E.setRaw(engine.tabs[2]!, 0, 0, 'solo'); E.setRaw(engine.tabs[2]!, 1, 1, '=1/0'); }, E.SHEET_LOCAL);
    layouts.slice(2).forEach((c, i) => {
      const o = { ...c[0] };
      if (o.tabs && o.tabs[0] !== 'nope') o.tabs = [engine.tabs[2]!.id];
      check(f, `sheetToDesign[${i + 2}]`, normalize(E.sheetToDesign(engine, o)), normalize(c[1]));
    });
    // readRange over the state the TS package stored.
    const rr = misc.readRange;
    const ro = E.openSheetState(new Uint8Array(Buffer.from(rr.state, 'base64')));
    check(f, 'readRange.tabs', ro.tabs.map((t) => t.id), rr.tabs);
    for (const c of rr.cases as any[][]) check(f, `readRange(${c[0]}, ${c[1]})`, E.readRange(ro, c[0], c[1]), c[2]);
    expect(f.slice(0, 20)).toEqual([]);
  });
});

describe('sheet recalc sequences (recalc.json)', () => {
  const g = golden('sheet', 'recalc.json');
  test.each((g.cases as any[]).map((c) => [c.name, c]))('%s', (_name, rc) => {
    const d = sheetDoc(rc.seed, rc.rows, rc.cols, g.now);
    const engine = new E.SheetEngine(E.sheetRoot(d));
    const f: string[] = [];
    (rc.steps as any[]).forEach((st, i) => {
      applyStep(d, engine, st.step);
      const label = `${rc.name}[${i}] ${st.step.op}`;
      check(f, `${label}.tabs`, engine.tabs.map(tabJson), st.tabs);
      check(f, `${label}.values`, evalJson(engine), st.values);
    });
    expect(f.slice(0, 10)).toEqual([]);
    engine.destroy();
  });
});

describe('sheet sync', () => {
  test('two browsers converge; Y.UndoManager undoes facade writes', () => {
    const a = new Y.Doc();
    const b = new Y.Doc();
    const net = Symbol('provider');
    a.on('update', (u: Uint8Array, o: unknown) => o !== net && Y.applyUpdate(b, u, net));
    b.on('update', (u: Uint8Array, o: unknown) => o !== net && Y.applyUpdate(a, u, net));
    E.initSheet(a, { rows: 20, cols: 6 });
    const ea = new E.SheetEngine(E.sheetRoot(a));
    const eb = new E.SheetEngine(E.sheetRoot(b));
    expect(eb.tabs.map((t) => t.id)).toEqual(ea.tabs.map((t) => t.id));
    let notified = 0;
    eb.subscribe(() => notified++);
    const undo = new Y.UndoManager(E.sheetRoot(a), { trackedOrigins: new Set([E.SHEET_LOCAL]), captureTimeout: 0 });
    a.transact(() => { E.setRaw(ea.tabs[0]!, 0, 0, '2'); E.setRaw(ea.tabs[0]!, 0, 1, '=A1*21'); }, E.SHEET_LOCAL);
    expect(eb.value(eb.tabs[0]!.id, 0, 1)).toBe(42);
    expect(notified).toBeGreaterThan(0);
    // Undo / redo on A (yjs `Y.UndoManager` over the facade's writes) flow back into both mirrors.
    undo.undo();
    expect(E.rawAt(eb.tabs[0]!, 0, 1)).toBe('');
    expect(eb.value(eb.tabs[0]!.id, 0, 1)).toBe(null);
    undo.redo();
    expect(eb.value(eb.tabs[0]!.id, 0, 1)).toBe(42);
    // Direct yjs writes (the grid sizes columns this way) reach both mirrors.
    b.transact(() => eb.tabs[0]!.colW.set(eb.tabs[0]!.cols[1]!, 180), E.SHEET_LOCAL);
    expect(E.colWidth(ea.tabs[0]!, 1)).toBe(180);
    // Structural edit with formula rewrite, made on B in one SHEET_LOCAL transaction; A sees the rewritten formula.
    const undoB = new Y.UndoManager(E.sheetRoot(b), { trackedOrigins: new Set([E.SHEET_LOCAL]), captureTimeout: 0 });
    eb.insert(eb.tabs[0]!.id, 'cols', 0, 1);
    expect(ea.tabs[0]!.cols.length).toBe(7);
    expect(E.rawAt(ea.tabs[0]!, 0, 2)).toBe('=B1*21');
    expect(ea.value(ea.tabs[0]!.id, 0, 2)).toBe(42);
    // The insert and its formula rewrite are one undo step on B.
    undoB.undo();
    expect(ea.tabs[0]!.cols.length).toBe(6);
    expect(E.rawAt(ea.tabs[0]!, 0, 1)).toBe('=A1*21');
    expect(ea.value(ea.tabs[0]!.id, 0, 1)).toBe(42);
    // Concurrent edits while offline-ish (both edit, then the network delivers).
    a.transact(() => E.setRaw(ea.tabs[0]!, 5, 1, 'from A'), E.SHEET_LOCAL);
    b.transact(() => E.setRaw(eb.tabs[0]!, 6, 1, 'from B'), E.SHEET_LOCAL);
    const tsv = (e: E.SheetEngine) => E.tabToCsv(e, e.tabs[0]!);
    expect(tsv(ea)).toBe(tsv(eb));
    expect(tsv(ea)).toContain('from A');
    expect(tsv(ea)).toContain('from B');
    ea.destroy();
    eb.destroy();
  });

  test('xlsx export round-trips', async () => {
    const d = sheetDoc(5, 10, 4, 1_700_000_000_000);
    const e = new E.SheetEngine(E.sheetRoot(d));
    d.transact(() => E.writeRows(e.tabs[0]!, 0, 0, [['Item', 'Qty'], ['Pen', '3'], ['Ink', '=B2*2']]), E.SHEET_LOCAL);
    const bytes = await E.sheetToXlsx(e, { creator: 'test' });
    expect(bytes[0]).toBe(0x50); // "PK" zip
    const back = await E.readXlsx(bytes);
    expect(back.Sheet1![0]).toEqual(['Item', 'Qty']);
    expect(back.Sheet1![2]![1]).toEqual({ formula: 'B2*2', result: 6 }); // exceljs' cell value shape
  });
});
