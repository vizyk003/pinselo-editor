import { resolvePage, sortedPages, type PageId } from '@oc/engine';
import { WHITE, paintCanvas } from '../lib/engine';
import { LayoutGrid, Clock, ChevronDown, ChevronUp, Copy, EyeOff, GalleryHorizontal, Lock, Maximize, Minus, Plus, Trash2, StickyNote } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { t } from '../i18n';
import { useScrollFade } from '../components/ui';
import { useDoc, useEd, useEditorState } from './context';
import { usePageMenu } from './PageMenu';

const THUMB_H = 72;

/** Page thumbnails painted by oc-render into 2D canvases, throttled. */
export function PageThumb({ pageId, w, h, height = THUMB_H }: { pageId: PageId; w: number; h: number; height?: number }) {
  const { editor, engine } = useEd();
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let lastDoc = editor.doc;
    const render = () => {
      const c = ref.current;
      if (!c) return;
      const scale = height / h;
      const tw = Math.max(1, Math.round(w * scale));
      const th = height;
      paintCanvas(engine, c, tw * 2, th * 2, WHITE, resolvePage(editor.doc, pageId), scale * 2);
      c.style.width = `${tw}px`;
    };
    const schedule = () => {
      clearTimeout(timer);
      timer = setTimeout(render, 500);
    };
    render();
    const off = editor.subscribe(() => {
      // Only re-render when the document (not just UI state) changed.
      if (editor.doc === lastDoc) return;
      lastDoc = editor.doc;
      schedule();
    });
    const offP = engine.painter.onInvalidate(schedule);
    return () => {
      clearTimeout(timer);
      off();
      offP();
    };
  }, [editor, engine, pageId, w, h, height]);
  return <canvas ref={ref} style={{ height }} />;
}

export function PageStrip() {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const pages = sortedPages(doc);
  const [drag, setDrag] = useState<PageId | null>(null);
  const { w, h } = doc.meta.dims;
  const row = useScrollFade();
  const menu = usePageMenu();
  return (
    <div className="pagestrip" role="toolbar" aria-orientation="horizontal" aria-label={t('Pages')} ref={row}>
      {menu.node}
      {pages.map((p, i) => (
        // A11Y-02: one tab stop (roving tabindex on the active page); ←/→/Home/End go to a page, Enter/Space too,
        // Alt+←/→ moves the page.
        <div key={p.id} className={`p ${p.id === state.activePageId ? 'active' : ''}`} title={p.title ? t('Page {n} – {title}', { n: i + 1, title: p.title }) : t('Page {n}', { n: i + 1 })}
          role="button" tabIndex={p.id === state.activePageId ? 0 : -1} aria-current={p.id === state.activePageId ? 'page' : undefined}
          aria-label={p.title ? t('Page {n} – {title}', { n: i + 1, title: p.title }) : t('Page {n}', { n: i + 1 })}
          onKeyDown={(e) => {
            const strip = e.currentTarget.parentElement;
            const go = (j: number) => {
              const q = pages[Math.max(0, Math.min(pages.length - 1, j))];
              if (!q) return;
              e.preventDefault();
              if (e.altKey && !readOnly) editor.movePage(p.id, Math.max(0, Math.min(pages.length - 1, j)));
              else editor.setActivePage(q.id, true);
              requestAnimationFrame(() => (strip?.querySelector('.p[tabindex="0"]') as HTMLElement | null)?.focus());
            };
            if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') go(i - 1);
            else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') go(i + 1);
            else if (e.key === 'Home') go(0);
            else if (e.key === 'End') go(pages.length - 1);
            else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); editor.setActivePage(p.id, true); }
            else if (!readOnly && ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu')) {
              // g97: the page menu from the keyboard, under the thumbnail.
              e.preventDefault();
              const r = e.currentTarget.getBoundingClientRect();
              menu.open(p.id, r.left, r.top);
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            if (readOnly) return;
            editor.setActivePage(p.id, true);
            menu.open(p.id, e.clientX, e.clientY);
          }}
          aria-haspopup={readOnly ? undefined : 'menu'}
          onClick={() => editor.setActivePage(p.id, true)}
          draggable={!readOnly}
          onDragStart={() => setDrag(p.id)}
          onDragOver={(e) => e.preventDefault()}
          onDrop={() => { if (drag && drag !== p.id) editor.movePage(drag, i); setDrag(null); }}>
          <PageThumb pageId={p.id} w={w} h={h} />
          <span className="n">{i + 1}</span>
          {p.hidden && <EyeOff size={12} style={{ position: 'absolute', right: 4, top: 4, color: '#fff' }} />}
          {p.locked && <Lock size={12} aria-label={t('Locked')} style={{ position: 'absolute', left: 4, top: 4, color: '#fff' }} />}
        </div>
      ))}
      {!readOnly && <button className="add" title={t('Add page (⌘Enter)')} aria-label={t('Add page')} onClick={() => editor.addPage()}><Plus size={18} /></button>}
    </div>
  );
}

