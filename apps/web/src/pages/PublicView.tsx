/**
 * Public view link (CO-034): anyone with the link can view, no account needed. Rendered client-side with the
 * same engine; the "Made with" badge appears here only — never in downloaded files (GT-008).
 * Also serves custom link slugs (g131) and the chrome-less embed mode `?embed=1` (g135).
 */
import { BRAND } from '@oc/config';
import { descendants, resolvePage, sortedPages, type DesignDoc, type EmbedEl, type FormEl, type PageId } from '@oc/engine';
import { FormOverlay } from '../components/FormOverlay';
import { EmbedOverlay } from '../components/EmbedOverlay';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Link, useParams, useSearchParams } from 'react-router';
import { ApiError, siteApi } from '../api';
import { sharingApi } from '../api/sharing';
import { ReportButton } from '../components/ReportDialog';
import { loadEngine, paintCanvas, WHITE, type Engine } from '../lib/engine';
import { isRtl, t, tn } from '../i18n';
import './embedView.css';

function PageCanvas({ engine, doc, pageId, width, designId }: { engine: Engine; doc: DesignDoc; pageId: PageId; width: number; designId: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const el = ref.current!;
    const { w, h } = doc.meta.dims;
    const dpr = window.devicePixelRatio || 1;
    const cw = Math.min(width, w);
    const ch = (cw / w) * h;
    el.style.width = `${cw}px`;
    el.style.height = `${ch}px`;
    // oc-render into a 2D canvas per page.
    const draw = () => paintCanvas(engine, el, cw * dpr, ch * dpr, WHITE, resolvePage(doc, pageId), (cw / w) * dpr);
    draw();
    const off = engine.painter.onInvalidate(() => requestAnimationFrame(draw));
    return () => {
      off();
    };
  }, [engine, doc, pageId, width]);
  const { w } = doc.meta.dims;
  const scale = Math.min(width, w) / w;
  const forms = descendants(doc, pageId).filter((e): e is FormEl => e.type === 'form' && e.visible !== false && !e.xf.rot);
  const embeds = descendants(doc, pageId).filter((e): e is EmbedEl => e.type === 'embed' && e.visible !== false && !e.xf.rot);
  return (
    <div style={{ position: 'relative' }}>
      <canvas ref={ref} />
      {forms.map((f) => <FormOverlay key={f.id} doc={doc} el={f} designId={designId} scale={scale} />)}
      {embeds.map((e) => <EmbedOverlay key={e.id} doc={doc} el={e} scale={scale} />)}
    </div>
  );
}

