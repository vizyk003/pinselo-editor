/**
 * Website insights (WS-010) and public view link insights: first-party, cookie-free, aggregated per day.
 * Charts are plain SVG.
 */
import { useEffect, useMemo, useState } from 'react';
import { siteApi, type InsightTop, type SiteInsights } from '../api';
import { fmtDate, fmtNumber, t, tk, tn } from '../i18n';

const DAY = 86_400_000;
const iso = (d: Date) => d.toISOString().slice(0, 10);
const daysAgo = (n: number) => iso(new Date(Date.now() - n * DAY));
const fmtDay = (d: string, o: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' }) => fmtDate(`${d}T12:00:00Z`, o);

const RANGES = [
  { days: 7, label: tk('Last 7 days') },
  { days: 30, label: tk('Last 30 days') },
  { days: 90, label: tk('Last 90 days') },
  { days: 365, label: tk('Last 12 months') },
];
const DEVICES: Record<string, string> = { mobile: tk('Mobile'), tablet: tk('Tablet'), desktop: tk('Desktop') };

function countryName(code: string): string {
  if (!code) return t('Unknown');
  try {
    return new Intl.DisplayNames([document.documentElement.lang || 'en'], { type: 'region' }).of(code) ?? code;
  } catch {
    return code;
  }
}

/** Daily views (bars) and unique visitors (line). */
export function TrendChart({ series, height = 140 }: { series: SiteInsights['series']; height?: number }) {
  const W = 520;
  const H = height;
  const pad = { l: 34, r: 8, t: 8, b: 20 };
  const max = Math.max(1, ...series.map((s) => Math.max(s.views, s.visitors)));
  const n = Math.max(1, series.length);
  const bw = (W - pad.l - pad.r) / n;
  const y = (v: number) => pad.t + (H - pad.t - pad.b) * (1 - v / max);
  const line = series.map((s, i) => `${i ? 'L' : 'M'}${(pad.l + bw * (i + 0.5)).toFixed(1)},${y(s.visitors).toFixed(1)}`).join('');
  const total = series.reduce((a, s) => a + s.views, 0);
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label={t('{n} views from {from} to {to}', { n: total, from: series[0] ? fmtDay(series[0].day) : '', to: series.at(-1) ? fmtDay(series.at(-1)!.day) : '' })} style={{ display: 'block' }}>
      {[0, 0.5, 1].map((f) => (
        <g key={f}>
          <line x1={pad.l} x2={W - pad.r} y1={y(max * f)} y2={y(max * f)} stroke="var(--border)" strokeWidth={1} />
          <text x={pad.l - 6} y={y(max * f) + 4} textAnchor="end" fontSize={10} fill="var(--text-2)">{fmtNumber(Math.round(max * f))}</text>
        </g>
      ))}
      {series.map((s, i) => (
        <rect key={s.day} x={pad.l + bw * i + bw * 0.15} y={y(s.views)} width={Math.max(1, bw * 0.7)} height={Math.max(0, H - pad.b - y(s.views))} rx={Math.min(3, bw * 0.2)} fill="var(--accent)" opacity={0.35}>
          <title>{`${fmtDay(s.day)}: ${tn(s.views, '{n} view', '{n} views')}, ${tn(s.visitors, '{n} visitor', '{n} visitors')}`}</title>
        </rect>
      ))}
      {series.length > 1 && <path d={line} fill="none" stroke="var(--accent)" strokeWidth={2} strokeLinejoin="round" />}
      {series[0] && <text x={pad.l} y={H - 4} fontSize={10} fill="var(--text-2)">{fmtDay(series[0].day)}</text>}
      {series.length > 1 && <text x={W - pad.r} y={H - 4} fontSize={10} textAnchor="end" fill="var(--text-2)">{fmtDay(series.at(-1)!.day)}</text>}
    </svg>
  );
}

