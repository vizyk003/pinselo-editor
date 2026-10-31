/**
 * Sheets (A15): a spreadsheet design type, collaboratively edited in the design's own Y.Doc (root map
 * "sheet", see @oc/sheet). Virtualised grid (only visible cells are in the DOM, so 10k+ rows scroll
 * smoothly), HyperFormula formulas across tabs, formatting, merges, frozen panes, resizable tracks, live
 * presence, undo/redo, TSV clipboard exchange with other spreadsheets, CSV/XLSX/PDF/PNG export, and
 * charts linked to ranges that can be inserted into any design.
 */
import type { HocuspocusProvider } from '@hocuspocus/provider';
import type { ClipboardPayload } from '@oc/engine';
import { CHART_KINDS, makeChart, type ChartKind } from '@oc/engine';
import {
  a1, addTab, autoAlign, colName, colWidth, DEFAULT_ROW_H, displayRows, fmtAt, formatRange, formatValue, hasSheet, initSheet, keyAt,
  mergeRanges, normRange, parseDelimited, parseRange, patchFormat, rangeToChartData, rangeToTable, rawAt, rawRows, rowHeight, setMerge, setRaw,
  SHEET_LOCAL, SheetEngine, sheetRoot, sheetToXlsx, shiftFormula, snapTab, splitKey, tabToCsv, toDelimited, usedRange, writeRows,
  type CellFormat, type NumberFormat, type Range, type TabSnap,
} from '@oc/engine';
import {
  AlignCenter, AlignLeft, AlignRight, BarChart3, Bold, ChevronLeft, Download, Italic, Plus, Redo2, Share2, Snowflake, Strikethrough, Table as TableIcon,
  TableCellsMerge, Underline, Undo2, Upload, WrapText,
} from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useNavigate } from 'react-router';
import * as Y from 'yjs';
import { api, download, type DesignSummary, type Role, type User } from '../api';
import { Avatar, MenuItem, Modal, PopoverButton, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import { copyPayload, setPendingInsert } from './linkedCharts';
import './sheet.css';

interface Props { ydoc: Y.Doc; provider: HocuspocusProvider; design: DesignSummary & { role: Role }; user: User }

/** Row-header width and column-header height (px). */
const HW = 46;
const HH = 24;
const SHEET_CLIP = 'oc-sheet:';
const CLIP_KIND = 'application/x-opencanvas+json' as const;

interface Sel { ar: number; ac: number; fr: number; fc: number }
interface Edit { r: number; c: number; text: string; via: 'cell' | 'bar' }
interface Remote { id: number; name: string; color: string; sel: Range; r: number; c: number }

const rangeOf = (s: Sel): Range => normRange({ r: s.ar, c: s.ac }, { r: s.fr, c: s.fc });
const b64 = (s: string) => btoa(unescape(encodeURIComponent(s)));
const unb64 = (s: string) => decodeURIComponent(escape(atob(s)));

function prefix(n: number, size: (i: number) => number): Float64Array {
  const p = new Float64Array(n + 1);
  for (let i = 0; i < n; i++) p[i + 1] = p[i]! + size(i);
  return p;
}
/** Index i with pos[i] <= x < pos[i+1], clamped to [0, n-1]. */
function bisect(pos: Float64Array, x: number): number {
  let lo = 0;
  let hi = pos.length - 2;
  if (hi < 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (pos[mid]! <= x) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

const NUMBER_FORMATS: Array<[NumberFormat, string]> = [
  ['auto', tk('Automatic')], ['number', tk('Number')], ['percent', tk('Percent')], ['currency', tk('Currency')],
  ['date', tk('Date')], ['datetime', tk('Date time')], ['text', tk('Plain text')],
];
const SHEET_CHARTS: ChartKind[] = ['column', 'bar', 'stacked-column', 'line', 'area', 'pie', 'donut'];

export default function SheetEditor(props: Props) {
  const { ydoc, design } = props;
  const readOnly = design.role === 'view' || design.role === 'comment';
  const [ctx, setCtx] = useState<{ engine: SheetEngine; undo: Y.UndoManager } | null>(null);
  useEffect(() => {
    const root = sheetRoot(ydoc);
    // Sheets get their first tab server-side; this covers designs created before that (or offline).
    if (!hasSheet(ydoc) && !readOnly) initSheet(ydoc);
    const engine = new SheetEngine(root);
    const undo = new Y.UndoManager(root, { trackedOrigins: new Set([SHEET_LOCAL]), captureTimeout: 400 });
    setCtx({ engine, undo });
    return () => {
      undo.destroy();
      engine.destroy();
    };
  }, [ydoc, readOnly]);
  if (!ctx) return null;
  return <SheetView {...props} engine={ctx.engine} undo={ctx.undo} />;
}

function SheetView({ ydoc, provider, design, user, engine, undo }: Props & { engine: SheetEngine; undo: Y.UndoManager }) {
  const nav = useNavigate();
  const toast = useToast();
  const readOnly = design.role === 'view' || design.role === 'comment';
  const root = engine.root;
  const version = useSyncExternalStore(useCallback((fn: () => void) => engine.subscribe(fn), [engine]), () => engine.version);

  const [title, setTitle] = useState(design.title);
  const [tabId, setTabId] = useState(() => engine.tabs[0]?.id ?? '');
  const tab: TabSnap | undefined = engine.tab(tabId) ?? engine.tabs[0];
  useEffect(() => {
    if (tab && tab.id !== tabId) setTabId(tab.id);
  }, [tab, tabId]);
  const [sel, setSel] = useState<Sel>({ ar: 0, ac: 0, fr: 0, fc: 0 });
  const [edit, setEditState] = useState<Edit | null>(null);
  // Mirrored synchronously: keys typed before the in-cell input mounts (fast typists, IMEs, automation)
  // must append to the draft instead of starting a new one.
  const editRef = useRef<Edit | null>(null);
  const setEdit: React.Dispatch<React.SetStateAction<Edit | null>> = useCallback((v) => {
    editRef.current = typeof v === 'function' ? v(editRef.current) : v;
    setEditState(editRef.current);
  }, []);
  const [chartOpen, setChartOpen] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; kind: 'cell' | 'row' | 'col' } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [remotes, setRemotes] = useState<Remote[]>([]);
  const [peers, setPeers] = useState<Array<{ id: number; name: string; color: string }>>([]);
  const gridRef = useRef<HTMLDivElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const range = rangeOf(sel);
  const rows = tab?.rows.length ?? 0;
  const cols = tab?.cols.length ?? 0;

  const tx = useCallback((fn: () => void) => ydoc.transact(fn, SHEET_LOCAL), [ydoc]);

  // Keep the selection inside the grid when rows/cols are removed (locally or by a collaborator).
  useEffect(() => {
    if (!rows || !cols) return;
    const cl = (v: number, n: number) => Math.max(0, Math.min(n - 1, v));
    if (sel.ar >= rows || sel.fr >= rows || sel.ac >= cols || sel.fc >= cols) setSel({ ar: cl(sel.ar, rows), ac: cl(sel.ac, cols), fr: cl(sel.fr, rows), fc: cl(sel.fc, cols) });
  }, [rows, cols, sel]);

  // ---------------------------------------------------------------- presence (awareness)
  useEffect(() => {
    if (!tab) return;
    const ra = tab.rows[sel.ar];
    const ca = tab.cols[sel.ac];
    const rf = tab.rows[sel.fr];
    const cf = tab.cols[sel.fc];
    provider.awareness?.setLocalStateField('sheet', ra && ca && rf && cf ? { tab: tab.id, a: `${ra}:${ca}`, f: `${rf}:${cf}` } : null);
  }, [provider, tab, sel]);
  useEffect(() => {
    const aw = provider.awareness;
    if (!aw) return;
    const update = () => {
      const list: Remote[] = [];
      const ps: Array<{ id: number; name: string; color: string }> = [];
      const t0 = engine.tab(tabId);
      aw.getStates().forEach((st, id) => {
        if (id === aw.clientID) return;
        const u = st.user as { name: string; color: string } | undefined;
        if (!u) return;
        ps.push({ id, name: u.name, color: u.color });
        const s = st.sheet as { tab: string; a: string; f: string } | null | undefined;
        if (!s || !t0 || s.tab !== t0.id) return;
        const pa = splitKey(s.a);
        const pf = splitKey(s.f);
        const r = t0.rowIx.get(pa[0]);
        const c = t0.colIx.get(pa[1]);
        const r2 = t0.rowIx.get(pf[0]);
        const c2 = t0.colIx.get(pf[1]);
        if (r === undefined || c === undefined || r2 === undefined || c2 === undefined) return;
        list.push({ id, name: u.name, color: u.color, r, c, sel: normRange({ r, c }, { r: r2, c: c2 }) });
      });
      setRemotes(list);
      setPeers(ps);
    };
    update();
    aw.on('change', update);
    return () => aw.off('change', update);
  }, [provider, engine, tabId, version]);

  // ---------------------------------------------------------------- editing
  const focusGrid = () => gridRef.current?.focus({ preventScroll: true });
  const moveTo = useCallback((r: number, c: number, extend = false) => {
    const R = Math.max(0, Math.min(rows - 1, r));
    const C = Math.max(0, Math.min(cols - 1, c));
    setSel((s) => (extend ? { ...s, fr: R, fc: C } : { ar: R, ac: C, fr: R, fc: C }));
  }, [rows, cols]);

  const startEdit = (text?: string, via: Edit['via'] = 'cell') => {
    if (readOnly || !tab) return;
    setEdit({ r: sel.ar, c: sel.ac, text: text ?? rawAt(tab, sel.ar, sel.ac), via });
  };
  const commit = (move: [number, number] | null) => {
    const e = editRef.current;
    if (!e || !tab) return;
    setEdit(null);
    if (e.text !== rawAt(tab, e.r, e.c)) tx(() => setRaw(tab, e.r, e.c, e.text));
    if (move) moveTo(e.r + move[0], e.c + move[1]);
    focusGrid();
  };

  /** Format a range — whole columns are clamped to the used area (+100 rows) so a column click stays cheap. */
  const eachCell = (fn: (r: number, c: number) => void) => {
    if (!tab) return;
    const used = usedRange(tab);
    const r1 = range.r0 === 0 && range.r1 === rows - 1 ? Math.min(range.r1, (used?.r1 ?? 0) + 100) : range.r1;
    const c1 = range.c0 === 0 && range.c1 === cols - 1 ? Math.min(range.c1, (used?.c1 ?? 0) + 5) : range.c1;
    for (let r = range.r0; r <= r1; r++) for (let c = range.c0; c <= c1; c++) fn(r, c);
  };
  const activeFmt: CellFormat = (tab && fmtAt(tab, sel.ar, sel.ac)) || {};
  const format = (patch: Partial<CellFormat> | null) => {
    if (readOnly || !tab) return;
    tx(() => eachCell((r, c) => patchFormat(tab, r, c, patch)));
  };
  const toggle = (k: 'b' | 'i' | 'u' | 's' | 'wrap') => format({ [k]: !activeFmt[k] });

  const clearRange = () => {
    if (readOnly || !tab) return;
    tx(() => eachCell((r, c) => setRaw(tab, r, c, '')));
  };

  const fillDown = () => {
    if (readOnly || !tab || range.r1 === range.r0) return;
    tx(() => {
      for (let c = range.c0; c <= range.c1; c++) {
        const src = rawAt(tab, range.r0, c);
        const f = fmtAt(tab, range.r0, c);
        for (let r = range.r0 + 1; r <= range.r1; r++) {
          setRaw(tab, r, c, shiftFormula(src, r - range.r0, 0));
          patchFormat(tab, r, c, f ? { ...f } : null);
        }
      }
    });
  };

  // ---------------------------------------------------------------- clipboard (TSV + HTML table, formulas kept between sheets)
  const copy = (e: ClipboardEvent, cut: boolean) => {
    if (!tab) return;
    const disp = displayRows(engine, tab, range);
    const raws = rawRows(tab, range);
    const fmts = raws.map((row, i) => row.map((_, j) => fmtAt(tab, range.r0 + i, range.c0 + j) ?? null));
    const html = `<meta charset="utf-8"><!--${SHEET_CLIP}${b64(JSON.stringify({ r0: range.r0, c0: range.c0, raws, fmts }))}--><table>${disp.map((r) => `<tr>${r.map((v) => `<td>${v.replace(/&/g, '&amp;').replace(/</g, '&lt;')}</td>`).join('')}</tr>`).join('')}</table>`;
    e.clipboardData?.setData('text/plain', toDelimited(disp, '\t', '\n'));
    e.clipboardData?.setData('text/html', html);
    e.preventDefault();
    if (cut) clearRange();
  };
  const paste = (e: ClipboardEvent) => {
    if (readOnly || !tab) return;
    const dt = e.clipboardData;
    if (!dt) return;
    e.preventDefault();
    const html = dt.getData('text/html');
    const m = html && new RegExp(`<!--${SHEET_CLIP}([A-Za-z0-9+/=]+)-->`).exec(html);
    let data: string[][];
    let fmts: Array<Array<CellFormat | null>> | null = null;
    if (m) {
      const p = JSON.parse(unb64(m[1]!)) as { r0: number; c0: number; raws: string[][]; fmts: Array<Array<CellFormat | null>> };
      data = p.raws.map((row) => row.map((v) => shiftFormula(v, range.r0 - p.r0, range.c0 - p.c0)));
      fmts = p.fmts;
    } else {
      const text = dt.getData('text/plain').replace(/\r\n?/g, '\n').replace(/\n$/, '');
      data = parseDelimited(text, text.includes('\t') ? '\t' : '\u0000');
    }
    if (!data.length) return;
    // One copied cell fills the whole selection.
    if (data.length === 1 && data[0]!.length === 1 && (range.r1 > range.r0 || range.c1 > range.c0)) {
      const v = data[0]![0]!;
      const f = fmts?.[0]?.[0];
      tx(() => eachCell((r, c) => {
        setRaw(tab, r, c, m ? shiftFormula(v, r - range.r0, c - range.c0) : v);
        if (f !== undefined) patchFormat(tab, r, c, f);
      }));
      return;
    }
    tx(() => {
      writeRows(tab, range.r0, range.c0, data, () => snapTab(tab.id, tab.map));
      const s = snapTab(tab.id, tab.map);
      if (fmts) fmts.forEach((row, i) => row.forEach((f, j) => patchFormat(s, range.r0 + i, range.c0 + j, f)));
    });
    setSel({ ar: range.r0, ac: range.c0, fr: range.r0 + data.length - 1, fc: range.c0 + Math.max(...data.map((r) => r.length)) - 1 });
  };
  const handlers = useRef({ copy, paste });
  handlers.current = { copy, paste };
  useEffect(() => {
    const own = () => document.activeElement === gridRef.current;
    const c = (e: ClipboardEvent) => own() && handlers.current.copy(e, false);
    const x = (e: ClipboardEvent) => own() && !readOnly && handlers.current.copy(e, true);
    const p = (e: ClipboardEvent) => own() && handlers.current.paste(e);
    document.addEventListener('copy', c);
    document.addEventListener('cut', x);
    document.addEventListener('paste', p);
    return () => {
      document.removeEventListener('copy', c);
      document.removeEventListener('cut', x);
      document.removeEventListener('paste', p);
    };
  }, [readOnly]);

  // ---------------------------------------------------------------- keyboard
  const pageRows = Math.max(1, Math.floor((gridRef.current?.clientHeight ?? 600) / DEFAULT_ROW_H) - 2);
  const edge = (r: number, c: number, dr: number, dc: number): [number, number] => {
    if (!tab) return [r, c];
    const filled = (rr: number, cc: number) => rawAt(tab, rr, cc) !== '';
    let rr = r;
    let cc = c;
    const inside = (a: number, b: number) => a >= 0 && b >= 0 && a < rows && b < cols;
    const startFilled = filled(r, c) && inside(r + dr, c + dc) && filled(r + dr, c + dc);
    while (inside(rr + dr, cc + dc)) {
      rr += dr;
      cc += dc;
      if (startFilled ? !inside(rr + dr, cc + dc) || !filled(rr + dr, cc + dc) : filled(rr, cc)) break;
    }
    return [rr, cc];
  };
  const keyRef = useRef<(e: React.KeyboardEvent) => void>(() => {});
  // Open with the active cell focused, so typing goes straight into A1 (and Tab/Enter don't reach the page links).
  const hasTab = Boolean(tab);
  useEffect(() => {
    const a = document.activeElement;
    if (hasTab && (!a || a === document.body)) focusGrid();
  }, [hasTab]); // eslint-disable-line react-hooks/exhaustive-deps
  // Keys typed while nothing has focus (after a toolbar click or a closed menu) belong to the grid, not the page.
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      const a = document.activeElement;
      if (e.defaultPrevented || (a && a !== document.body) || !gridRef.current) return;
      if (['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'].includes(e.key) || document.querySelector('.modal-backdrop, .popover')) return;
      focusGrid();
      keyRef.current(e as unknown as React.KeyboardEvent);
    };
    window.addEventListener('keydown', k, true);
    return () => window.removeEventListener('keydown', k, true);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editRef.current) {
      // The editor input has not taken focus yet: keep feeding the draft.
      if (e.key === 'Enter') { e.preventDefault(); commit([e.shiftKey ? -1 : 1, 0]); }
      else if (e.key === 'Tab') { e.preventDefault(); commit([0, e.shiftKey ? -1 : 1]); }
      else if (e.key === 'Escape') setEdit(null);
      else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey) { e.preventDefault(); setEdit((x) => x && { ...x, text: x.text + e.key }); }
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key;
    const arrows: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    if (arrows[k]) {
      e.preventDefault();
      const [dr, dc] = arrows[k]!;
      const fromR = e.shiftKey ? sel.fr : sel.ar;
      const fromC = e.shiftKey ? sel.fc : sel.ac;
      const [r, c] = mod ? edge(fromR, fromC, dr, dc) : [fromR + dr, fromC + dc];
      moveTo(r, c, e.shiftKey);
      return;
    }
    if (k === 'Tab') { e.preventDefault(); moveTo(sel.ar, sel.ac + (e.shiftKey ? -1 : 1)); return; }
    if (k === 'Enter') { e.preventDefault(); if (e.shiftKey) moveTo(sel.ar - 1, sel.ac); else startEdit(); return; }
    if (k === 'F2') { e.preventDefault(); startEdit(); return; }
    if (k === 'PageDown' || k === 'PageUp') { e.preventDefault(); moveTo(sel.ar + (k === 'PageDown' ? pageRows : -pageRows), sel.ac, e.shiftKey); return; }
    if (k === 'Home') { e.preventDefault(); moveTo(mod ? 0 : sel.ar, 0, e.shiftKey); return; }
    if (k === 'Delete' || k === 'Backspace') { e.preventDefault(); clearRange(); return; }
    if (mod && !e.altKey) {
      const lk = k.toLowerCase();
      if (lk === 'z') { e.preventDefault(); if (e.shiftKey) undo.redo(); else undo.undo(); return; }
      if (lk === 'y') { e.preventDefault(); undo.redo(); return; }
      if (lk === 'b') { e.preventDefault(); toggle('b'); return; }
      if (lk === 'i') { e.preventDefault(); toggle('i'); return; }
      if (lk === 'u') { e.preventDefault(); toggle('u'); return; }
      if (lk === 'd') { e.preventDefault(); fillDown(); return; }
      if (lk === 'a') { e.preventDefault(); setSel({ ar: 0, ac: 0, fr: rows - 1, fc: cols - 1 }); return; }
      return;
    }
    if (k.length === 1 && !e.altKey && !readOnly) {
      e.preventDefault();
      startEdit(k);
    }
  };
  keyRef.current = onKeyDown;

  // ---------------------------------------------------------------- structure
  const insertRows = (where: 'above' | 'below') => tab && !readOnly && engine.insert(tab.id, 'rows', where === 'above' ? range.r0 : range.r1 + 1, range.r1 - range.r0 + 1);
  const insertCols = (where: 'left' | 'right') => tab && !readOnly && engine.insert(tab.id, 'cols', where === 'left' ? range.c0 : range.c1 + 1, range.c1 - range.c0 + 1);
  const deleteRows = () => tab && !readOnly && engine.remove(tab.id, 'rows', range.r0, range.r1 - range.r0 + 1);
  const deleteCols = () => tab && !readOnly && engine.remove(tab.id, 'cols', range.c0, range.c1 - range.c0 + 1);
  const freeze = (k: 'frozenRows' | 'frozenCols', n: number) => tab && !readOnly && tx(() => tab.map.set(k, Math.max(0, Math.min(n, 20))));
  const merges = useMemo(() => (tab ? mergeRanges(tab) : []), [tab, version]); // eslint-disable-line react-hooks/exhaustive-deps
  const inMerge = merges.find((m) => sel.ar >= m.r0 && sel.ar <= m.r1 && sel.ac >= m.c0 && sel.ac <= m.c1);
  const toggleMerge = () => {
    if (!tab || readOnly) return;
    if (inMerge && range.r0 === inMerge.r0 && range.c0 === inMerge.c0 && range.r1 <= inMerge.r1 && range.c1 <= inMerge.c1) tx(() => setMerge(tab, null, { r: sel.ar, c: sel.ac }));
    else if (range.r1 > range.r0 || range.c1 > range.c0) tx(() => setMerge(tab, range));
  };

  // ---------------------------------------------------------------- tabs
  const newTab = () => {
    if (readOnly) return;
    let id = '';
    tx(() => { id = addTab(root, undefined, tab?.id); });
    setTabId(id);
    setSel({ ar: 0, ac: 0, fr: 0, fc: 0 });
    focusGrid();
  };
  const switchTab = (id: string) => {
    setEdit(null);
    setTabId(id);
    setSel({ ar: 0, ac: 0, fr: 0, fc: 0 });
    focusGrid();
  };

  // ---------------------------------------------------------------- import / export
  const saveTitle = async () => {
    const v = title.trim();
    if (v && v !== design.title) await api.updateDesign(design.id, { title: v }).catch(() => {});
  };
  const fileBase = (title || 'sheet').replace(/[\\/:*?"<>|]+/g, '-');
  const exportCsv = () => tab && download(new Blob([`﻿${tabToCsv(engine, tab)}`], { type: 'text/csv;charset=utf-8' }), `${fileBase}${engine.tabs.length > 1 ? ` - ${tab.name}` : ''}.csv`);
  const exportXlsx = async () => {
    try {
      const bytes = await sheetToXlsx(engine, { creator: user.name });
      download(new Blob([bytes as BlobPart], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' }), `${fileBase}.xlsx`);
    } catch (e) {
      toast(t('Export failed: {message}', { message: (e as Error).message }));
    }
  };
  const exportRender = async (format: 'pdf' | 'png') => {
    toast(t('Preparing…'));
    try {
      download(await api.exportDesign(design.id, { format }), `${fileBase}.${format === 'png' ? 'png' : 'pdf'}`);
    } catch (e) {
      toast(t('Export failed: {message}', { message: (e as Error).message }));
    }
  };
  const importFile = async (f: File) => {
    const text = await f.text();
    const first = text.split(/\r?\n/, 1)[0] ?? '';
    const delim = /\.tsv$/i.test(f.name) || first.split('\t').length > first.split(',').length ? '\t' : first.split(';').length > first.split(',').length ? ';' : ',';
    const data = parseDelimited(text, delim);
    let id = '';
    tx(() => {
      id = addTab(root, f.name.replace(/\.[^.]+$/, ''), tab?.id);
      const tabs = root.get('tabs') as Y.Map<Y.Map<unknown>>;
      const s = snapTab(id, tabs.get(id)!);
      writeRows(s, 0, 0, data, () => snapTab(id, tabs.get(id)!));
    });
    switchTab(id);
  };

  // ---------------------------------------------------------------- copy range as a design table
  const copyAsTable = () => {
    if (!tab) return;
    const el = rangeToTable(engine, tab, range);
    const payload: ClipboardPayload = { kind: CLIP_KIND, elements: [el], roots: [el.id], assets: {} };
    if (copyPayload(payload, 'Pinselo table')) toast(t('Copied — paste it into any design'));
  };

  if (!tab) {
    return (
      <div className="doc-shell">
        <header className="editor-header">
          <button className="btn ghost sm" onClick={() => nav('/')}><ChevronLeft size={16} /> {t('Home')}</button>
        </header>
        <p className="muted" style={{ padding: 24 }}>{t('This sheet is empty.')}</p>
      </div>
    );
  }

  const activeRaw = rawAt(tab, sel.ar, sel.ac);
  const btn = (on: boolean, label: string, icon: React.ReactNode, fn: () => void, disabled = false) => (
    <button className={`btn ghost sm icon ${on ? 'active' : ''}`} title={label} aria-label={label} aria-pressed={on} disabled={disabled || readOnly} onMouseDown={(e) => e.preventDefault()} onClick={fn}>{icon}</button>
  );
  const selLabel = formatRange(range);

  return (
    <div className="doc-shell sheet-shell">
      <header className="editor-header">
        <button className="btn ghost sm" onClick={() => nav('/')} aria-label={t('Home')}><ChevronLeft size={16} /><span className="hide-mobile"> {t('Home')}</span></button>
        <span className="surface-label hide-mobile">{t('Sheet')}</span>
        <div className="grow" />
        <input className="title-input" value={title} disabled={readOnly} onChange={(e) => setTitle(e.target.value)} onBlur={saveTitle} onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()} aria-label={t('Document title')} />
        <div className="grow" />
        <div className="row" style={{ gap: 2 }}>
          {peers.slice(0, 5).map((p) => <Avatar key={p.id} name={p.name} color={p.color} size={26} />)}
        </div>
        {!readOnly && <button className="btn ghost sm" onClick={() => fileRef.current?.click()} title={t('Import CSV as a new tab')}><Upload size={16} /><span className="hide-mobile"> {t('Import')}</span></button>}
        <PopoverButton className="btn ghost sm" label={<><Download size={16} /><span className="hide-mobile"> {t('Download')}</span></>} title={t('Download')} content={(close) => (
          <div className="menu">
            <MenuItem label={t('CSV (this tab)')} onClick={() => { close(); exportCsv(); }} />
            <MenuItem label={t('Excel workbook (.xlsx)')} onClick={() => { close(); void exportXlsx(); }} />
            <MenuItem label="PDF" onClick={() => { close(); void exportRender('pdf'); }} />
            <MenuItem label="PNG" onClick={() => { close(); void exportRender('png'); }} />
          </div>
        )} />
        <button className="btn sm white" onClick={() => void navigator.clipboard.writeText(location.href).then(() => toast(t('Link copied — share access from Home')))}><Share2 size={16} /><span className="hide-narrow"> {t('Share')}</span></button>
        <input ref={fileRef} type="file" accept=".csv,.tsv,text/csv,text/tab-separated-values" hidden onChange={(e) => { const f = e.target.files?.[0]; if (f) void importFile(f); e.target.value = ''; }} />
      </header>

      <div className="ctx-toolbar doc-toolbar" role="toolbar" aria-label={t('Formatting')}>
        {btn(false, t('Undo'), <Undo2 size={16} />, () => undo.undo())}
        {btn(false, t('Redo'), <Redo2 size={16} />, () => undo.redo())}
        <div className="sep" />
        <select className="input" style={{ width: 120 }} value={activeFmt.nf ?? 'auto'} disabled={readOnly} aria-label={t('Number format')} onChange={(e) => format({ nf: e.target.value === 'auto' ? undefined : (e.target.value as NumberFormat) })}>
          {NUMBER_FORMATS.map(([v, l]) => <option key={v} value={v}>{t(l)}</option>)}
        </select>
        {btn(false, t('Fewer decimals'), <span style={{ fontSize: 11 }}>.0←</span>, () => format({ dp: Math.max(0, (activeFmt.dp ?? 2) - 1) }))}
        {btn(false, t('More decimals'), <span style={{ fontSize: 11 }}>.00→</span>, () => format({ dp: Math.min(10, (activeFmt.dp ?? 0) + 1) }))}
        <select className="input" style={{ width: 64 }} value={activeFmt.size ?? 13} disabled={readOnly} aria-label={t('Font size')} onChange={(e) => format({ size: Number(e.target.value) === 13 ? undefined : Number(e.target.value) })}>
          {[10, 11, 12, 13, 14, 16, 18, 20, 24, 32].map((s) => <option key={s} value={s}>{s}</option>)}
        </select>
        <div className="sep" />
        {btn(Boolean(activeFmt.b), t('Bold (⌘B)'), <Bold size={16} />, () => toggle('b'))}
        {btn(Boolean(activeFmt.i), t('Italic (⌘I)'), <Italic size={16} />, () => toggle('i'))}
        {btn(Boolean(activeFmt.u), t('Underline (⌘U)'), <Underline size={16} />, () => toggle('u'))}
        {btn(Boolean(activeFmt.s), t('Strikethrough'), <Strikethrough size={16} />, () => toggle('s'))}
        <label className="btn ghost sm icon" title={t('Text colour')}><input type="color" disabled={readOnly} aria-label={t('Text colour')} value={activeFmt.color ?? '#1f2328'} className="sheet-swatch" onChange={(e) => format({ color: e.target.value })} /></label>
        <label className="btn ghost sm icon" title={t('Fill colour')}><input type="color" disabled={readOnly} aria-label={t('Fill colour')} value={activeFmt.fill ?? '#ffffff'} className="sheet-swatch" onChange={(e) => format({ fill: e.target.value === '#ffffff' ? undefined : e.target.value })} /></label>
        <div className="sep" />
        {btn(activeFmt.align === 'left', t('Align left'), <AlignLeft size={16} />, () => format({ align: 'left' }))}
        {btn(activeFmt.align === 'center', t('Align centre'), <AlignCenter size={16} />, () => format({ align: 'center' }))}
        {btn(activeFmt.align === 'right', t('Align right'), <AlignRight size={16} />, () => format({ align: 'right' }))}
        <select className="input" style={{ width: 88 }} value={activeFmt.valign ?? 'bottom'} disabled={readOnly} aria-label={t('Vertical alignment')} onChange={(e) => format({ valign: e.target.value === 'bottom' ? undefined : (e.target.value as CellFormat['valign']) })}>
          <option value="top">{t('Top')}</option><option value="middle">{t('Middle')}</option><option value="bottom">{t('Bottom')}</option>
        </select>
        {btn(Boolean(activeFmt.wrap), t('Wrap text'), <WrapText size={16} />, () => toggle('wrap'))}
        {btn(Boolean(inMerge), inMerge ? t('Unmerge cells') : t('Merge cells'), <TableCellsMerge size={16} />, toggleMerge)}
        <PopoverButton className="btn ghost sm icon" title={t('Freeze')} label={<Snowflake size={16} />} content={(close) => (
          <div className="menu">
            <MenuItem label={t('No frozen rows')} onClick={() => { close(); freeze('frozenRows', 0); }} disabled={readOnly} />
            <MenuItem label={t('Freeze 1 row')} onClick={() => { close(); freeze('frozenRows', 1); }} disabled={readOnly} />
            <MenuItem label={t('Freeze up to row {n}', { n: sel.ar + 1 })} onClick={() => { close(); freeze('frozenRows', sel.ar + 1); }} disabled={readOnly} />
            <MenuItem label={t('No frozen columns')} onClick={() => { close(); freeze('frozenCols', 0); }} disabled={readOnly} />
            <MenuItem label={t('Freeze 1 column')} onClick={() => { close(); freeze('frozenCols', 1); }} disabled={readOnly} />
            <MenuItem label={t('Freeze up to column {n}', { n: colName(sel.ac) })} onClick={() => { close(); freeze('frozenCols', sel.ac + 1); }} disabled={readOnly} />
          </div>
        )} />
        <button className="btn ghost sm" disabled={readOnly} onClick={() => format(null)}>{t('Clear formatting')}</button>
        <div className="sep" />
        <button className="btn ghost sm" onClick={() => setChartOpen(true)} title={t('Make a chart linked to the selected range')}><BarChart3 size={16} /> {t('Chart')}</button>
        <button className="btn ghost sm" onClick={copyAsTable} title={t('Copy the selection as a table for any design')}><TableIcon size={16} /> {t('Copy as table')}</button>
      </div>

      <div className="sheet-formula">
        <input className="input sheet-namebox" aria-label={t('Cell reference')} key={selLabel} defaultValue={selLabel} onKeyDown={(e) => {
          if (e.key !== 'Enter') return;
          const p = parseRange((e.target as HTMLInputElement).value);
          if (p) setSel({ ar: p.range.r0, ac: p.range.c0, fr: p.range.r1, fc: p.range.c1 });
          focusGrid();
        }} />
        <span className="faint sheet-fx">fx</span>
        <input className="input grow sheet-bar" aria-label={t('Formula')} readOnly={readOnly}
          value={edit ? edit.text : activeRaw}
          onFocus={() => !edit && !readOnly && setEdit({ r: sel.ar, c: sel.ac, text: activeRaw, via: 'bar' })}
          onChange={(e) => setEdit((x) => (x ? { ...x, text: e.target.value } : { r: sel.ar, c: sel.ac, text: e.target.value, via: 'bar' }))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') { e.preventDefault(); commit([e.shiftKey ? -1 : 1, 0]); }
            else if (e.key === 'Tab') { e.preventDefault(); commit([0, e.shiftKey ? -1 : 1]); }
            else if (e.key === 'Escape') { setEdit(null); focusGrid(); }
          }}
          onBlur={() => edit?.via === 'bar' && commit(null)} />
      </div>

      <Grid
        engine={engine} tab={tab} version={version} sel={sel} setSel={setSel} edit={edit} setEdit={setEdit} commit={commit}
        remotes={remotes} merges={merges} readOnly={readOnly} gridRef={gridRef} onKeyDown={onKeyDown} tx={tx}
        onMenu={(x, y, kind) => setMenu({ x, y, kind })}
      />

      <div className="sheet-tabs" role="tablist" aria-label={t('Sheets')}>
        {!readOnly && <button className="btn ghost sm icon" onClick={newTab} title={t('Add sheet')} aria-label={t('Add sheet')}><Plus size={16} /></button>}
        {engine.tabs.map((x) => renaming === x.id ? (
          <input key={x.id} className="input sheet-tab-input" autoFocus defaultValue={x.name} aria-label={t('Rename')}
            onBlur={(e) => { engine.renameTab(x.id, e.target.value || x.name); setRenaming(null); }}
            onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); if (e.key === 'Escape') setRenaming(null); }} />
        ) : (
          <button key={x.id} role="tab" aria-selected={x.id === tab.id} className={`sheet-tab ${x.id === tab.id ? 'on' : ''}`} onClick={() => switchTab(x.id)} onDoubleClick={() => !readOnly && setRenaming(x.id)}
            onContextMenu={(e) => { e.preventDefault(); if (!readOnly && engine.tabs.length > 1 && confirm(t('Delete sheet “{name}”?', { name: x.name }))) engine.deleteTab(x.id); }}>
            {x.name}
          </button>
        ))}
        <div className="grow" />
        {!readOnly && <button className="btn ghost sm" onClick={() => engine.insert(tab.id, 'rows', tab.rows.length, 1000)}>{t('Add 1,000 rows')}</button>}
        <span className="faint" style={{ fontSize: 12, paddingRight: 8 }}>{selLabel} · {rows.toLocaleString()} × {cols}</span>
      </div>

      {menu && (
        <div className="popover menu sheet-menu" style={{ left: menu.x, top: menu.y }} role="menu" onMouseLeave={() => setMenu(null)} onClick={() => setMenu(null)}>
          {menu.kind !== 'col' && <>
            <MenuItem label={t('Insert row above')} onClick={() => insertRows('above')} disabled={readOnly} />
            <MenuItem label={t('Insert row below')} onClick={() => insertRows('below')} disabled={readOnly} />
            <MenuItem label={t('Delete row')} onClick={deleteRows} disabled={readOnly} danger />
          </>}
          {menu.kind !== 'row' && <>
            <MenuItem label={t('Insert column left')} onClick={() => insertCols('left')} disabled={readOnly} />
            <MenuItem label={t('Insert column right')} onClick={() => insertCols('right')} disabled={readOnly} />
            <MenuItem label={t('Delete column')} onClick={deleteCols} disabled={readOnly} danger />
          </>}
          {menu.kind === 'cell' && <MenuItem label={t('Fill down (⌘D)')} onClick={fillDown} disabled={readOnly} />}
        </div>
      )}

      {chartOpen && <ChartDialog engine={engine} tab={tab} range={range} designId={design.id} onClose={() => setChartOpen(false)} />}
    </div>
  );
}

