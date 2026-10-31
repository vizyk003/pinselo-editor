/**
 * Library (EL-020/030..034): stock photos, video, audio, GIFs and stickers from the configured providers,
 * plus emoji stickers and the graphics & icon library (A4). Click to add, or drag onto the page (or into a frame / grid cell).
 */
import { Music, Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { ApiError, contentApi, type EmojiEntry, type StockItem, type StockKind } from '../api';
import { SearchField, useScrollFade, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import { useEd } from './context';
import { GraphicsGrid } from './GraphicsPanel';
import { markDragMedia, type DropMedia } from './dropTarget';
import { insertAssetAt } from './insert';
import { fmtTime } from './media';

type Tab = StockKind | 'emoji' | 'graphics';
const TABS: Array<{ id: Tab; label: string; search: string }> = [
  { id: 'image', label: tk('Photos'), search: tk('Search photos') }, { id: 'graphics', label: tk('Graphics'), search: tk('Search icons and graphics') }, { id: 'video', label: tk('Videos'), search: tk('Search videos') },
  { id: 'audio', label: tk('Audio'), search: tk('Search audio') }, { id: 'gif', label: tk('GIFs'), search: tk('Search gifs') },
  { id: 'sticker', label: tk('Stickers'), search: tk('Search stickers') }, { id: 'emoji', label: tk('Emoji'), search: tk('Search emoji') },
];
const EMOJI_GROUPS: Array<[string, string, string]> = [
  ['smileys-emotion', '😀', tk('smileys & emotion')], ['people-body', '👋', tk('people & body')], ['animals-nature', '🐶', tk('animals & nature')],
  ['food-drink', '🍕', tk('food & drink')], ['travel-places', '✈️', tk('travel & places')], ['activities', '⚽', tk('activities')],
  ['objects', '💡', tk('objects')], ['symbols', '❤️', tk('symbols')], ['flags', '🏁', tk('flags')],
];

export const STOCK_MIME = 'application/x-oc-stock';
/** Stock photos fill shapes and frames; videos, GIFs and stickers (stored as looping videos) fill frames. */
export const stockMedia = (kind: StockKind): DropMedia | null => (kind === 'image' ? 'photo' : kind === 'audio' ? null : 'video');

let providersCache: Promise<Array<{ id: string; label: string; kinds: StockKind[] }>> | null = null;
const providers = () => (providersCache ??= contentApi.providers().then((r) => r.providers).catch(() => []));

export function LibraryPanel() {
  const [tab, setTab] = useState<Tab>('image');
  const tabsRow = useScrollFade();
  return (
    <>
      <div className="lib-tabs" role="tablist" ref={tabsRow}>
        {TABS.map((x) => (
          <button key={x.id} role="tab" aria-selected={tab === x.id} className={tab === x.id ? 'on' : ''} onClick={() => setTab(x.id)}>{t(x.label)}</button>
        ))}
      </div>
      {tab === 'emoji' ? <EmojiGrid /> : tab === 'graphics' ? <GraphicsGrid /> : <StockGrid kind={tab} key={tab} />}
    </>
  );
}

function StockGrid({ kind }: { kind: StockKind }) {
  const { editor } = useEd();
  const toast = useToast();
  const [list, setList] = useState<Array<{ id: string; label: string; kinds: StockKind[] }> | null>(null);
  const [provider, setProvider] = useState('');
  const [q, setQ] = useState('');
  const [query, setQuery] = useState('');
  const [items, setItems] = useState<StockItem[]>([]);
  const [page, setPage] = useState(1);
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [err, setErr] = useState('');

  useEffect(() => {
    void providers().then((ps) => {
      const ok = ps.filter((p) => p.kinds.includes(kind));
      setList(ok);
      setProvider(ok[0]?.id ?? '');
    });
  }, [kind]);

  useEffect(() => {
    if (!provider) return;
    let cancelled = false;
    setBusy(true);
    setErr('');
    contentApi.search(provider, kind, query, page)
      .then((r) => {
        if (cancelled) return;
        setItems((cur) => (page === 1 ? r.items : [...cur, ...r.items]));
        setMore(r.hasMore);
      })
      .catch((e) => !cancelled && setErr(e instanceof ApiError ? e.message ?? t('Search failed.') : t('Search failed.')))
      .finally(() => !cancelled && setBusy(false));
    return () => {
      cancelled = true;
    };
  }, [provider, kind, query, page]);

  const add = async (it: StockItem) => {
    setAdding(it.id);
    try {
      const { asset } = await contentApi.import(it.provider, it.kind, it.id);
      insertAssetAt(editor, asset, undefined);
      if (it.license && !/CC0|Public Domain|Pexels|Unsplash|Pixabay/i.test(it.license)) toast(t('Added — credit: {author} ({license}). See Download → Credits.', { author: it.author ?? t('the author'), license: it.license }));
    } catch (e) {
      toast(e instanceof ApiError ? e.message ?? t('Couldn’t add it.') : t('Couldn’t add it.'));
    } finally {
      setAdding(null);
    }
  };

  if (list && !list.length) {
    const hint = kind === 'video' ? t('Pexels or Pixabay (OC_PEXELS_KEY / OC_PIXABAY_KEY)') : kind === 'gif' || kind === 'sticker' ? 'GIPHY (OC_GIPHY_KEY)' : t('a stock provider');
    const kindName = ({ image: t('image'), video: t('video'), audio: t('audio'), gif: 'GIF', sticker: t('sticker') } as Record<string, string>)[kind] ?? kind;
    return <p className="faint" style={{ fontSize: 13 }}>{t('No {kind} library is connected on this server. An administrator can add an API key for {hint}. You can still upload your own files.', { kind: kindName, hint })}</p>;
  }

  return (
    <>
      <form className="row" style={{ position: 'relative' }} onSubmit={(e) => { e.preventDefault(); setPage(1); setQuery(q.trim()); }}>
        <SearchField value={q} onChange={setQ} placeholder={t(TABS.find((x) => x.id === kind)?.search ?? 'Search')} />
      </form>
      {list && list.length > 1 && (
        <div className="row" style={{ flexWrap: 'wrap', gap: 4 }}>
          {list.map((p) => <button key={p.id} className={`btn sm ${provider === p.id ? 'active' : ''}`} onClick={() => { setPage(1); setProvider(p.id); }}>{p.label}</button>)}
        </div>
      )}
      {err && <div className="form-error">{err}</div>}
      {kind === 'audio' ? (
        <div className="list">
          {items.map((it) => (
            <button key={it.id} className="item audio-row" onClick={() => add(it)} disabled={adding === it.id} title={`${it.title} — ${it.author ?? ''} · ${it.license}`}
              draggable onDragStart={(e) => e.dataTransfer.setData(STOCK_MIME, JSON.stringify({ provider: it.provider, kind: it.kind, id: it.id }))}>
              <Music size={16} />
              <div className="main"><div className="t">{it.title}</div><div className="sub">{it.author ?? ''} · {it.license}</div></div>
              <span className="faint" style={{ fontSize: 12 }}>{it.durationMs ? fmtTime(it.durationMs) : ''}</span>
            </button>
          ))}
        </div>
      ) : (
        <div className="masonry">
          {items.map((it) => (
            <button key={`${it.provider}:${it.id}`} className={`m-tile${adding === it.id ? ' busy' : ''}`} onClick={() => add(it)} title={`${it.title}${it.author ? ` — ${it.author}` : ''} · ${it.license}`}
              draggable onDragStart={(e) => {
                e.dataTransfer.setData(STOCK_MIME, JSON.stringify({ provider: it.provider, kind: it.kind, id: it.id }));
                markDragMedia(e.dataTransfer, stockMedia(it.kind));
              }}>
              <img src={it.thumb} alt={it.title} loading="lazy" referrerPolicy="no-referrer" style={{ aspectRatio: it.w && it.h ? `${it.w} / ${it.h}` : undefined }} />
              {it.kind === 'video' && it.durationMs ? <span className="dur">{fmtTime(it.durationMs)}</span> : null}
            </button>
          ))}
        </div>
      )}
      {busy && <div className="spinner" />}
      {!busy && more && <button className="btn sm" onClick={() => setPage((p) => p + 1)}>{t('Load more')}</button>}
      {!busy && !items.length && !err && <div className="empty">{t('No results.')}</div>}
      {list?.find((p) => p.id === provider) && <p className="faint" style={{ fontSize: 11, margin: 0 }}>{t('From {provider}. Credits for licensed media are listed under Download → Credits.', { provider: list.find((p) => p.id === provider)!.label })}</p>}
    </>
  );
}

let emojiCache: Promise<EmojiEntry[]> | null = null;

function EmojiGrid() {
  const { editor } = useEd();
  const toast = useToast();
  const [all, setAll] = useState<EmojiEntry[] | null>(null);
  const [q, setQ] = useState('');
  const [group, setGroup] = useState('smileys-emotion');
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    void (emojiCache ??= contentApi.emoji()).then(setAll).catch(() => setAll([]));
  }, []);
  const shown = useMemo(() => {
    if (!all) return [];
    const term = q.trim().toLowerCase();
    if (term) return all.filter((e) => e.l.includes(term) || e.t.some((x) => x.startsWith(term))).slice(0, 300);
    return all.filter((e) => e.g === group);
  }, [all, q, group]);
  const add = async (e: EmojiEntry, at?: [number, number]) => {
    try {
      const { asset } = await contentApi.emojiAsset(e.h);
      const { w, h } = editor.doc.meta.dims;
      const s = Math.round(Math.min(w, h) * 0.22);
      insertAssetAt(editor, { ...asset, w: s, h: s }, at);
    } catch {
      toast(t('Couldn’t add that emoji.'));
    }
  };
  return (
    <>
      <SearchField value={q} onChange={setQ} placeholder={t('Search emoji')} onClear={() => setQ('')} />
      {!q && (
        <div className="row" style={{ gap: 2, flexWrap: 'wrap' }}>
          {EMOJI_GROUPS.map(([g, icon, label]) => (
            <button key={g} className={`btn sm icon ${group === g ? 'active' : ''}`} title={t(label)} onClick={() => { setGroup(g); ref.current?.scrollTo(0, 0); }}>{icon}</button>
          ))}
        </div>
      )}
      <div ref={ref} className="emoji-grid">
        {shown.map((e) => (
          <button key={e.h} title={e.l} onClick={() => add(e)} draggable onDragStart={(ev) => {
            ev.dataTransfer.setData('application/x-oc-emoji', e.h);
            markDragMedia(ev.dataTransfer, 'graphic');
          }}>
            <img src={`/api/emoji/${e.h}.svg`} alt={e.l} loading="lazy" width={36} height={36} />
          </button>
        ))}
      </div>
      {all && !shown.length && <div className="empty">{t('No emoji found.')}</div>}
      <p className="faint" style={{ fontSize: 11, margin: 0 }}>Noto Emoji © Google, Apache License 2.0.</p>
    </>
  );
}
