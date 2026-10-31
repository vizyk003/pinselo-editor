// SPDX-License-Identifier: AGPL-3.0-only
/**
 * `@oc/sheet` exports used by apps/web (`apps/web/src/sheet/*`), backed by crates/oc-sheet (the formula engine,
 * model writes, CSV, XLSX, charts, table layout).
 *
 * The browser's yjs `Y.Doc` stays the source of truth: the grid writes some `Y.Map`s directly
 * (`tab.colW.set(…)`, `tab.map.set('frozenRows', …)`), `Y.UndoManager` tracks `SHEET_LOCAL` transactions, and the
 * provider syncs it. One yrs mirror per yjs doc (created on first use, kept until the doc is destroyed) holds the
 * live formula engine:
 *
 * * every yjs update not made by the mirror is applied to it (remote edits, direct grid writes, undo/redo);
 * * the model writes (`setRaw`, `patchFormat`, `setMerge`, `writeRows`, `addTab`, `initSheet`, …) and the engine's
 *   structural edits run on the mirror; its update is applied to the yjs doc at once — inside the caller's
 *   `ydoc.transact(…, SHEET_LOCAL)` when there is one, so undo records it under that origin as before, else in a
 *   transaction of its own (`SHEET_LOCAL` for the engine's structural edits, like TS).
 *
 * `TabSnap`s keep the TS shape: plain row/column ids plus the tab's live yjs maps, so the grid's direct reads and
 * writes keep working. The few accessors that only read those maps (`sheetRoot`, `snapTab`, `keyAt`, `rawAt`,
 * `fmtAt`, `colWidth`, `rowHeight`) are yjs plumbing and stay here; everything that computes runs in Rust.
 */
import type * as SH from './types/sheet';
import type { CellValue, TabSnap } from './types/sheet';
import * as Y from 'yjs';
import { onEngineReady, raw } from './wasm';

/** The origin of the grid's own transactions (`Y.UndoManager` tracks it). */
export const SHEET_LOCAL: unique symbol = Symbol('sheet-local');

export let SHEET_ROOT: typeof SH.SHEET_ROOT;
export let DEFAULT_ROWS: typeof SH.DEFAULT_ROWS;
export let DEFAULT_COLS: typeof SH.DEFAULT_COLS;
export let DEFAULT_COL_W: typeof SH.DEFAULT_COL_W;
export let DEFAULT_ROW_H: typeof SH.DEFAULT_ROW_H;
export let MAX_ROWS: typeof SH.MAX_ROWS;
export let MAX_COLS: typeof SH.MAX_COLS;

onEngineReady(() => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- the TS constants' values (checked by the tests)
  const c = raw.sheetConstants() as any;
  ({ SHEET_ROOT, DEFAULT_ROWS, DEFAULT_COLS, DEFAULT_COL_W, DEFAULT_ROW_H, MAX_ROWS, MAX_COLS } = c);
});

// ------------------------------------------------------------------ the mirror

/** Marks a yjs transaction made only of mirror updates (its update event is not fed back to the mirror). */
const PUSHED = Symbol('oc-sheet-mirror');

export interface SheetMirrorOptions {
  /** yrs client id of the mirror (default random, different from the yjs doc's). */
  clientId?: number;
  /** Deterministic row/column/tab ids (Mulberry32 seed); default Web Crypto. */
  seed?: number;
  /** Fixed clock for `NOW()`/`TODAY()` (ms); default the browser clock and timezone. */
  now?: number;
}

class SheetMirror {
  readonly raw: raw.SheetEngine;
  private version: number;
  private readonly listeners = new Set<() => void>();
  private readonly onUpdate = (update: Uint8Array, _origin: unknown, _doc: Y.Doc, tr: Y.Transaction) => {
    if (tr?.meta.get(PUSHED)) return;
    this.after(this.raw.applyRemoteUpdate(update));
  };