// ==================================================================== grid

interface GridProps {
  engine: SheetEngine; tab: TabSnap; version: number; sel: Sel; setSel: React.Dispatch<React.SetStateAction<Sel>>;
  edit: Edit | null; setEdit: React.Dispatch<React.SetStateAction<Edit | null>>; commit: (move: [number, number] | null) => void;
  remotes: Remote[]; merges: Range[]; readOnly: boolean; gridRef: React.RefObject<HTMLDivElement | null>;
  onKeyDown: (e: React.KeyboardEvent) => void; tx: (fn: () => void) => void;
  onMenu: (x: number, y: number, kind: 'cell' | 'row' | 'col') => void;
}

function Grid({ engine, tab, version, sel, setSel, edit, setEdit, commit, remotes, merges, readOnly, gridRef, onKeyDown, tx, onMenu }: GridProps) {
  const boxRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ w: 800, h: 500, x: 0, y: 0 });
  const [resize, setResize] = useState<{ axis: 'col' | 'row'; i: number; size: number } | null>(null);
  const drag = useRef<null | 'cells' | 'rows' | 'cols'>(null);

  const R = tab.rows.length;
  const C = tab.cols.length;
  const colPos = useMemo(() => prefix(C, (c) => (resize?.axis === 'col' && resize.i === c ? resize.size : colWidth(tab, c))), [tab, C, version, resize]); // eslint-disable-line react-hooks/exhaustive-deps
  const rowPos = useMemo(() => prefix(R, (r) => (resize?.axis === 'row' && resize.i === r ? resize.size : rowHeight(tab, r))), [tab, R, version, resize]); // eslint-disable-line react-hooks/exhaustive-deps
  const fr = Math.min(tab.frozenRows, R - 1);
  const fc = Math.min(tab.frozenCols, C - 1);
  const frozenH = rowPos[fr]!;
  const frozenW = colPos[fc]!;

  useLayoutEffect(() => {
    const el = gridRef.current;
    if (!el) return;
    const measure = () => setView((v) => ({ ...v, w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [gridRef]);
  const raf = useRef(0);
  const onScroll = () => {
    cancelAnimationFrame(raf.current);
    raf.current = requestAnimationFrame(() => {
      const el = gridRef.current;
      if (el) setView((v) => ({ ...v, x: el.scrollLeft, y: el.scrollTop }));
    });
  };

  // Screen position of a track start (frozen tracks don't scroll).
  const sx = (c: number) => HW + colPos[c]! - (c >= fc ? view.x : 0);
  const sy = (r: number) => HH + rowPos[r]! - (r >= fr ? view.y : 0);

  // Keep the active cell visible.
  useEffect(() => {
    const el = gridRef.current;
    if (!el || edit) return;
    const r = sel.fr;
    const c = sel.fc;
    if (r >= fr) {
      const top = rowPos[r]! - frozenH;
      const bottom = rowPos[r + 1]! - (el.clientHeight - HH);
      if (top < el.scrollTop) el.scrollTop = top;
      else if (bottom > el.scrollTop) el.scrollTop = bottom;
    }
    if (c >= fc) {
      const left = colPos[c]! - frozenW;
      const right = colPos[c + 1]! - (el.clientWidth - HW);
      if (left < el.scrollLeft) el.scrollLeft = left;
      else if (right > el.scrollLeft) el.scrollLeft = right;
    }
  }, [sel.fr, sel.fc]); // eslint-disable-line react-hooks/exhaustive-deps

  // Visible scrolling tracks.
  const r0 = Math.max(fr, bisect(rowPos, view.y + frozenH));
  const r1 = Math.min(R - 1, bisect(rowPos, view.y + view.h - HH) + 1);
  const c0 = Math.max(fc, bisect(colPos, view.x + frozenW));
  const c1 = Math.min(C - 1, bisect(colPos, view.x + view.w - HW) + 1);
  const frozenRowList = Array.from({ length: fr }, (_, i) => i);
  const frozenColList = Array.from({ length: fc }, (_, i) => i);
  const rowList = Array.from({ length: Math.max(0, r1 - r0 + 1) }, (_, i) => r0 + i);
  const colList = Array.from({ length: Math.max(0, c1 - c0 + 1) }, (_, i) => c0 + i);

  // ---------------------------------------------------------------- hit testing
  const hit = (clientX: number, clientY: number) => {
    const b = gridRef.current!.getBoundingClientRect();
    const x = clientX - b.left;
    const y = clientY - b.top;
    const cx = x - HW < frozenW ? x - HW : x - HW + view.x;
    const cy = y - HH < frozenH ? y - HH : y - HH + view.y;
    return { x, y, r: bisect(rowPos, Math.max(0, cy)), c: bisect(colPos, Math.max(0, cx)), inColHead: y < HH, inRowHead: x < HW };
  };
  const nearEdge = (h: ReturnType<typeof hit>): { axis: 'col' | 'row'; i: number } | null => {
    if (h.inColHead && !h.inRowHead) {
      for (const c of [h.c - 1, h.c]) if (c >= 0 && Math.abs(sx(c + 1) - h.x) <= 4) return { axis: 'col', i: c };
    }
    if (h.inRowHead && !h.inColHead) {
      for (const r of [h.r - 1, h.r]) if (r >= 0 && Math.abs(sy(r + 1) - h.y) <= 3) return { axis: 'row', i: r };
    }
    return null;
  };
  /** While typing a formula, clicking a cell inserts its reference instead of moving. */
  const formulaPointing = edit && edit.text.startsWith('=') && /[=(+\-*/,:&^<>]$/.test(edit.text);

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    const h = hit(e.clientX, e.clientY);
    if (h.inColHead && h.inRowHead) {
      setSel({ ar: 0, ac: 0, fr: R - 1, fc: C - 1 });
      return;
    }
    const edge = !readOnly && nearEdge(h);
    if (edge) {
      e.preventDefault();
      const start = edge.axis === 'col' ? e.clientX : e.clientY;
      const base = edge.axis === 'col' ? colWidth(tab, edge.i) : rowHeight(tab, edge.i);
      let size = base;
      const move = (ev: MouseEvent) => {
        size = Math.max(edge.axis === 'col' ? 24 : 16, base + ((edge.axis === 'col' ? ev.clientX : ev.clientY) - start));
        setResize({ ...edge, size });
      };
      const up = () => {
        window.removeEventListener('mousemove', move);
        window.removeEventListener('mouseup', up);
        setResize(null);
        tx(() => {
          if (edge.axis === 'col') tab.colW.set(tab.cols[edge.i]!, Math.round(size));
          else tab.rowH.set(tab.rows[edge.i]!, Math.round(size));
        });
      };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
      return;
    }
    if (formulaPointing && !h.inColHead && !h.inRowHead) {
      e.preventDefault();
      const base = edit!.text;
      const at = { r: h.r, c: h.c };
      setEdit({ ...edit!, text: base + a1(h.r, h.c) });
      const move = (ev: MouseEvent) => {
        const k = hit(ev.clientX, ev.clientY);
        const rg = normRange(at, { r: k.r, c: k.c });
        setEdit((x) => x && { ...x, text: base + formatRange(rg) });
      };
      const up = () => {
        window.removeEventListener('mousemove', move);
        window.removeEventListener('mouseup', up);
      };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
      return;
    }
    if (edit) commit(null);
    gridRef.current?.focus({ preventScroll: true });
    if (h.inColHead) {
      drag.current = 'cols';
      setSel((s) => (e.shiftKey ? { ...s, fr: R - 1, fc: h.c, ar: 0 } : { ar: 0, ac: h.c, fr: R - 1, fc: h.c }));
    } else if (h.inRowHead) {
      drag.current = 'rows';
      setSel((s) => (e.shiftKey ? { ...s, fr: h.r, fc: C - 1, ac: 0 } : { ar: h.r, ac: 0, fr: h.r, fc: C - 1 }));
    } else {
      drag.current = 'cells';
      // Clicking inside a merged block selects its anchor.
      const m = merges.find((x) => h.r >= x.r0 && h.r <= x.r1 && h.c >= x.c0 && h.c <= x.c1);
      const r = m ? m.r0 : h.r;
      const c = m ? m.c0 : h.c;
      setSel((s) => (e.shiftKey ? { ...s, fr: r, fc: c } : { ar: r, ac: c, fr: m ? m.r1 : r, fc: m ? m.c1 : c }));
    }
    const move = (ev: MouseEvent) => {
      const k = hit(ev.clientX, ev.clientY);
      if (drag.current === 'cols') setSel((s) => ({ ...s, fc: k.c }));
      else if (drag.current === 'rows') setSel((s) => ({ ...s, fr: k.r }));
      else setSel((s) => ({ ...s, fr: k.r, fc: k.c }));
    };
    const up = () => {
      drag.current = null;
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  const onMouseMove = (e: React.MouseEvent) => {
    if (drag.current || !gridRef.current) return;
    const edge = !readOnly && nearEdge(hit(e.clientX, e.clientY));
    gridRef.current.style.cursor = edge ? (edge.axis === 'col' ? 'col-resize' : 'row-resize') : formulaPointing ? 'crosshair' : '';
  };
  const onDoubleClick = (e: React.MouseEvent) => {
    const h = hit(e.clientX, e.clientY);
    if (h.inColHead || h.inRowHead || readOnly) return;
    setEdit({ r: sel.ar, c: sel.ac, text: rawAt(tab, sel.ar, sel.ac), via: 'cell' });
  };
  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    const h = hit(e.clientX, e.clientY);
    const rg = rangeOf(sel);
    const inside = h.r >= rg.r0 && h.r <= rg.r1 && h.c >= rg.c0 && h.c <= rg.c1;
    if (!inside) setSel({ ar: h.r, ac: h.c, fr: h.r, fc: h.c });
    onMenu(e.clientX, e.clientY, h.inColHead ? 'col' : h.inRowHead ? 'row' : 'cell');
  };

  // ---------------------------------------------------------------- rendering
  const covered = (r: number, c: number) => merges.some((m) => r >= m.r0 && r <= m.r1 && c >= m.c0 && c <= m.c1 && !(r === m.r0 && c === m.c0));
  const mergeAt = (r: number, c: number) => merges.find((m) => m.r0 === r && m.c0 === c);
  const cellDiv = (r: number, c: number, ox: number, oy: number) => {
    if (covered(r, c)) return null;
    const m = mergeAt(r, c);
    const w = m ? colPos[m.c1 + 1]! - colPos[c]! : colPos[c + 1]! - colPos[c]!;
    const h = m ? rowPos[m.r1 + 1]! - rowPos[r]! : rowPos[r + 1]! - rowPos[r]!;
    const key = keyAt(tab, r, c);
    const raw = key ? tab.cells.get(key) : undefined;
    const f = key ? tab.fmts.get(key) : undefined;
    if (raw === undefined && !f && !m) return null;
    const v = raw === undefined ? null : engine.value(tab.id, r, c);
    const text = formatValue(v, f, typeof v === 'number' ? engine.kind(tab.id, r, c) : 'other');
    const err = v !== null && typeof v === 'object';
    const style: React.CSSProperties = {
      left: colPos[c]! - ox, top: rowPos[r]! - oy, width: w, height: h,
      justifyContent: { left: 'flex-start', center: 'center', right: 'flex-end' }[autoAlign(v, f)],
      alignItems: { top: 'flex-start', middle: 'center', bottom: 'flex-end' }[f?.valign ?? 'bottom'],
    };
    if (f?.fill) style.background = f.fill;
    if (f?.color) style.color = f.color;
    if (f?.b) style.fontWeight = 700;
    if (f?.i) style.fontStyle = 'italic';
    if (f?.u || f?.s) style.textDecoration = `${f.u ? 'underline' : ''} ${f.s ? 'line-through' : ''}`;
    if (f?.size) style.fontSize = f.size;
    if (f?.wrap) style.whiteSpace = 'pre-wrap';
    if (m) style.zIndex = 1;
    return <div key={`${r}:${c}`} className={`sheet-cell${err ? ' err' : ''}${m ? ' merged' : ''}`} style={style} title={err ? text : undefined}><span>{text}</span></div>;
  };
  /** Cells of one pane plus merges anchored outside it that reach in. */
  const paneCells = (rs: number[], cs: number[], ox: number, oy: number) => {
    const out: React.ReactNode[] = [];
    for (const r of rs) for (const c of cs) out.push(cellDiv(r, c, ox, oy));
    if (rs.length && cs.length) {
      const [ra, rb, ca, cb] = [rs[0]!, rs[rs.length - 1]!, cs[0]!, cs[cs.length - 1]!];
      for (const m of merges) {
        const anchorVisible = m.r0 >= ra && m.r0 <= rb && m.c0 >= ca && m.c0 <= cb;
        const intersects = !(m.r1 < ra || m.r0 > rb || m.c1 < ca || m.c0 > cb);
        if (intersects && !anchorVisible) out.push(cellDiv(m.r0, m.c0, ox, oy));
      }
    }
    return out;
  };
  const rect = (rg: Range, ox: number, oy: number) => ({ left: colPos[rg.c0]! - ox, top: rowPos[rg.r0]! - oy, width: colPos[rg.c1 + 1]! - colPos[rg.c0]!, height: rowPos[rg.r1 + 1]! - rowPos[rg.r0]! });
  const selRange = rangeOf(sel);
  const activeM = mergeAt(sel.ar, sel.ac);
  const activeRange = activeM ?? { r0: sel.ar, c0: sel.ac, r1: sel.ar, c1: sel.ac };
  const overlays = (ox: number, oy: number) => (
    <>
      {remotes.map((p) => (
        <div key={p.id} className="sheet-remote" style={{ ...rect(p.sel, ox, oy), borderColor: p.color }}>
          {p.r === p.sel.r0 && p.c === p.sel.c0 && <span style={{ background: p.color }}>{p.name}</span>}
        </div>
      ))}
      {(selRange.r1 > selRange.r0 || selRange.c1 > selRange.c0) && <div className="sheet-sel" style={rect(selRange, ox, oy)} />}
      <div className="sheet-active" style={rect(activeRange, ox, oy)} />
    </>
  );
  const gridLines = (rs: number[], cs: number[], ox: number, oy: number) => (
    <>
      {rs.map((r) => <div key={`h${r}`} className="sheet-hline" style={{ top: rowPos[r + 1]! - oy - 1 }} />)}
      {cs.map((c) => <div key={`v${c}`} className="sheet-vline" style={{ left: colPos[c + 1]! - ox - 1 }} />)}
    </>
  );
  const pane = (key: string, left: number, top: number, width: number, height: number, rs: number[], cs: number[], ox: number, oy: number) => (
    width > 0 && height > 0 ? (
      <div key={key} className="sheet-pane" style={{ left, top, width, height }}>
        {gridLines(rs, cs, ox, oy)}
        {paneCells(rs, cs, ox, oy)}
        {overlays(ox, oy)}
      </div>
    ) : null
  );
  const bodyW = view.w - HW - frozenW;
  const bodyH = view.h - HH - frozenH;
  const inSel = (lo: number, hi: number, i: number) => i >= lo && i <= hi;
  const colHead = (cs: number[], ox: number) => cs.map((c) => (
    <div key={c} className={`sheet-colhead${inSel(selRange.c0, selRange.c1, c) ? ' on' : ''}`} style={{ left: colPos[c]! - ox, width: colPos[c + 1]! - colPos[c]! }}>{colName(c)}</div>
  ));
  const rowHead = (rs: number[], oy: number) => rs.map((r) => (
    <div key={r} className={`sheet-rowhead${inSel(selRange.r0, selRange.r1, r) ? ' on' : ''}`} style={{ top: rowPos[r]! - oy, height: rowPos[r + 1]! - rowPos[r]! }}>{r + 1}</div>
  ));

  // In-cell editor geometry.
  let editor: React.ReactNode = null;
  if (edit && edit.via === 'cell') {
    const m = mergeAt(edit.r, edit.c);
    const w = m ? colPos[m.c1 + 1]! - colPos[edit.c]! : colPos[edit.c + 1]! - colPos[edit.c]!;
    const h = m ? rowPos[m.r1 + 1]! - rowPos[edit.r]! : rowPos[edit.r + 1]! - rowPos[edit.r]!;
    const f = fmtAt(tab, edit.r, edit.c);
    editor = (
      <input
        className="sheet-editor" autoFocus value={edit.text} aria-label={t('Cell {ref}', { ref: a1(edit.r, edit.c) })}
        style={{ left: sx(edit.c), top: sy(edit.r), minWidth: w, width: Math.max(w, edit.text.length * 7.5 + 16), height: h, fontWeight: f?.b ? 700 : undefined, fontStyle: f?.i ? 'italic' : undefined, fontSize: f?.size }}
        onChange={(e) => setEdit({ ...edit, text: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === 'Enter') { e.preventDefault(); commit([e.shiftKey ? -1 : 1, 0]); }
          else if (e.key === 'Tab') { e.preventDefault(); commit([0, e.shiftKey ? -1 : 1]); }
          else if (e.key === 'Escape') { e.preventDefault(); setEdit(null); gridRef.current?.focus({ preventScroll: true }); }
          else if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !edit.text.startsWith('=')) { e.preventDefault(); commit([e.key === 'ArrowUp' ? -1 : 1, 0]); }
        }}
        onBlur={(e) => {
          // Clicking cells to point at references keeps the editor open.
          if (formulaPointing && e.relatedTarget === gridRef.current) return;
          commit(null);
        }}
      />
    );
  }

  return (
    <div className="sheet-grid" ref={boxRef}>
      <div className="sheet-layer" style={{ width: view.w, height: view.h }}>
        {pane('body', HW + frozenW, HH + frozenH, bodyW, bodyH, rowList, colList, view.x + frozenW, view.y + frozenH)}
        {pane('top', HW + frozenW, HH, bodyW, frozenH, frozenRowList, colList, view.x + frozenW, 0)}
        {pane('left', HW, HH + frozenH, frozenW, bodyH, rowList, frozenColList, 0, view.y + frozenH)}
        {pane('corner', HW, HH, frozenW, frozenH, frozenRowList, frozenColList, 0, 0)}
        <div className="sheet-heads-c" style={{ left: HW + frozenW, width: Math.max(0, bodyW) }}>{colHead(colList, view.x + frozenW)}</div>
        {fc > 0 && <div className="sheet-heads-c" style={{ left: HW, width: frozenW }}>{colHead(frozenColList, 0)}</div>}
        <div className="sheet-heads-r" style={{ top: HH + frozenH, height: Math.max(0, bodyH) }}>{rowHead(rowList, view.y + frozenH)}</div>
        {fr > 0 && <div className="sheet-heads-r" style={{ top: HH, height: frozenH }}>{rowHead(frozenRowList, 0)}</div>}
        <div className="sheet-corner" />
        {fr > 0 && <div className="sheet-freeze-h" style={{ top: HH + frozenH - 1 }} />}
        {fc > 0 && <div className="sheet-freeze-v" style={{ left: HW + frozenW - 1 }} />}
      </div>
      <div
        ref={gridRef} className="sheet-scroll" tabIndex={0} role="grid" aria-label={t('Spreadsheet')} aria-rowcount={R} aria-colcount={C}
        aria-activedescendant={undefined} onScroll={onScroll} onMouseDown={onMouseDown} onMouseMove={onMouseMove} onDoubleClick={onDoubleClick}
        onContextMenu={onContextMenu} onKeyDown={onKeyDown}
      >
        <div style={{ width: HW + colPos[C]!, height: HH + rowPos[R]! }} />
      </div>
      {editor}
      <div className="sr-only" aria-live="polite">{`${a1(sel.ar, sel.ac)} ${formatValue(engine.value(tab.id, sel.ar, sel.ac))}`}</div>
    </div>
  );
}

// ==================================================================== chart from selection

function ChartDialog({ engine, tab, range, designId, onClose }: { engine: SheetEngine; tab: TabSnap; range: Range; designId: string; onClose: () => void }) {
  const nav = useNavigate();
  const toast = useToast();
  const [kind, setKind] = useState<ChartKind>('column');
  const [targets, setTargets] = useState<DesignSummary[] | null>(null);
  // A single cell selected: chart the used block around it.
  const rg = range.r0 === range.r1 && range.c0 === range.c1 ? usedRange(tab) ?? range : range;
  const data = useMemo(() => rangeToChartData(engine.rangeValues(tab.id, rg)), [engine, tab.id, rg.r0, rg.r1, rg.c0, rg.c1]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    void api.designs({}).then((r) => setTargets(r.designs.filter((d) => d.surface !== 'sheet' && d.surface !== 'doc').slice(0, 8))).catch(() => setTargets([]));
  }, []);
  const payload = (): ClipboardPayload => {
    const el = makeChart({ xf: { x: 0, y: 0, w: 800, h: 500, rot: 0 }, kind, labels: data.labels, series: data.series });
    el.source = { designId, sheet: tab.id, sheetName: tab.name, range: formatRange(rg), refreshedAt: Date.now() };
    el.name = `${tab.name}!${formatRange(rg)}`;
    return { kind: CLIP_KIND, elements: [el], roots: [el.id], assets: {} };
  };
  const insertInto = async (id: string | null) => {
    try {
      const target = id ?? (await api.createDesign({ preset: 'presentation', title: `${tab.name} chart` })).id;
      setPendingInsert(payload());
      nav(`/design/${target}`);
    } catch (e) {
      toast((e as Error).message);
    }
  };
  const empty = !data.series.length || !data.labels.length;
  return (
    <Modal title={t('Chart from selection')} onClose={onClose} size="sm">
      <div className="col" style={{ gap: 12 }}>
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {t('Linked to {range}. When the sheet changes, the chart updates in every design it is used in.', { range: `${tab.name}!${formatRange(rg)}` })}
        </p>
        <label className="field">{t('Chart type')}
          <select className="input" value={kind} onChange={(e) => setKind(e.target.value as ChartKind)}>
            {SHEET_CHARTS.filter((k) => CHART_KINDS.includes(k)).map((k) => <option key={k} value={k}>{k}</option>)}
          </select>
        </label>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>{empty ? t('Select a range with numbers first.') : `${data.labels.slice(0, 6).join(', ')}${data.labels.length > 6 ? '…' : ''} · ${data.series.map((s) => s.name).join(', ')}`}</p>
        <button className="btn primary" disabled={empty} onClick={() => { if (copyPayload(payload(), 'Pinselo chart')) { toast(t('Copied — paste it into any design')); onClose(); } }}>{t('Copy linked chart')}</button>
        <div className="faint" style={{ fontSize: 11, textTransform: 'uppercase', letterSpacing: '.06em', fontWeight: 700 }}>{t('Insert into design')}</div>
        <div className="menu" style={{ maxHeight: 220, overflow: 'auto' }}>
          <MenuItem label={t('New presentation')} onClick={() => void insertInto(null)} disabled={empty} />
          {targets === null ? <span className="faint">…</span> : targets.map((d) => <MenuItem key={d.id} label={d.title || t('Untitled')} onClick={() => void insertInto(d.id)} disabled={empty} />)}
        </div>
      </div>
    </Modal>
  );
}