export function StatusBar({ notesOpen, setNotesOpen, stripOpen, setStripOpen, timelineOpen, setTimelineOpen }: { notesOpen: boolean; setNotesOpen: (b: boolean) => void; stripOpen: boolean; setStripOpen: (b: boolean) => void; timelineOpen: boolean; setTimelineOpen: (b: boolean) => void }) {
  const { editor, readOnly, openDialog } = useEd();
  const state = useEditorState();
  const doc = useDoc();
  const pages = sortedPages(doc);
  const idx = pages.findIndex((p) => p.id === state.activePageId);
  const zoom = Math.round(state.viewport.zoom * 100);
  return (
    <div className="statusbar">
      <button className={`btn ghost sm ${notesOpen ? 'active' : ''}`} onClick={() => setNotesOpen(!notesOpen)} title={t('Speaker notes')}><StickyNote size={15} /> {t('Notes')}</button>
      <button className={`btn ghost sm ${timelineOpen ? '' : ''}`} onClick={() => { if (timelineOpen) setTimelineOpen(false); setStripOpen(timelineOpen ? true : !stripOpen); }}><GalleryHorizontal size={15} /> {t('Pages')} {stripOpen && !timelineOpen ? <ChevronDown size={14} className="faint" /> : <ChevronUp size={14} className="faint" />}</button>
      <button className={`btn ghost sm ${timelineOpen ? 'active' : ''}`} onClick={() => setTimelineOpen(!timelineOpen)} title={t('Timeline: play, trim, split, soundtrack, captions')}><Clock size={15} /> {t('Timeline')}</button>
      <span className="muted nowrap" style={{ fontSize: 12 }}>{t('Page {n} / {total}', { n: idx + 1, total: pages.length })}</span>
      {!readOnly && (
        <>
          <button className="btn ghost sm icon" title={t('Duplicate page')} onClick={() => editor.duplicatePage()}><Copy size={15} /></button>
          <button className="btn ghost sm icon" title={t('Delete page')} onClick={() => editor.deletePage()} disabled={pages.length <= 1}><Trash2 size={15} /></button>
        </>
      )}
      <div className="grow" />
      <button className="btn ghost sm icon" onClick={() => editor.zoomBy(0.8)} title={t('Zoom out')}><Minus size={15} /></button>
      <input type="range" min={10} max={400} value={Math.min(400, zoom)} onChange={(e) => editor.zoomTo(+e.target.value / 100)} aria-label={t('Zoom')} />
      <button className="btn ghost sm icon" onClick={() => editor.zoomBy(1.25)} title={t('Zoom in')}><Plus size={15} /></button>
      <span style={{ width: 44, textAlign: 'right', fontSize: 12 }} className="muted">{zoom}%</span>
      <button className="btn ghost sm" onClick={() => editor.zoomToFit()} title={t('Fit page (⌘0)')}><Maximize size={15} /> {t('Fit')}</button>
      <button className="btn ghost sm icon" onClick={() => openDialog('grid')} title={t('Grid view: all pages')}><LayoutGrid size={15} /></button>
    </div>
  );
}

export function NotesBar() {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const page = doc.pages[state.activePageId];
  const [v, setV] = useState(page?.notes ?? '');
  useEffect(() => setV(page?.notes ?? ''), [page?.id, page?.notes]);
  if (!page) return null;
  return (
    <div style={{ padding: '8px 12px', background: 'var(--surface)', borderTop: '1px solid var(--border)' }}>
      <textarea className="input" rows={3} placeholder={t('Add speaker notes for this page…')} value={v} readOnly={readOnly}
        onChange={(e) => setV(e.target.value)} onBlur={() => v !== (page.notes ?? '') && editor.updatePage(page.id, { notes: v })} />
    </div>
  );
}