  constructor(readonly ydoc: Y.Doc, opts: SheetMirrorOptions & { live?: boolean } = {}) {
    let clientId = opts.clientId;
    while (clientId === undefined || clientId === ydoc.clientID) clientId = crypto.getRandomValues(new Uint32Array(1))[0]!;
    this.raw = new raw.SheetEngine(Y.encodeStateAsUpdate(ydoc), { clientId, seed: opts.seed, now: opts.now, live: opts.live !== false });
    this.version = this.raw.version;
    if (opts.live !== false) {
      ydoc.on('update', this.onUpdate);
      ydoc.on('destroy', () => {
        ydoc.off('update', this.onUpdate);
        mirrors.delete(ydoc);
        this.raw.free();
      });
    }
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** After a Rust call: apply the mirror's updates to the yjs doc, notify when the engine version moved. */
  after(status: number, origin: unknown = null): void {
    if (status % 2 === 1) {
      const ups = this.raw.drainLocalUpdates() as Uint8Array[];
      if (ups.length) {
        // Inside the caller's transaction the updates join it (and its origin); otherwise they get their own.
        const own = (this.ydoc as unknown as { _transaction: unknown })._transaction === null;
        Y.transact(this.ydoc, (tr) => {
          const local = tr.local;
          for (const u of ups) Y.applyUpdate(this.ydoc, u, origin);
          tr.local = local; // applyUpdate marks the transaction remote; these are local edits
          if (own) tr.meta.set(PUSHED, true);
        }, origin);
      }
    }
    const v = Math.floor(status / 2);
    if (v !== this.version) {
      this.version = v;
      for (const l of this.listeners) l();
    }
  }
}

const mirrors = new WeakMap<Y.Doc, SheetMirror>();

/**
 * The mirror of a yjs doc, created with `opts` if it does not exist yet (engine addition: tests pass a seed and a
 * fixed clock; the browser never needs to call it).
 */
export function attachSheet(ydoc: Y.Doc, opts: SheetMirrorOptions = {}): void {
  if (!mirrors.has(ydoc)) mirrors.set(ydoc, new SheetMirror(ydoc, opts));
}

function mirrorOf(ydoc: Y.Doc | null | undefined): SheetMirror {
  if (!ydoc) throw new Error('the sheet map is not attached to a Y.Doc');
  let m = mirrors.get(ydoc);
  if (!m) mirrors.set(ydoc, (m = new SheetMirror(ydoc)));
  return m;
}

const tabMirror = (t: TabSnap) => mirrorOf(t.map.doc);

// ------------------------------------------------------------------ yjs plumbing (reads of the browser doc)

export const sheetRoot: typeof SH.sheetRoot = (ydoc) => ydoc.getMap<unknown>(SHEET_ROOT);

export const snapTab: typeof SH.snapTab = (id, map) => {
  const rows = (map.get('rows') as Y.Array<string>).toArray();
  const cols = (map.get('cols') as Y.Array<string>).toArray();
  return {
    id,
    name: String(map.get('name') ?? id),
    map,
    rows,
    cols,
    rowIx: new Map(rows.map((r, i) => [r, i])),
    colIx: new Map(cols.map((c, i) => [c, i])),
    cells: map.get('cells') as TabSnap['cells'],
    fmts: map.get('fmts') as TabSnap['fmts'],
    colW: map.get('colW') as TabSnap['colW'],
    rowH: map.get('rowH') as TabSnap['rowH'],
    merges: map.get('merges') as TabSnap['merges'],
    frozenRows: Number(map.get('frozenRows') ?? 0),
    frozenCols: Number(map.get('frozenCols') ?? 0),
  };
};

export const keyAt: typeof SH.keyAt = (t, r, c) => {
  const row = t.rows[r];
  const col = t.cols[c];
  return row && col ? `${row}:${col}` : null;
};
export const rawAt: typeof SH.rawAt = (t, r, c) => {
  const k = keyAt(t, r, c);
  return k ? (t.cells.get(k) ?? '') : '';
};
export const fmtAt: typeof SH.fmtAt = (t, r, c) => {
  const k = keyAt(t, r, c);
  return k ? t.fmts.get(k) : undefined;
};
export const colWidth: typeof SH.colWidth = (t, c) => t.colW.get(t.cols[c] ?? '') ?? DEFAULT_COL_W;
export const rowHeight: typeof SH.rowHeight = (t, r) => t.rowH.get(t.rows[r] ?? '') ?? DEFAULT_ROW_H;

// ------------------------------------------------------------------ model (computed in Rust)

export const hasSheet: typeof SH.hasSheet = (ydoc) => mirrorOf(ydoc).raw.hasSheet();
export const usedRange: typeof SH.usedRange = (t) => tabMirror(t).raw.usedRange(t.id);
export const mergeRanges: typeof SH.mergeRanges = (t) => tabMirror(t).raw.mergeRanges(t.id);
export const rawRows: typeof SH.rawRows = (t, r) => tabMirror(t).raw.rawRows(t.id, r);
export const splitKey: typeof SH.splitKey = (k) => raw.splitKey(k) as [string, string];

/** `initSheet(ydoc, { name?, rows?, cols?, origin? })`: one empty tab unless a sheet exists; the first tab id. */
export const initSheet: typeof SH.initSheet = (ydoc, opts = {}) => {
  const m = mirrorOf(ydoc);
  const id = m.raw.initSheet(opts.name, opts.rows, opts.cols);
  m.after(m.raw.status(), opts.origin ?? null);
  return id;
};

/** Writes: call inside `ydoc.transact` (the mirror's update joins that transaction). */
export const setRaw: typeof SH.setRaw = (t, r, c, value) => {
  const m = tabMirror(t);
  m.after(m.raw.setRaw(t.id, r, c, value));
};

export const patchFormat: typeof SH.patchFormat = (t, r, c, patch) => {
  const m = tabMirror(t);
  // `undefined` removes a key in TS; Rust reads `null` as remove.
  const p = patch && Object.fromEntries(Object.entries(patch).map(([k, v]) => [k, v === undefined ? null : v]));
  m.after(m.raw.patchFormat(t.id, r, c, p));
};

export const setMerge: typeof SH.setMerge = (t, r, anchor) => {
  const m = tabMirror(t);
  m.after(m.raw.setMerge(t.id, r, anchor));
};

export const insertTracks: typeof SH.insertTracks = (t, axis, at, n) => {
  const m = tabMirror(t);
  m.after(m.raw.insertTracks(t.id, axis, at, n));
};

export const deleteTracks: typeof SH.deleteTracks = (t, axis, at, n) => {
  const m = tabMirror(t);
  m.after(m.raw.deleteTracks(t.id, axis, at, n));
};

/** `writeRows(t, r, c, rows, snap?)`: grows the grid when needed (the Rust side re-snaps; `snap` is not needed). */
export const writeRows: typeof SH.writeRows = (t, r, c, rows) => {
  const m = tabMirror(t);
  m.after(m.raw.writeRows(t.id, r, c, rows));
};

export const addTab: typeof SH.addTab = (root, name, after) => {
  const m = mirrorOf(root.doc);
  const id = m.raw.addTab(name, after);
  m.after(m.raw.status());
  return id;
};

export const uniqueTabName: typeof SH.uniqueTabName = (root, base, except) => mirrorOf(root.doc).raw.uniqueTabName(base, except);

/** The model write (no formula rewrite); `engine.deleteTab` also rewrites formulas referencing the tab. */
export const deleteTab: typeof SH.deleteTab = (root, id) => {
  const m = mirrorOf(root.doc);
  m.after(m.raw.deleteTabModel(id));
};

// ------------------------------------------------------------------ engine

type TSSheetEngine = SH.SheetEngine;
/** Members of the TS engine provided with the same signatures (`hf`, HyperFormula itself, is not). */
type SheetEngineApi = Omit<Pick<TSSheetEngine, keyof TSSheetEngine>, 'hf'>;

export class SheetEngine implements SheetEngineApi {
  readonly root: Y.Map<unknown>;
  private readonly m: SheetMirror;
  private readonly listeners = new Set<() => void>();
  private readonly off: () => void;
  private _tabs: TabSnap[] = [];
  private _rev = -1;

