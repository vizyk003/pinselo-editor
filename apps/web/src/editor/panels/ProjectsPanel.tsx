/**
 * Projects panel (g84): browse my designs and folders without leaving the editor, open one in a new tab, or bring
 * its pages and elements into this design. The other design is read from its last saved snapshot
 * (GET /api/designs/:id/doc, view access): no second live connection, so nothing can be written back to it.
 */
import { bodyPlainText, childrenOf, RenderTarget, resolvePage, sortedPages, worldBounds, type DesignDoc, type Element, type ElementId, type PageId, type Rect } from '@oc/engine';
import { ChevronDown, ChevronRight, ExternalLink, Folder as FolderIcon, LayoutTemplate, Plus, Search, Users } from 'lucide-react';
import { Fragment, useEffect, useMemo, useRef, useState } from 'react';
import { thumbUrl, type DesignSummary, type Folder } from '../../api';
import { editorFilesApi, type ProjectsSearch } from '../../api/editorFiles';
import { SearchField, useToast } from '../../components/ui';
import { t, tn } from '../../i18n';
import { WHITE, type Engine } from '../../lib/engine';
import { useEd } from '../context';
import { childFolders, folderPath, sortFolders } from '../../pages/home/projectsModel';
import { canInsertFrom, importElementsOps, importPageOps, pickableElements } from './projects';

const openInNewTab = (id: string) => window.open(`/design/${id}`, '_blank', 'noopener');

