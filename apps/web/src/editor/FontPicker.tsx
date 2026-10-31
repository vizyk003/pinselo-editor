/**
 * Font picker (plan 11 g69): search and category filters over every family, with sections for the fonts this design
 * already uses, the ones picked recently on this device and the brand kit's fonts, above the full list.
 */
import type { FontFamily } from '@oc/engine';
import { ChevronDown } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { PopoverButton, SearchField } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { documentFonts } from '../lib/contentRules';
import { previewFamily } from '../lib/cssFonts';
import { activeControls, governingKits, useBrandHub } from './brandStore';
import { useDoc, useEd } from './context';

const FONT_CATS = ['all', 'uploaded', 'sans', 'serif', 'display', 'script', 'handwriting', 'mono'] as const;
const FONT_CAT_LABELS: Record<string, string> = {
  all: tk('All'), uploaded: tk('Brand'), sans: tk('Sans'), serif: tk('Serif'), display: tk('Display'), script: tk('Script'), handwriting: tk('Handwriting'), mono: tk('Mono'),
};
const fontCatLabel = (c: string) => (FONT_CAT_LABELS[c] ? t(FONT_CAT_LABELS[c]) : c[0]!.toUpperCase() + c.slice(1));

// ------------------------------------------------------------------ recently used (per device)

const RECENT_KEY = 'oc:recent-fonts';
const RECENT_MAX = 8;
function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]') as unknown;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}
let recent = loadRecent();
const listeners = new Set<() => void>();
/** Remember a family the user picked or added text in (most recent first). */
export function recordRecentFont(family: string) {
  recent = [family, ...recent.filter((f) => f !== family)].slice(0, RECENT_MAX);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  } catch {
    /* private mode: keep in memory */
  }
  for (const l of listeners) l();
}
const useRecentFonts = () => useSyncExternalStore((cb) => {
  listeners.add(cb);
  return () => listeners.delete(cb);
}, () => recent);

export function FontPicker({ value, onPick }: { value: string; onPick: (f: string) => void }) {
  const { engine } = useEd();
  const doc = useDoc();
  const hub = useBrandHub();
  const recentFonts = useRecentFonts();
  const [q, setQ] = useState('');
  const [cat, setCat] = useState<(typeof FONT_CATS)[number]>('all');
  // Brand control "brand fonts only" (A8): just the governing team's kit fonts.
  const onlyFonts = activeControls(hub).fonts;
  const every = engine.fonts.families();
  const all = onlyFonts ? every.filter((f) => onlyFonts.includes(f.family)) : every;
  const cats = FONT_CATS.filter((c) => c === 'all' || all.some((f) => f.category === c));
  // Uploaded brand fonts first, then bundled families, then Google Fonts by popularity. ~2,000 families,
  // so rows render a page at a time as the list scrolls.
  const [limit, setLimit] = useState(80);
  const needle = q.trim().toLowerCase();
  const fams = useMemo(() => all
    .filter((f) => (cat === 'all' || f.category === cat) && (!needle || f.family.toLowerCase().includes(needle)))
    .sort((a, b) => (b.category === 'uploaded' ? 1 : 0) - (a.category === 'uploaded' ? 1 : 0) || (needle ? Number(!a.family.toLowerCase().startsWith(needle)) - Number(!b.family.toLowerCase().startsWith(needle)) : 0)), [all, cat, needle]);
  const byName = useMemo(() => new Map(all.map((f) => [f.family, f])), [all]);
  const pick = (family: string, close: () => void) => {
    recordRecentFont(family);
    onPick(family);
    close();
  };
  // Sections show only on the unfiltered "All" list.
  const sectioned = !needle && cat === 'all';
  const kitFonts = [...new Set(governingKits(hub).flatMap((k) => Object.values(k.fonts).filter(Boolean) as string[]))];
  const section = (id: string, label: string, names: string[], close: () => void) => {
    const list = names.map((n) => byName.get(n)).filter((f): f is FontFamily => Boolean(f));
    if (!list.length) return null;
    return (
      <div role="group" aria-labelledby={`fp-${id}`} key={id}>
        <div className="font-sec" id={`fp-${id}`}>{label}</div>
        {list.map((f) => <FontRow key={`${id}:${f.family}`} f={f} selected={f.family === value} onPick={() => pick(f.family, close)} />)}
      </div>
    );
  };
  return (
    <PopoverButton className="btn sm font-btn" title={t('Font')} label={<><span style={{ fontFamily: previewFamily(every.find((f) => f.family === value) ?? every[0]!) }}>{value}</span><ChevronDown size={14} className="faint" aria-hidden /></>}
      content={(close) => (
        <div className="pop-sm" style={{ display: 'grid', gap: 8 }}>
          <SearchField value={q} onChange={(v) => { setQ(v); setLimit(80); }} placeholder={t('Search fonts')} autoFocus />
          <div className="font-cats">{cats.map((c) => <button key={c} className={cat === c ? 'on' : ''} aria-pressed={cat === c} onClick={() => { setCat(c); setLimit(80); }}>{fontCatLabel(c)}</button>)}</div>
          <div className="menu scroll" style={{ maxHeight: 360 }} onScroll={(e) => {
            const el = e.currentTarget;
            if (el.scrollTop + el.clientHeight > el.scrollHeight - 200 && limit < fams.length) setLimit((n) => n + 80);
          }}>
            {sectioned && section('doc', t('Document fonts'), documentFonts(doc), close)}
            {sectioned && section('recent', t('Recently used'), recentFonts, close)}
            {sectioned && !onlyFonts && section('brand', t('Brand fonts'), kitFonts, close)}
            {sectioned && <div className="font-sec">{t('All fonts')}</div>}
            {fams.slice(0, limit).map((f) => <FontRow key={f.family} f={f} selected={f.family === value} onPick={() => pick(f.family, close)} />)}
            {!fams.length && <div className="faint" style={{ padding: 8 }}>{t('No fonts match')}</div>}
          </div>
          {onlyFonts && <span className="faint" style={{ fontSize: 11 }}>{t('Your team limits fonts to the brand kit.')}</span>}
          {!onlyFonts && <span className="faint" style={{ fontSize: 11 }}>{tn(all.length, '{n} family, including all of Google Fonts · upload brand fonts in Brand', '{n} families, including all of Google Fonts · upload brand fonts in Brand')}</span>}
        </div>
      )} />
  );
}

/** A font in the picker; its preview face loads only once the row scrolls into view. */
function FontRow({ f, selected, onPick }: { f: FontFamily; selected: boolean; onPick: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) {
        setVisible(true);
        io.disconnect();
      }
    });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <button ref={ref} onClick={onPick} aria-current={selected || undefined} style={{ fontFamily: visible ? previewFamily(f) : undefined, fontSize: 16, fontWeight: selected ? 700 : 400, minHeight: 34 }}>
      {f.family}
      <span className="kbd" style={{ fontFamily: 'Inter' }}>{fontCatLabel(f.category).toLocaleLowerCase()}</span>
    </button>
  );
}
