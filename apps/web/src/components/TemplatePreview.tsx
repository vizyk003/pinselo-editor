/**
 * Template preview before use (g25): every page of the template, its size and type, category and tags, who made it,
 * "Use this template", a copyable link and similar templates. Opened from TemplateCard (dashboard, Templates page,
 * public home) and from a shared /templates?t=<id> link.
 */
import { BRAND } from '@oc/config';
import { formatDims, resolvePage, sortedPages, type DesignDoc, type PageId } from '@oc/engine';
import { ChevronLeft, ChevronRight, Link2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { api, templateThumbUrl, type TemplateSummary } from '../api';
import { useAuth } from '../App';
import { useAuthGate } from '../auth/AuthGate';
import { isRtl, t, tk, tn } from '../i18n';
import { track } from '../lib/analytics';
import { WHITE, paintCanvas, type Engine } from '../lib/engine';
import { fetchTemplatePreview, presetOf, recordRecentTemplate, templateLink, type TemplatePreviewData } from '../lib/templates';
import { Modal, useToast } from './ui';

const SURFACE_LABEL: Record<string, string> = {
  graphic: tk('Graphic'), presentation: tk('Presentation'), print: tk('Print'), video: tk('Video'),
  doc: tk('Doc'), website: tk('Website'),
};

/** Create a design from a template (signed out: sign in first, then /?template= creates it). */
export function useTemplateUse() {
  const nav = useNavigate();
  const { requireAuth } = useAuthGate();
  const me = useAuth().user?.id ?? null;
  return useCallback(async (tpl: TemplateSummary): Promise<boolean> => {
    if (!requireAuth(`/?template=${encodeURIComponent(tpl.id)}`)) return false;
    track('template_use', { template: tpl.id });
    const { id } = await api.createDesign({ templateId: tpl.id });
    recordRecentTemplate(me, tpl);
    nav(`/design/${id}`);
    return true;
  }, [nav, requireAuth, me]);
}

/** One template page drawn client-side with the same engine as the editor (oc-render into a 2D canvas). */
function PageCanvas({ doc, pageId, maxW, maxH, label }: { doc: DesignDoc; pageId: PageId; maxW: number; maxH: number; label: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [engine, setEngine] = useState<Engine | null>(null);
  useEffect(() => {
    let live = true;
    // Loaded on demand (fonts manifest and first face) when a preview opens.
    void import('../lib/engine').then((m) => m.loadEngine()).then((e) => live && setEngine(e)).catch(() => {});
    return () => { live = false; };
  }, []);
  useEffect(() => {
    const el = ref.current;
    if (!el || !engine) return;
    const { w, h } = doc.meta.dims;
    const scale = Math.min(maxW / w, maxH / h);
    const dpr = window.devicePixelRatio || 1;
    el.style.width = `${Math.round(w * scale)}px`;
    el.style.height = `${Math.round(h * scale)}px`;
    el.width = Math.max(1, Math.round(w * scale * dpr));
    el.height = Math.max(1, Math.round(h * scale * dpr));
    const draw = () => paintCanvas(engine, el, el.width, el.height, WHITE, resolvePage(doc, pageId), scale * dpr);
    draw();
    let raf = 0;
    const off = engine.painter.onInvalidate(() => { cancelAnimationFrame(raf); raf = requestAnimationFrame(draw); });
    return () => { cancelAnimationFrame(raf); off(); };
  }, [engine, doc, pageId, maxW, maxH]);
  if (!engine) return <div className="spinner" />;
  return <canvas ref={ref} className="tpl-preview-canvas" role="img" aria-label={label} />;
}

export function TemplatePreviewModal({ id, initial, onClose, onOpen }: {
  id: string;
  /** The card's summary, shown while the details load. */
  initial?: TemplateSummary;
  onClose: () => void;
  /** Switch the dialog to another template (Similar templates). */
  onOpen: (tpl: TemplateSummary) => void;
}) {
  const toast = useToast();
  const use = useTemplateUse();
  const nav = useNavigate();
  const [data, setData] = useState<TemplatePreviewData | null>(null);
  const [err, setErr] = useState(false);
  const [page, setPage] = useState(0);
  const [busy, setBusy] = useState(false);
  const primaryBtn = useRef<HTMLButtonElement>(null);
  const dots = useRef<HTMLDivElement>(null);
  // Switching to a similar template unmounts the focused card: put focus on "Use this template" instead.
  const firstId = useRef(id);
  useEffect(() => {
    if (id !== firstId.current) primaryBtn.current?.focus();
  }, [id]);
  // Arrow keys on a page dot move the selection; keep focus on the selected dot.
  useEffect(() => {
    if (dots.current?.contains(document.activeElement)) dots.current.querySelectorAll<HTMLElement>('button')[page]?.focus();
  }, [page]);
  useEffect(() => {
    let live = true;
    setData(null);
    setErr(false);
    setPage(0);
    fetchTemplatePreview(id).then((d) => live && setData(d)).catch(() => live && setErr(true));
    track('template_preview', { template: id });
    return () => { live = false; };
  }, [id]);

  const tpl = data?.template ?? initial;
  const pages = data?.doc ? sortedPages(data.doc).filter((p) => !p.hidden) : [];
  const pageCount = data?.template.pages ?? 1;
  const preset = tpl ? presetOf(tpl.preset) : undefined;
  const unit = (data?.template.unit ?? preset?.unit ?? 'px') as Parameters<typeof formatDims>[2];
  const surface = data?.template.surface ?? preset?.surface;
  const go = (d: number) => setPage((p) => Math.max(0, Math.min(pages.length - 1, p + d)));

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(templateLink(id, location.origin));
      toast(t('Link copied'));
    } catch {
      toast(t('Could not copy the link'));
    }
  };
  const start = async () => {
    if (!tpl || busy) return;
    setBusy(true);
    try {
      if (!(await use(tpl))) onClose();
    } catch {
      toast(t('Could not create a design from this template'));
    } finally {
      setBusy(false);
    }
  };
  const browse = (params: Record<string, string>) => {
    onClose();
    nav(`/templates?${new URLSearchParams(params)}`);
  };

  return (
    <Modal title={tpl?.title ?? t('Template')} onClose={onClose} size="xl">
      {err && <div className="empty">{t('This template is not available.')}</div>}
      {!err && (
        <div className="tpl-preview">
          <div className="tpl-preview-stage" onKeyDown={(e) => {
            // Pages read in the text direction, so the arrows flip in right-to-left languages.
            const back = isRtl() ? 'ArrowRight' : 'ArrowLeft';
            if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') go(e.key === back ? -1 : 1);
          }}>
            <div className="tpl-preview-frame">
              {page === 0 || !data?.doc || !pages[page]
                ? <img src={templateThumbUrl(id)} alt={tpl?.title ?? ''} />
                : <PageCanvas key={pages[page]!.id} doc={data.doc} pageId={pages[page]!.id} maxW={560} maxH={420} label={t('Page {n} of {total}', { n: page + 1, total: pages.length })} />}
              {pages.length > 1 && (
                <>
                  <button className="btn icon tpl-preview-nav prev" onClick={() => go(-1)} disabled={page === 0} aria-label={t('Previous page')}><ChevronLeft size={18} /></button>
                  <button className="btn icon tpl-preview-nav next" onClick={() => go(1)} disabled={page >= pages.length - 1} aria-label={t('Next page')}><ChevronRight size={18} /></button>
                </>
              )}
            </div>
            {pages.length > 1 && (
              <div className="tpl-preview-dots" role="group" aria-label={t('Pages')} ref={dots}>
                {pages.map((p, i) => (
                  <button key={p.id} aria-current={i === page ? 'true' : undefined} className={i === page ? 'on' : ''} onClick={() => setPage(i)} aria-label={t('Page {n}', { n: i + 1 })} />
                ))}
              </div>
            )}
            {pageCount > 1 && <div className="faint center" aria-live="polite">{pages.length > 1 ? t('Page {n} of {total}', { n: page + 1, total: pages.length }) : tn(pageCount, '{n} page', '{n} pages')}</div>}
          </div>

          <div className="tpl-preview-info">
            {tpl && (
              <>
                <div className="row" style={{ flexWrap: 'wrap' }}>
                  {tpl.team && <span className="tpl-badge team">{t('TEAM')}</span>}
                  {surface && <span className="tpl-badge">{t(SURFACE_LABEL[surface] ?? surface)}</span>}
                </div>
                <div className="muted">
                  {preset ? `${t(preset.label)} · ` : ''}{formatDims(tpl.width, tpl.height, unit)}
                </div>
                {data?.template.description && <p style={{ margin: 0 }}>{data.template.description}</p>}
                <div className="faint">
                  {data?.template.teamName
                    ? t('By {name}', { name: data.template.teamName })
                    : t('By {name}', { name: data?.template.creator ?? BRAND.name })}
                  {data && data.template.uses > 0 && <> · {tn(data.template.uses, 'Used {n} time', 'Used {n} times')}</>}
                </div>
                <div className="chips">
                  <button className="chip" onClick={() => browse({ category: tpl.category })}>{t(tpl.category)}</button>
                  {(tpl.tags ?? []).slice(0, 8).map((tag) => (
                    <button key={tag} className="chip tpl-tag" onClick={() => browse({ tag })}>#{tag}</button>
                  ))}
                </div>
                <div className="row" style={{ marginTop: 'auto', flexWrap: 'wrap' }}>
                  <button ref={primaryBtn} className="btn primary" style={{ flex: 1 }} onClick={start} disabled={busy} autoFocus>
                    {busy ? t('Creating…') : t('Use this template')}
                  </button>
                  <button className="btn" onClick={copy} title={t('Copy link')} aria-label={t('Copy link')}><Link2 size={16} /></button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {data && data.similar.length > 0 && (
        <section>
          <h3 className="tpl-preview-h">{t('Similar templates')}</h3>
          <div className="tpl-row">
            {data.similar.map((s) => (
              <button key={s.id} className="tpl-card" style={{ background: 'none', border: 0, padding: 0, textAlign: 'start' }} onClick={() => onOpen(s)} title={s.title}>
                <div className="img" style={{ position: 'relative' }}>
                  <img src={templateThumbUrl(s.id)} alt="" loading="lazy" />
                </div>
                <div className="t">{s.title}</div>
              </button>
            ))}
          </div>
        </section>
      )}
      {!data && !err && <div className="spinner" />}
    </Modal>
  );
}