function TopList({ title, items, label = (k) => k || t('Direct / unknown') }: { title: string; items: InsightTop[]; label?: (key: string) => string }) {
  const max = Math.max(1, ...items.map((i) => i.n));
  return (
    <div style={{ minWidth: 0 }}>
      <h3 style={{ margin: '0 0 6px', fontSize: 13 }}>{title}</h3>
      {!items.length && <div className="faint" style={{ fontSize: 12 }}>{t('No data yet')}</div>}
      <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'grid', gap: 4 }}>
        {items.map((i) => (
          <li key={i.key} style={{ position: 'relative', fontSize: 12, padding: '3px 6px', display: 'flex', gap: 8 }}>
            <span aria-hidden="true" style={{ position: 'absolute', inset: 0, width: `${(i.n / max) * 100}%`, background: 'var(--accent)', opacity: 0.12, borderRadius: 4 }} />
            <span className="grow" style={{ position: 'relative', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={label(i.key)}>{label(i.key)}</span>
            <b style={{ position: 'relative' }}>{fmtNumber(i.n)}</b>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="box" style={{ padding: '10px 12px', gap: 2 }}>
      <span className="faint" style={{ fontSize: 12 }}>{label}</span>
      <b style={{ fontSize: 20 }}>{fmtNumber(value)}</b>
    </div>
  );
}

export function SiteInsightsPanel({ designId, analytics, onAnalytics }: { designId: string; analytics: boolean; onAnalytics: (on: boolean) => void }) {
  const [days, setDays] = useState<number | 'custom'>(30);
  const [custom, setCustom] = useState({ from: daysAgo(29), to: daysAgo(0) });
  const [data, setData] = useState<SiteInsights | null>(null);
  const [err, setErr] = useState(false);
  const range = useMemo(() => (days === 'custom' ? custom : { from: daysAgo(days - 1), to: daysAgo(0) }), [days, custom]);
  useEffect(() => {
    setErr(false);
    void siteApi.insights(designId, range.from, range.to).then(setData).catch(() => setErr(true));
  }, [designId, range]);
  return (
    <div style={{ display: 'grid', gap: 14 }}>
      <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>
        <select className="input" style={{ width: 'auto' }} aria-label={t('Date range')} value={String(days)} onChange={(e) => setDays(e.target.value === 'custom' ? 'custom' : +e.target.value)}>
          {RANGES.map((r) => <option key={r.days} value={r.days}>{t(r.label)}</option>)}
          <option value="custom">{t('Custom range')}</option>
        </select>
        {days === 'custom' && (
          <>
            <input className="input" type="date" style={{ width: 'auto' }} aria-label={t('From')} value={custom.from} max={custom.to} onChange={(e) => e.target.value && setCustom({ ...custom, from: e.target.value })} />
            <input className="input" type="date" style={{ width: 'auto' }} aria-label={t('To')} value={custom.to} min={custom.from} max={daysAgo(0)} onChange={(e) => e.target.value && setCustom({ ...custom, to: e.target.value })} />
          </>
        )}
        <span className="grow" />
        <label className="check" style={{ fontSize: 13 }}><input type="checkbox" checked={analytics} onChange={(e) => onAnalytics(e.target.checked)} /> {t('Collect insights')}</label>
      </div>
      {!analytics && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Insights are off: visits to this site are not counted.')}</p>}
      {err && <div className="form-error">{t('Couldn’t load insights.')}</div>}
      {!data && !err && <div className="spinner" />}
      {data && (
        <>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(110px, 1fr))', gap: 8 }}>
            <Stat label={t('Views')} value={data.totals.views} />
            <Stat label={t('Unique visitors')} value={data.totals.visitors} />
            <Stat label={t('Link clicks')} value={data.totals.clicks} />
            <Stat label={t('Form submissions')} value={data.totals.forms} />
          </div>
          <TrendChart series={data.series} />
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))', gap: 16 }}>
            <TopList title={t('Referrers')} items={data.referrers} />
            <TopList title={t('Countries')} items={data.countries} label={countryName} />
            <TopList title={t('Devices')} items={data.devices} label={(k) => (DEVICES[k] ? t(DEVICES[k]) : k)} />
            <TopList title={t('Sections viewed')} items={data.sections} label={(k) => k || t('Unknown')} />
            <TopList title={t('Outbound link clicks')} items={data.clicks} label={(k) => k || t('Unknown')} />
            <TopList title={t('Form submissions')} items={data.forms} label={(k) => k || t('Form')} />
          </div>
        </>
      )}
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Cookie-free and first-party: visitors are counted with a daily-changing anonymous hash, Do Not Track is honoured, and only daily totals are kept. Unique visitors over a range add up daily visitors.')}</p>
    </div>
  );
}

/** Share dialog: views of the public view link. */
export function ViewInsightsLine({ designId }: { designId: string }) {
  const [d, setD] = useState<{ views: number; visitors: number; series: SiteInsights['series'] } | null>(null);
  useEffect(() => {
    void siteApi.viewInsights(designId, 30).then(setD).catch(() => {});
  }, [designId]);
  if (!d) return null;
  return (
    <div style={{ display: 'grid', gap: 6 }}>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>
        {tn(d.views, '{n} view', '{n} views')} · {tn(d.visitors, '{n} unique viewer', '{n} unique viewers')} · {t('last 30 days')}
      </p>
      {d.views > 0 && <TrendChart series={d.series} height={70} />}
    </div>
  );
}