  constructor(root: Y.Map<unknown>, opts: { live?: boolean } = {}) {
    this.root = root;
    this.m = opts.live === false ? new SheetMirror(root.doc!, { live: false }) : mirrorOf(root.doc);
    this.off = this.m.subscribe(() => {
      for (const l of this.listeners) l();
    });
  }

  /** The wasm-bindgen handle of the mirror (shared by every engine on the same doc). */
  get raw(): raw.SheetEngine {
    return this.m.raw;
  }

  get ydoc(): Y.Doc {
    return this.root.doc!;
  }

  /** Bumped on every change (cheap memo key for UIs). */
  get version(): number {
    return this.m.raw.version;
  }

  /** Tab snapshots in display order; re-snapped only when a tab's structure changes (identity kept otherwise). */
  get tabs(): TabSnap[] {
    const rev = this.m.raw.structureRev;
    if (rev !== this._rev) {
      const maps = this.root.get('tabs') as Y.Map<Y.Map<unknown>> | undefined;
      this._tabs = maps ? this.m.raw.tabIds().filter((id) => maps.has(id)).map((id) => snapTab(id, maps.get(id)!)) : [];
      this._rev = rev;
    }
    return this._tabs;
  }

  destroy(): void {
    this.off();
    this.listeners.clear();
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** The mirror follows the document by itself; this forces a full recompute. */
  rebuild(): void {
    this.m.after(this.m.raw.rebuild());
  }

  tab(id: string): TabSnap | undefined {
    return this.tabs.find((t) => t.id === id);
  }

  value(tabId: string, r: number, c: number): CellValue {
    return this.m.raw.value(tabId, r, c) as CellValue;
  }

  kind(tabId: string, r: number, c: number): SH.ValueKind {
    return this.m.raw.kind(tabId, r, c) as SH.ValueKind;
  }

  rangeValues(tabId: string, r: SH.Range): CellValue[][] {
    return this.m.raw.rangeValues(tabId, r) as CellValue[][];
  }

  /** Insert rows/cols (references shift; rewritten formulas are written back in the same `SHEET_LOCAL` transaction). */
  insert(tabId: string, axis: 'rows' | 'cols', at: number, n: number): void {
    this.m.after(this.m.raw.insert(tabId, axis, at, n), SHEET_LOCAL);
  }

  remove(tabId: string, axis: 'rows' | 'cols', at: number, n: number): void {
    this.m.after(this.m.raw.remove(tabId, axis, at, n), SHEET_LOCAL);
  }

  renameTab(tabId: string, name: string): string {
    const n = this.m.raw.renameTab(tabId, name);
    this.m.after(this.m.raw.status(), SHEET_LOCAL);
    return n;
  }

  deleteTab(tabId: string): void {
    this.m.after(this.m.raw.deleteTab(tabId), SHEET_LOCAL);
  }

  /** The workbook's normalised formula text of a cell (engine addition; TS read it from HyperFormula). */
  formula(tabId: string, r: number, c: number): string | undefined {
    return this.m.raw.formula(tabId, r, c);
  }
}

/** `openSheetState(state)`: a read-only engine over a stored sheet design. */
export const openSheetState = (state: Uint8Array): SheetEngine => {
  const d = new Y.Doc();
  Y.applyUpdate(d, state);
  return new SheetEngine(sheetRoot(d), { live: false });
};

// ------------------------------------------------------------------ engine-backed helpers

/** A TS signature with this facade's `SheetEngine` in place of `@oc/sheet`'s (the rest is checked as is). */
type WithEngine<F> = F extends (engine: TSSheetEngine, ...rest: infer A) => infer R ? (engine: SheetEngine, ...rest: A) => R : never;

export const displayRows: WithEngine<typeof SH.displayRows> = (engine, t, r) => engine.raw.displayRows(t.id, r);
export const tabToCsv: WithEngine<typeof SH.tabToCsv> = (engine, t) => engine.raw.tabToCsv(t.id);
export const rangeToTable: WithEngine<typeof SH.rangeToTable> = (engine, t, range, opts = {}) => engine.raw.rangeToTable(t.id, range, opts);
export const sheetToDesign: WithEngine<typeof SH.sheetToDesign> = (engine, opts) => engine.raw.sheetToDesign(opts);
export const readRange: WithEngine<typeof SH.readRange> = (engine, sheet, range) => engine.raw.readRange(sheet, range);

/** `@oc/sheet/xlsx` `sheetToXlsx(engine, { creator? })` (formula cells carry their cached results). */
export const sheetToXlsx = async (engine: SheetEngine, opts: { creator?: string } = {}): Promise<Uint8Array> => engine.raw.toXlsx(opts.creator, Date.now());

/** `@oc/sheet/xlsx` `readXlsx(bytes)`. */
export const readXlsx = async (bytes: Uint8Array): Promise<Record<string, Array<Array<unknown>>>> => raw.readXlsx(bytes);

// ------------------------------------------------------------------ pure helpers

export const a1: typeof SH.a1 = (r, c) => raw.a1(r, c);
export const colName: typeof SH.colName = (c) => raw.colName(c);
export const colIndex: typeof SH.colIndex = (name) => raw.colIndex(name);
export const parseA1: typeof SH.parseA1 = (s) => raw.parseA1(s);
export const normRange: typeof SH.normRange = (a, b) => raw.normRange(a, b);
export const parseRange: typeof SH.parseRange = (s) => raw.parseRange(s);
export const formatRange: typeof SH.formatRange = (r) => raw.formatRange(r);
export const quoteSheet: typeof SH.quoteSheet = (name) => raw.quoteSheet(name);
export const shiftFormula: typeof SH.shiftFormula = (src, dr, dc) => raw.shiftFormula(src, dr, dc);
export const generalNumber: typeof SH.generalNumber = (n) => raw.generalNumber(n);
export const formatValue: typeof SH.formatValue = (v, fmt, kind) => raw.formatValue(v, fmt, kind);
export const autoAlign: typeof SH.autoAlign = (v, fmt) => raw.autoAlign(v, fmt) as ReturnType<typeof SH.autoAlign>;
export const parseDelimited: typeof SH.parseDelimited = (text, delim) => raw.parseDelimited(text, delim);
export const quoteField: typeof SH.quoteField = (s, delim) => raw.quoteField(s, delim);
export const toDelimited: typeof SH.toDelimited = (rows, delim, eol) => raw.toDelimited(rows, delim, eol);
export const rangeToChartData: typeof SH.rangeToChartData = (values) => raw.rangeToChartData(values);
export const chartPatch: typeof SH.chartPatch = (el, data, source) => raw.chartPatch(el, data, source);
export const chartChanged: typeof SH.chartChanged = (el, data) => raw.chartChanged(el, data);