export function PublicView() {
  const { id: key } = useParams();
  // Embed mode (g135): `?embed=1` inside an iframe — no app chrome, one page at a time, fitted to the frame.
  const embed = useSearchParams()[0].get('embed') === '1';
  const [id, setId] = useState<string | null>(null);
  const [data, setData] = useState<{ title: string; owner: string; doc: DesignDoc } | null>(null);
  const [engine, setEngine] = useState<Engine | null>(null);
  const [err, setErr] = useState('');
  const [win, setWin] = useState({ w: window.innerWidth, h: window.innerHeight });
  const [pageIdx, setPageIdx] = useState(0);
  const [needPw, setNeedPw] = useState<{ title: string; wrong: boolean } | null>(null);
  const [pw, setPw] = useState('');
  // /view/<key>: a design id, or a custom link slug (g131) the server resolves after the link's expiry and password checks.
  const open = (password?: string) => Promise.all([sharingApi.publicDesign(key!, password), loadEngine()]).then(([d, e]) => {
    setId(d.id);
    setNeedPw(null);
    setData(d);
    setEngine(e);
    document.title = `${d.title} – ${BRAND.name}`;
    // View insights for the owner (cookie-free; the server ignores DNT/GPC requests).
    if (navigator.doNotTrack !== '1') void siteApi.viewBeacon(d.id).catch(() => {});
  }).catch((x: unknown) => {
    const status = (x as ApiError).status;
    if (status === 401) setNeedPw((p) => ({ title: p?.title ?? '', wrong: Boolean(password) }));
    else if (status === 410) setErr(t('This link has expired. Ask the owner for a new one.'));
    else setErr(t('This design isn’t public, or the link is wrong.'));
  });
  useEffect(() => {
    void open();
    const r = () => setWin({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener('resize', r);
    return () => window.removeEventListener('resize', r);
  }, [key]);
  // oEmbed discovery for tools that read it from the page (the provider answers only while the link is public).
  useEffect(() => {
    if (!data || embed) return;
    const l = document.createElement('link');
    l.rel = 'alternate';
    l.type = 'application/json+oembed';
    l.href = `/api/public/oembed?url=${encodeURIComponent(location.origin + location.pathname)}&format=json`;
    l.title = data.title;
    document.head.appendChild(l);
    return () => l.remove();
  }, [data, embed]);
  const pages = data ? sortedPages(data.doc).filter((p) => !p.hidden) : [];
  useEffect(() => {
    if (!embed || pages.length < 2) return;
    const k = (e: KeyboardEvent) => {
      // Keys typed into a form element on the page (caret moves, radio groups) aren't page turns.
      if (e.defaultPrevented || (e.target as Element | null)?.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      // Right-to-left: the next page is to the left.
      const [next, prev] = isRtl() ? ['ArrowLeft', 'ArrowRight'] : ['ArrowRight', 'ArrowLeft'];
      if (e.key === next || e.key === 'PageDown') setPageIdx((i) => Math.min(pages.length - 1, i + 1));
      if (e.key === prev || e.key === 'PageUp') setPageIdx((i) => Math.max(0, i - 1));
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [embed, pages.length]);
  if (err) return <div className="auth"><div className="auth-card"><h2 style={{ margin: 0 }}>{t('Not available')}</h2><p className="muted">{err}</p></div></div>;
  if (needPw) return (
    <div className="auth"><form className="auth-card" onSubmit={(e) => { e.preventDefault(); void open(pw); }}>
      <h2 style={{ margin: 0 }}>{t('This design is password protected')}</h2>
      <input className="input" type="password" autoFocus={!embed} value={pw} onChange={(e) => setPw(e.target.value)} placeholder={t('Password')} aria-label={t('Password')} aria-invalid={needPw.wrong} />
      {needPw.wrong && <div style={{ color: 'var(--danger)', fontSize: 13 }}>{t('That password isn’t right.')}</div>}
      <button className="btn primary tall" disabled={!pw}>{t('View design')}</button>
    </form></div>
  );
  if (!data || !engine || !id) return <div className="empty">{t('Loading…')}</div>;
  if (embed) {
    const page = pages[Math.min(pageIdx, pages.length - 1)];
    const { w, h } = data.doc.meta.dims;
    // Fit the page inside the frame (the bar below takes 36px; 8px breathing room on each side).
    const fit = Math.max(40, Math.min(win.w - 16, ((win.h - 36 - 16) * w) / h));
    return (
      <div className="embed-view">
        <div className="embed-stage">
          {page && <PageCanvas key={page.id} engine={engine} doc={data.doc} pageId={page.id} width={fit} designId={id} />}
          {pages.length > 1 && (
            <>
              <button className="embed-nav prev" onClick={() => setPageIdx((i) => Math.max(0, i - 1))} disabled={pageIdx === 0} aria-label={t('Previous page')}><ChevronLeft size={18} /></button>
              <button className="embed-nav next" onClick={() => setPageIdx((i) => Math.min(pages.length - 1, i + 1))} disabled={pageIdx >= pages.length - 1} aria-label={t('Next page')}><ChevronRight size={18} /></button>
            </>
          )}
        </div>
        <div className="embed-bar">
          <a className="title" href={`/view/${encodeURIComponent(key!)}`} target="_blank" rel="noopener noreferrer" title={t('Open full view')}>{data.title}</a>
          {pages.length > 1 && <span className="count" aria-live="polite">{t('{n} / {total}', { n: Math.min(pageIdx, pages.length - 1) + 1, total: pages.length })}</span>}
          <a className="made" href="/" target="_blank" rel="noopener noreferrer"><img src="/favicon.svg" alt="" width={16} height={16} /> {t('Made with {brand}', { brand: BRAND.name })}</a>
        </div>
      </div>
    );
  }
  const width = Math.min(1000, win.w - 32);
  return (
    <div className="viewer">
      <div className="viewer-bar">
        <img src="/favicon.svg" alt="" width={24} height={24} />
        <b>{data.title}</b>
        <span className="faint">{t('by {owner}', { owner: data.owner })} · {tn(pages.length, '{n} page', '{n} pages')}</span>
        <span className="grow" />
        <ReportButton targetType="public_link" targetId={id} />
      </div>
      {pages.map((p) => <PageCanvas key={p.id} engine={engine} doc={data.doc} pageId={p.id} width={width} designId={id} />)}
      <Link to="/" className="made-with"><img src="/favicon.svg" alt="" width={18} height={18} /> {t('Made with {brand}', { brand: BRAND.name })}</Link>
    </div>
  );
}