export function ProjectsPanel() {
  const [folders, setFolders] = useState<Folder[] | null>(null);
  const [at, setAt] = useState<string | null>(null);
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const [q, setQ] = useState('');
  const [results, setResults] = useState<ProjectsSearch | null>(null);
  const [source, setSource] = useState<DesignSummary | null>(null);

  useEffect(() => {
    void editorFilesApi.folders().then(setFolders).catch(() => setFolders([]));
  }, []);

  useEffect(() => {
    let live = true;
    setDesigns(null);
    editorFilesApi.designsIn(at).then((r) => live && setDesigns(r)).catch(() => live && setDesigns([]));
    return () => {
      live = false;
    };
  }, [at]);

  const query = q.trim();
  useEffect(() => {
    if (!query) {
      setResults(null);
      return;
    }
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      editorFilesApi.search(query, ctl.signal).then(setResults).catch(() => !ctl.signal.aborted && setResults({ designs: [], folders: [] }));
    }, 250);
    return () => {
      clearTimeout(timer);
      ctl.abort();
    };
  }, [query]);

  const go = (id: string | null) => {
    setSource(null);
    setQ('');
    setAt(id);
  };
  // A design picked from search opens where it's filed, so the breadcrumbs lead back through its folder.
  const pickFound = (d: DesignSummary) => {
    setAt(d.folder_id && folders?.some((f) => f.id === d.folder_id) ? d.folder_id : null);
    setQ('');
    setSource(d);
  };
  const trail = folderPath(folders ?? [], at);
  const sub = folders ? sortFolders(childFolders(folders, at), 'az') : [];

  return (
    <>
      <SearchField value={q} onChange={(v) => { setQ(v); setSource(null); }} placeholder={t('Search designs and folders')} onClear={() => setQ('')} />

      {!query && (
        <nav aria-label={t('Folders')} className="row" style={{ flexWrap: 'wrap', gap: 2, fontSize: 13 }}>
          <button className="btn ghost sm" style={{ paddingInline: 6 }} onClick={() => go(null)} aria-current={!at && !source ? 'page' : undefined}>{t('Projects')}</button>
          {trail.map((f, i) => (
            <Fragment key={f.id}>
              <ChevronRight size={14} className="faint flip-rtl" aria-hidden />
              <button className="btn ghost sm" style={{ paddingInline: 6, maxWidth: 140, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', display: 'block' }}
                onClick={() => go(f.id)} aria-current={i === trail.length - 1 && !source ? 'page' : undefined} title={f.name}>{f.name}</button>
            </Fragment>
          ))}
          {source && (
            <>
              <ChevronRight size={14} className="faint flip-rtl" aria-hidden />
              <span style={{ fontWeight: 600, maxWidth: 160, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', paddingInline: 6 }} aria-current="page" title={source.title}>{source.title}</span>
            </>
          )}
        </nav>
      )}

      {source ? (
        <SourceDesign d={source} />
      ) : query ? (
        !results ? <div className="spinner" /> : (
          <>
            {results.folders.length > 0 && <><h4>{t('Folders')}</h4><FolderList folders={results.folders} onOpen={go} /></>}
            {results.designs.length > 0 && <><h4>{t('Designs')}</h4><DesignTiles designs={results.designs} onPick={pickFound} /></>}
            {!results.folders.length && !results.designs.length && <div className="empty">{t('No designs or folders match.')}</div>}
          </>
        )
      ) : (
        <>
          {folders === null && <div className="spinner" />}
          {sub.length > 0 && <><h4>{t('Folders')}</h4><FolderList folders={sub} onOpen={go} /></>}
          {folders && (
            !designs ? <div className="spinner" /> : designs.length ? (
              <><h4>{t('Designs')}</h4><DesignTiles designs={designs} onPick={setSource} /></>
            ) : (!sub.length && <div className="empty">{at ? t('This folder is empty.') : t('Nothing here yet.')}</div>)
          )}
        </>
      )}
    </>
  );
}

function FolderList({ folders, onOpen }: { folders: ReadonlyArray<Pick<Folder, 'id' | 'name' | 'team' | 'designs'>>; onOpen: (id: string) => void }) {
  return (
    <div className="col" style={{ gap: 2 }}>
      {folders.map((f) => (
        <button key={f.id} className="btn ghost sm" style={{ justifyContent: 'flex-start', width: '100%', height: 36 }} onClick={() => onOpen(f.id)}>
          {f.team ? <Users size={16} aria-hidden /> : <FolderIcon size={16} aria-hidden />}
          <span className="grow" style={{ textAlign: 'start', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {f.name}{f.team && <span className="faint" style={{ fontWeight: 400 }}> · {f.team}</span>}
          </span>
          <span className="faint">{f.designs}</span>
        </button>
      ))}
    </div>
  );
}

function DesignTiles({ designs, onPick }: { designs: DesignSummary[]; onPick: (d: DesignSummary) => void }) {
  const { design } = useEd();
  return (
    <div className="tiles two">
      {designs.map((d) => <DesignTile key={d.id} d={d} current={d.id === design.id} onPick={onPick} />)}
    </div>
  );
}

function DesignTile({ d, current, onPick }: { d: DesignSummary; current: boolean; onPick: (d: DesignSummary) => void }) {
  const [imgOk, setImgOk] = useState(true);
  const insertable = !current && canInsertFrom(d.surface);
  // Docs and sheets can't be inserted from: clicking opens them instead.
  const pick = () => (insertable ? onPick(d) : !current && openInNewTab(d.id));
  return (
    <div style={{ position: 'relative', minWidth: 0 }}>
      <button className="tile" style={{ width: '100%', padding: 4, opacity: current ? 0.6 : 1 }} onClick={pick} disabled={current}
        title={insertable ? d.title : current ? t('This design') : t('Opens in a new tab')} aria-label={insertable ? t('Show pages of {title}', { title: d.title }) : t('Open {title}', { title: d.title })}>
        {imgOk ? <img src={thumbUrl(d.id, d.updated_at)} alt="" loading="lazy" style={{ objectFit: 'contain' }} onError={() => setImgOk(false)} /> : <LayoutTemplate size={24} className="faint" />}
      </button>
      {!current && (
        <button className="btn sm icon" style={{ position: 'absolute', top: 4, right: 4, width: 24, height: 24 }} title={t('Open in new tab')} aria-label={t('Open in new tab')} onClick={() => openInNewTab(d.id)}>
          <ExternalLink size={12} />
        </button>
      )}
      <div style={{ fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: 4 }} title={d.title}>{d.title}</div>
      <div className="faint" style={{ fontSize: 11 }}>{current ? t('This design') : tn(d.page_count, '{n} page', '{n} pages')}</div>
    </div>
  );
}

/** One design opened for inserting: its pages, each with "Add page" and its elements one by one. */
function SourceDesign({ d }: { d: DesignSummary }) {
  const { editor, readOnly } = useEd();
  const toast = useToast();
  const [doc, setDoc] = useState<DesignDoc | null>(null);
  const [failed, setFailed] = useState(false);
  const [expanded, setExpanded] = useState<PageId | null>(null);
  useEffect(() => {
    let live = true;
    setDoc(null);
    setFailed(false);
    editorFilesApi.sourceDoc(d.id).then((x) => live && setDoc(x)).catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [d.id]);

  if (failed) return <div className="empty">{t('Couldn’t load this design.')}</div>;
  if (!doc) return <div className="spinner" />;
  const pages = sortedPages(doc);
  const target = editor.doc.meta.dims;

  const addPage = (pageId: PageId) => {
    const plan = importPageOps(doc, pageId, target, editor.state.activePageId);
    if (plan && editor.apply(plan.ops).ok) {
      editor.setActivePage(plan.created[0] as PageId, true);
      toast(t('Page added'));
    } else toast(t('Couldn’t add that page.'));
  };
  const addElement = (id: ElementId) => {
    const plan = importElementsOps(doc, [id], target, editor.state.activePageId);
    if (plan && editor.apply(plan.ops).ok) {
      editor.select(plan.created as ElementId[]);
      toast(t('Added to the page'));
    } else toast(t('Couldn’t add that element.'));
  };

  return (
    <>
      <div className="row">
        <span className="faint grow" style={{ fontSize: 12 }}>{tn(pages.length, '{n} page', '{n} pages')}</span>
        <button className="btn ghost sm" onClick={() => openInNewTab(d.id)}><ExternalLink size={14} /> {t('Open in new tab')}</button>
      </div>
      {pages.map((p, i) => {
        const label = p.title ? t('Page {n} – {title}', { n: i + 1, title: p.title }) : t('Page {n}', { n: i + 1 });
        const open = expanded === p.id;
        const els = open ? pickableElements(doc, p.id) : [];
        return (
          <div key={p.id} className="col" style={{ gap: 6 }}>
            <DocCanvas doc={doc} pageId={p.id} maxW={248} maxH={160} label={label} />
            <div className="row" style={{ gap: 4 }}>
              <span className="grow" style={{ fontSize: 12, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={label}>{label}</span>
              {!readOnly && <button className="btn sm" onClick={() => addPage(p.id)}><Plus size={14} /> {t('Add page')}</button>}
              <button className="btn ghost sm icon" onClick={() => setExpanded(open ? null : p.id)} aria-expanded={open}
                title={open ? t('Hide elements') : t('Show elements')} aria-label={open ? t('Hide elements') : t('Show elements')}>
                {open ? <ChevronDown size={16} /> : <ChevronRight size={16} className="flip-rtl" />}
              </button>
            </div>
            {open && (els.length ? (
              <div className="tiles" role="list" aria-label={t('Elements')}>
                {els.map((el, j) => {
                  const name = elementName(el) || t('Element {n}', { n: j + 1 });
                  return (
                    // The list item wraps the button, so each tile is still announced (and navigable) as a button.
                    <div key={el.id} role="listitem" style={{ display: 'grid' }}>
                      <button className="tile" style={{ padding: 4 }} disabled={readOnly} onClick={() => addElement(el.id)}
                        title={readOnly ? name : t('Add “{name}” to this page', { name })} aria-label={t('Add “{name}” to this page', { name })}>
                        <DocCanvas doc={doc} pageId={p.id} focus={el.id} maxW={64} maxH={64} label={name} />
                      </button>
                    </div>
                  );
                })}
              </div>
            ) : <div className="faint" style={{ fontSize: 12 }}>{t('This page is empty.')}</div>)}
          </div>
        );
      })}
    </>
  );
}

function elementName(el: Element): string {
  if (el.name) return el.name;
  if (el.type === 'text') return bodyPlainText(el.body).slice(0, 40).trim();
  return '';
}

/**
 * The area to draw: the whole page, or one element's bounds with a little margin — cut to the page when the element
 * bleeds off it (the thumbnail shows what the page shows; inserting still brings the whole element).
 */
function regionOf(doc: DesignDoc, focus: ElementId | undefined): Rect {
  const { w, h } = doc.meta.dims;
  if (!focus || !doc.elements[focus]) return { x: 0, y: 0, w, h };
  const b = worldBounds(doc, focus);
  const pad = Math.max(b.w, b.h) * 0.04;
  let x0 = b.x - pad, y0 = b.y - pad, x1 = b.x + b.w + pad, y1 = b.y + b.h + pad;
  if (x0 < w && y0 < h && x1 > 0 && y1 > 0) {
    x0 = Math.max(0, x0); y0 = Math.max(0, y0); x1 = Math.min(w, x1); y1 = Math.min(h, y1);
  }
  return { x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) };
}

/**
 * A page of a read-only document, or one of its top-level elements on its own, drawn client-side with the editor's
 * engine (CPU surface into a 2D canvas, like the page strip) once it scrolls into view.
 */
function DocCanvas({ doc, pageId, focus, maxW, maxH, label }: { doc: DesignDoc; pageId: PageId; focus?: ElementId; maxW: number; maxH: number; label: string }) {
  const { engine } = useEd();
  const ref = useRef<HTMLCanvasElement>(null);
  const [visible, setVisible] = useState(false);
  const region = useMemo(() => regionOf(doc, focus), [doc, focus]);
  const scale = Math.min(maxW / region.w, maxH / region.h);
  const cssW = Math.max(1, Math.round(region.w * scale));
  const cssH = Math.max(1, Math.round(region.h * scale));
  useEffect(() => {
    const c = ref.current;
    if (!c || typeof IntersectionObserver === 'undefined') return setVisible(true);
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) {
        setVisible(true);
        io.disconnect();
      }
    }, { rootMargin: '200px' });
    io.observe(c);
    return () => io.disconnect();
  }, []);
  useEffect(() => {
    if (!visible) return;
    let raf = 0;
    const draw = () => drawRegion(engine, ref.current, doc, pageId, region, scale, focus);
    draw();
    // Fonts and images arrive later: redraw when the painter says something changed.
    const off = engine.painter.onInvalidate(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(draw);
    });
    return () => {
      cancelAnimationFrame(raf);
      off();
    };
  }, [visible, engine, doc, pageId, region, scale, focus]);
  return (
    <canvas ref={ref} role="img" aria-label={label}
      style={{ width: cssW, height: cssH, maxWidth: '100%', display: 'block', margin: focus ? 'auto' : undefined, background: '#fff', borderRadius: focus ? 3 : 6, boxShadow: focus ? undefined : '0 0 0 1px var(--border)' }} />
  );
}

function drawRegion(engine: Engine, c: HTMLCanvasElement | null, doc: DesignDoc, pageId: PageId, region: Rect, scale: number, focus?: ElementId) {
  if (!c) return;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const w = Math.max(1, Math.round(region.w * scale * dpr));
  const h = Math.max(1, Math.round(region.h * scale * dpr));
  const t = new RenderTarget().begin(0, 0, w, h);
  t.clear(...WHITE);
  t.scale(scale * dpr);
  t.translate(-region.x, -region.y);
  // One element alone: every other top-level element of the page is skipped; the page background stays, so light
  // text from a dark page is still readable.
  const skip = focus ? new Set(childrenOf(doc, pageId).filter((e) => e.id !== focus).map((e) => e.id)) : undefined;
  engine.painter.draw(t, resolvePage(doc, pageId, skip ? { skip } : {}));
  const img = t.end();
  c.width = w;
  c.height = h;
  c.getContext('2d')?.putImageData(img, 0, 0);
}
