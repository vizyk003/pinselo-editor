/**
 * A small spreadsheet for chart data (labels down, series across) — shared by the design editor's chart
 * toolbar and doc chart blocks. Edits stay local until a cell loses focus (or rows/series change), then
 * `onCommit` receives the new data.
 */
import { CHART_COLORS, hex, toHex, type ChartKind, type ChartSeries, type ColorValue } from '@oc/engine';
import { Plus, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { t } from '../i18n';

export interface ChartDataValue { kind: ChartKind; labels: string[]; series: ChartSeries[] }

export const SINGLE_SERIES_KINDS: ReadonlySet<ChartKind> = new Set(['pie', 'donut', 'funnel', 'progress-ring', 'progress-bar', 'gauge']);

export function ChartDataEditor({ value, sliceColorAt, onCommit, onSliceColor }: {
  value: ChartDataValue;
  /** Current colour of slice `i` (pie/donut/funnel). */
  sliceColorAt?: (i: number) => ColorValue;
  onCommit: (data: { labels: string[]; series: ChartSeries[] }) => void;
  /** Slice colours changed (pie/donut/funnel); omitted → no slice colour pickers. */
  onSliceColor?: (colors: ColorValue[]) => void;
}) {
  const [labels, setLabels] = useState(value.labels);
  const [series, setSeries] = useState(value.series.map((s) => ({ ...s, values: [...s.values] })));
  const commit = (l = labels, s = series) => onCommit({ labels: l, series: s });
  const pie = SINGLE_SERIES_KINDS.has(value.kind);
  const shown = pie ? series.slice(0, 1) : series;
  const slices = (value.kind === 'pie' || value.kind === 'donut' || value.kind === 'funnel') && onSliceColor && sliceColorAt;
  /** Paste from a spreadsheet or CSV: first row = series names, first column = labels. */
  const onPaste = (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text/plain');
    if (!/[\t,;]/.test(text) || !text.includes('\n')) return;
    const delim = text.includes('\t') ? '\t' : text.split('\n')[0]!.includes(';') ? ';' : ',';
    const rows = text.trim().split(/\r?\n/).map((r) => r.split(delim).map((c) => c.trim().replace(/^"|"$/g, '')));
    if (rows.length < 2 || rows[0]!.length < 2) return;
    e.preventDefault();
    const num = (c: string) => Number(c.replace(/[^0-9.\-eE]/g, '')) || 0;
    const nl = rows.slice(1).map((r) => r[0] ?? '');
    const ns = rows[0]!.slice(1).map((name, si) => ({ name: name || t('Series {n}', { n: si + 1 }), color: series[si]?.color ?? hex(CHART_COLORS[si % CHART_COLORS.length]!), values: rows.slice(1).map((r) => num(r[si + 1] ?? '')) }));
    setLabels(nl);
    setSeries(ns);
    commit(nl, ns);
  };
  return (
    <div className="col" style={{ width: 'min(560px, 90vw)' }} onPaste={onPaste}>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Tip: paste cells from a spreadsheet or CSV here to replace the data (first row = series names, first column = labels).')}</p>
      <div style={{ overflow: 'auto', maxHeight: 360 }}>
        <table style={{ borderCollapse: 'collapse', width: '100%' }}>
          <thead>
            <tr>
              <th style={{ textAlign: 'left', fontSize: 12, padding: 4 }}>{t('Label')}</th>
              {shown.map((s, si) => (
                <th key={si} style={{ padding: 4 }}>
                  <div className="row" style={{ gap: 4 }}>
                    <input type="color" value={toHex(s.color)} onChange={(e) => { const n = series.map((x, i) => (i === si ? { ...x, color: hex(e.target.value) } : x)); setSeries(n); commit(labels, n); }} style={{ width: 24, height: 24, border: 0, padding: 0 }} aria-label={t('Series colour')} />
                    <input className="input" style={{ height: 28 }} value={s.name} onChange={(e) => setSeries(series.map((x, i) => (i === si ? { ...x, name: e.target.value } : x)))} onBlur={() => commit()} aria-label={t('Series name')} />
                    {!pie && series.length > 1 && <button className="btn ghost sm icon" onClick={() => { const n = series.filter((_, i) => i !== si); setSeries(n); commit(labels, n); }} aria-label={t('Remove series')}><Trash2 size={13} /></button>}
                  </div>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {labels.map((l, li) => (
              <tr key={li}>
                <td style={{ padding: 3 }}>
                  <div className="row" style={{ gap: 4 }}>
                    {slices && (
                      <input type="color" value={toHex(sliceColorAt(li))} style={{ width: 24, height: 24, border: 0, padding: 0 }} aria-label={t('Slice colour')}
                        onChange={(e) => onSliceColor(labels.map((_, i) => (i === li ? hex(e.target.value) : sliceColorAt(i))))} />
                    )}
                    <input className="input" style={{ height: 28 }} value={l} onChange={(e) => setLabels(labels.map((x, i) => (i === li ? e.target.value : x)))} onBlur={() => commit()} aria-label={t('Label {n}', { n: li + 1 })} />
                  </div>
                </td>
                {shown.map((s, si) => (
                  <td key={si} style={{ padding: 3 }}>
                    <input className="input" style={{ height: 28 }} type="number" value={s.values[li] ?? 0}
                      onChange={(e) => setSeries(series.map((x, i) => (i === si ? { ...x, values: labels.map((_, k) => (k === li ? +e.target.value : x.values[k] ?? 0)) } : x)))}
                      onBlur={() => commit()} aria-label={`${s.name} ${l}`} />
                  </td>
                ))}
                <td><button className="btn ghost sm icon" onClick={() => { const nl = labels.filter((_, i) => i !== li); const ns = series.map((x) => ({ ...x, values: x.values.filter((_, i) => i !== li) })); setLabels(nl); setSeries(ns); commit(nl, ns); }} aria-label={t('Remove row')} disabled={labels.length < 2}><Trash2 size={13} /></button></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="row">
        <button className="btn sm" onClick={() => { const nl = [...labels, t('Item {n}', { n: labels.length + 1 })]; const ns = series.map((x) => ({ ...x, values: [...x.values, 0] })); setLabels(nl); setSeries(ns); commit(nl, ns); }}><Plus size={14} /> {t('Row')}</button>
        {!pie && <button className="btn sm" onClick={() => { const ns = [...series, { name: t('Series {n}', { n: series.length + 1 }), values: labels.map(() => 0), color: hex(CHART_COLORS[series.length % CHART_COLORS.length]!) }]; setSeries(ns); commit(labels, ns); }}><Plus size={14} /> {t('Series')}</button>}
        <span className="faint" style={{ fontSize: 12 }}>{pie ? t('Pie charts use the first series.') : t('Edit cells, then click away to apply.')}</span>
      </div>
    </div>
  );
}
