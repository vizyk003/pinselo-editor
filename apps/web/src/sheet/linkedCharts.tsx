/**
 * Linked charts in the design editor (A15): a chart whose `source` points at a sheet range refreshes its
 * cached labels/series when the design opens and on demand. Also: inserting a chart handed over from a sheet
 * ("Insert into design…") and the shared clipboard format, so a linked chart pastes into any design.
 */
import type { ClipboardPayload, Editor } from '@oc/engine';
import { chartChanged, chartPatch, rangeToChartData } from '@oc/engine';
import type { CellValue } from '@oc/engine';
import type { ChartEl, ChartSource, Element } from '@oc/engine';
import { ExternalLink, Link2Off, RefreshCw } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useToast } from '../components/ui';
import { t } from '../i18n';

/** Same marker the canvas editor's copy/paste uses (EditorShell). */
const CLIP_MARK = 'oc-clipboard:';
const PENDING_KEY = 'oc:pending-insert';

export async function fetchSheetRange(src: Pick<ChartSource, 'designId' | 'sheet' | 'range'>): Promise<{ values: CellValue[][]; sheetName: string; title: string }> {
  const q = new URLSearchParams({ sheet: src.sheet, range: src.range });
  const r = await fetch(`/api/designs/${encodeURIComponent(src.designId)}/sheet/values?${q}`, { credentials: 'include' });
  if (!r.ok) throw new Error(r.status === 404 ? t('The linked sheet is not available.') : `HTTP ${r.status}`);
  return (await r.json()) as { values: CellValue[][]; sheetName: string; title: string };
}

/**
 * Refresh linked charts (all, or `only`). Automatic refreshes are not undo steps; a manual refresh is.
 * Returns how many charts changed.
 */
export async function refreshLinkedCharts(editor: Editor, opts: { only?: string[]; manual?: boolean } = {}): Promise<number> {
  const charts = Object.values(editor.doc.elements).filter((e): e is ChartEl => e.type === 'chart' && !e.deleted && Boolean(e.source) && (!opts.only || opts.only.includes(e.id)));
  let n = 0;
  for (const el of charts) {
    const src = el.source!;
    let got;
    try {
      got = await fetchSheetRange(src);
    } catch (e) {
      if (opts.manual) throw e;
      continue;
    }
    const data = rangeToChartData(got.values);
    const cur = editor.doc.elements[el.id];
    if (!cur || cur.type !== 'chart' || cur.deleted) continue;
    if (!chartChanged(cur, data) && !opts.manual) continue;
    const patch = chartPatch(cur, data, { ...src, sheetName: got.sheetName, refreshedAt: Date.now() });
    const op = [{ op: 'update' as const, id: el.id, patch: patch as unknown as Record<string, unknown> }];
    if (opts.manual) editor.apply(op);
    else editor.store.apply(op, 'migration');
    n++;
  }
  return n;
}

/** On open: insert a chart handed over from a sheet, then refresh every linked chart once. */
export function useLinkedCharts(editor: Editor, readOnly: boolean) {
  useEffect(() => {
    if (readOnly) return;
    let raw: string | null = null;
    try {
      raw = sessionStorage.getItem(PENDING_KEY);
      sessionStorage.removeItem(PENDING_KEY);
    } catch {
      // Storage blocked: nothing pending.
    }
    if (raw) {
      try {
        const p = JSON.parse(raw) as { payload: ClipboardPayload; at: number };
        if (Date.now() - p.at < 5 * 60_000) {
          // Centre the chart on the page.
          const page = editor.doc.meta.dims;
          const els = p.payload.elements.map((e) => ({ ...e, xf: { ...e.xf, x: Math.max(0, (page.w - e.xf.w) / 2), y: Math.max(0, (page.h - e.xf.h) / 2) } })) as Element[];
          editor.paste({ ...p.payload, elements: els }, 0);
        }
      } catch {
        // Malformed hand-over: ignore.
      }
    }
    void refreshLinkedCharts(editor).catch(() => {});
  }, [editor, readOnly]);
}

/** Hand a payload to another design: it is inserted when that design opens (see useLinkedCharts). */
export function setPendingInsert(payload: ClipboardPayload) {
  try {
    sessionStorage.setItem(PENDING_KEY, JSON.stringify({ payload, at: Date.now() }));
  } catch {
    // Storage blocked: the copy/paste route still works.
  }
}

/** Put design elements on the system clipboard in the editor's paste format (works inside a click handler). */
export function copyPayload(payload: ClipboardPayload, label: string): boolean {
  const json = JSON.stringify(payload);
  const html = `<meta charset="utf-8"><!--${CLIP_MARK}${btoa(unescape(encodeURIComponent(json)))}--><span>${label}</span>`;
  const on = (e: ClipboardEvent) => {
    e.clipboardData?.setData('text/html', html);
    e.clipboardData?.setData('text/plain', label);
    e.preventDefault();
  };
  document.addEventListener('copy', on, { once: true });
  const ok = document.execCommand('copy');
  document.removeEventListener('copy', on);
  return ok;
}

/** Chart toolbar addition for linked charts: source, refresh, open, unlink. */
export function LinkedChartControls({ el, editor, readOnly }: { el: ChartEl; editor: Editor; readOnly: boolean }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const src = el.source;
  if (!src) return null;
  const label = `${src.sheetName ? `${src.sheetName}!` : ''}${src.range}`;
  return (
    <div className="group">
      <span className="faint" style={{ fontSize: 12, whiteSpace: 'nowrap' }} title={t('Linked to sheet range {range}', { range: label })}>{t('Linked: {range}', { range: label })}</span>
      <button className="btn ghost sm icon" disabled={busy || readOnly} title={t('Refresh data')} aria-label={t('Refresh data')} onClick={async () => {
        setBusy(true);
        try {
          await refreshLinkedCharts(editor, { only: [el.id], manual: true });
          toast(t('Chart data refreshed'));
        } catch (e) {
          toast((e as Error).message);
        } finally {
          setBusy(false);
        }
      }}><RefreshCw size={16} /></button>
      <button className="btn ghost sm icon" title={t('Open sheet')} aria-label={t('Open sheet')} onClick={() => window.open(`/design/${src.designId}`, '_blank', 'noopener')}><ExternalLink size={16} /></button>
      {!readOnly && <button className="btn ghost sm icon" title={t('Unlink from sheet')} aria-label={t('Unlink from sheet')} onClick={() => editor.apply([{ op: 'update', id: el.id, patch: { source: null } }])}><Link2Off size={16} /></button>}
    </div>
  );
}
