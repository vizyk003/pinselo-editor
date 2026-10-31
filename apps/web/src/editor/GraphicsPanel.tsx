/**
 * Graphics & icons (A4): searchable open-licence icon sets (Lucide, Tabler, Phosphor) and decorative graphics. Click to add,
 * or drag onto the page. Inserted graphics are SVG assets: their colours can be changed from the toolbar (SvgColors.tsx).
 */
import type { AssetRef, Vec2 } from '@oc/engine';
import { Search } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { contentApi, type GraphicEntry } from '../api';
import { SearchField, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import type { Editor } from '@oc/engine';
import { useEd } from './context';
import { markDragMedia, setAssetDrag } from './dropTarget';
import { insertAssetAt, type DropOptions } from './insert';

export const GRAPHIC_MIME = 'application/x-oc-graphic';

/** `name`: a set's proper name (not translated). */
const FILTERS: Array<{ id: string; label: string; name?: true; styles?: Array<[string, string]> }> = [
  { id: 'all', label: tk('All') },
  { id: 'graphics', label: tk('Graphics') },
  { id: 'icons', label: tk('Icons') },
  { id: 'lucide', label: 'Lucide', name: true },
  { id: 'tabler', label: 'Tabler', name: true, styles: [['outline', tk('Outline')], ['filled', tk('Filled')]] },
  { id: 'phosphor', label: 'Phosphor', name: true, styles: [['regular', tk('Regular')], ['fill', tk('Fill')], ['duotone', tk('Duotone')]] },
];

/** Graphic id → design asset (created once per session; the server also reuses it per user). */
const assets = new Map<string, Promise<AssetRef>>();
const assetFor = (id: string) => {
  let p = assets.get(id);
  if (!p) {
    p = contentApi.graphicAsset(id).then((r) => r.asset);
    p.catch(() => assets.delete(id));
    assets.set(id, p);
  }
  return p;
};
const ready = new Map<string, AssetRef>();

/** Inserted size: icons like emoji stickers, graphics a bit larger; aspect kept. */
function sized(editor: Editor, a: AssetRef, kind: GraphicEntry['kind'] | undefined): AssetRef {
  const { w, h } = editor.doc.meta.dims;
  const s = Math.round(Math.min(w, h) * (kind === 'graphic' ? 0.4 : 0.22));
  const k = s / Math.max(a.w, a.h, 1);
  return { ...a, w: Math.max(8, Math.round(a.w * k)), h: Math.max(8, Math.round(a.h * k)) };
}

/** Insert a library graphic (also used by the canvas drop handler). */
export async function insertGraphic(editor: Editor, id: string, at?: Vec2, pageId?: Parameters<typeof insertAssetAt>[3], kind?: GraphicEntry['kind'], opts?: DropOptions) {
  const a = await assetFor(id);
  insertAssetAt(editor, sized(editor, a, kind ?? (id.startsWith('oc-') || id.startsWith('gravity-') ? 'graphic' : 'icon')), at, pageId, opts);
}

export function GraphicsGrid() {
  const { editor } = useEd();
  const toast = useToast();
  const [q, setQ] = useState('');
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  const [style, setStyle] = useState('');
  const [items, setItems] = useState<GraphicEntry[]>([]);
  const [page, setPage] = useState(1);
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const sentinel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = setTimeout(() => { setPage(1); setQuery(q.trim()); }, 250);
    return () => clearTimeout(timer);
  }, [q]);

  useEffect(() => {
    let cancelled = false;
    setBusy(true);
    setErr(false);
    contentApi.graphics(query, filter, page, style || undefined)
      .then((r) => {
        if (cancelled) return;
        setItems((cur) => (page === 1 ? r.items : [...cur, ...r.items]));
        setMore(r.hasMore);
      })
      .catch(() => !cancelled && setErr(true))
      .finally(() => !cancelled && setBusy(false));
    return () => {
      cancelled = true;
    };
  }, [query, filter, style, page]);

  // Lazy loading: fetch the next page when the end of the grid scrolls into view.
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !more || busy) return;
    const io = new IntersectionObserver((es) => es.some((e) => e.isIntersecting) && setPage((p) => p + 1), { rootMargin: '300px' });
    io.observe(el);
    return () => io.disconnect();
  }, [more, busy, items.length]);

  const add = async (g: GraphicEntry) => {
    setAdding(g.id);
    try {
      await insertGraphic(editor, g.id, undefined, undefined, g.kind);
    } catch {
      toast(t('Couldn’t add that graphic.'));
    } finally {
      setAdding(null);
    }
  };
  const current = FILTERS.find((f) => f.id === filter);

  return (
    <>
      <SearchField value={q} onChange={setQ} placeholder={t('Search icons and graphics')} onClear={() => setQ('')} />
      <div className="font-cats" role="radiogroup" aria-label={t('Graphic sets')}>
        {FILTERS.map((f) => (
          <button key={f.id} role="radio" aria-checked={filter === f.id} className={filter === f.id ? 'on' : ''} onClick={() => { setFilter(f.id); setStyle(''); setPage(1); }}>{f.name ? f.label : t(f.label)}</button>
        ))}
      </div>
      {current?.styles && (
        <div className="font-cats" role="radiogroup" aria-label={t('Icon style')}>
          {[['', tk('All')] as [string, string], ...current.styles].map(([id, label]) => (
            <button key={id || 'all'} role="radio" aria-checked={style === id} className={style === id ? 'on' : ''} onClick={() => { setStyle(id); setPage(1); }}>{t(label)}</button>
          ))}
        </div>
      )}
      <div className="tiles" style={{ gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
        {items.map((g) => (
          <button
            key={g.id} className={`tile${adding === g.id ? ' busy' : ''}`} title={`${g.name.replace(/-/g, ' ')}${g.style ? ` · ${g.style}` : ''} · ${g.license}`}
            style={{ background: '#fff', padding: g.kind === 'icon' ? 12 : 6 }}
            onClick={() => add(g)}
            onPointerDown={() => void assetFor(g.id).then((a) => ready.set(g.id, a)).catch(() => {})}
            draggable
            onDragStart={(e) => {
              e.dataTransfer.setData(GRAPHIC_MIME, JSON.stringify({ id: g.id, kind: g.kind }));
              markDragMedia(e.dataTransfer, 'graphic');
              // Already created (pointerdown starts it): use the regular asset drop path.
              const a = ready.get(g.id);
              if (a) setAssetDrag(e.dataTransfer, sized(editor, a, g.kind));
            }}
          >
            <img src={`/api/graphics/${g.id}.svg`} alt={g.name.replace(/-/g, ' ')} loading="lazy" draggable={false} style={{ objectFit: 'contain', borderRadius: 0 }} />
          </button>
        ))}
      </div>
      <div ref={sentinel} />
      {busy && <div className="spinner" />}
      {err && <div className="form-error">{t('Search failed.')}</div>}
      {!busy && !err && !items.length && <div className="empty">{t('No graphics found.')}</div>}
      <p className="faint" style={{ fontSize: 11, margin: 0 }}>{t('Icons: Lucide (ISC), Tabler and Phosphor (MIT). Illustrations: Gravity UI (MIT). Pinselo graphics are public domain (CC0).')}</p>
    </>
  );
}
