/** Grid view (ED-080): every page at once — reorder by dragging, select several, duplicate or delete. */
import { sortedPages, type PageId } from '@oc/engine';
import { Copy, EyeOff, Trash2, X } from 'lucide-react';
import { useState } from 'react';
import { useDoc, useEd, useEditorState } from './context';
import { PageThumb } from './PageStrip';
import { t, tn } from '../i18n';

export function PageGridView({ onClose }: { onClose: () => void }) {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const pages = sortedPages(doc);
  const [sel, setSel] = useState<PageId[]>([state.activePageId]);
  const [drag, setDrag] = useState<PageId | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const { w, h } = doc.meta.dims;
  const thumbH = Math.round(Math.min(180, (220 * h) / w));

  const click = (id: PageId, e: React.MouseEvent) => {
    if (e.shiftKey || e.metaKey || e.ctrlKey) setSel((s) => (s.includes(id) ? s.filter((x) => x !== id) : [...s, id]));
    else setSel([id]);
  };
  const open = (id: PageId) => {
    editor.setActivePage(id, true);
    onClose();
  };
  return (
    <div className="page-grid" role="dialog" aria-label={t('All pages')}>
      <header className="row">
        <h2 className="grow" style={{ margin: 0, fontSize: 18 }}>{tn(pages.length, '{n} page', '{n} pages')}</h2>
        {!readOnly && (
          <>
            <span className="faint" style={{ fontSize: 12 }}>{t('{count} selected · drag to reorder · double-click to open', { count: sel.length })}</span>
            <button className="btn sm" onClick={() => { for (const id of [...sel].reverse()) editor.duplicatePage(id); }}><Copy size={14} /> {t('Duplicate')}</button>
            <button className="btn sm" onClick={() => sel.forEach((id) => editor.updatePage(id, { hidden: !doc.pages[id]?.hidden }))}><EyeOff size={14} /> {t('Hide/show')}</button>
            <button className="btn sm danger" disabled={sel.length >= pages.length} onClick={() => { sel.forEach((id) => editor.deletePage(id)); setSel([]); }}><Trash2 size={14} /> {t('Delete')}</button>
          </>
        )}
        <button className="btn sm icon" onClick={onClose} aria-label={t('Close grid view')}><X size={16} /></button>
      </header>
      <div className="page-grid-body">
        {pages.map((p, i) => (
          <div key={p.id}
            className={`pg-card${sel.includes(p.id) ? ' sel' : ''}${over === i ? ' over' : ''}${p.hidden ? ' hidden' : ''}`}
            draggable={!readOnly}
            onDragStart={() => setDrag(p.id)}
            onDragOver={(e) => { e.preventDefault(); setOver(i); }}
            onDragLeave={() => setOver((o) => (o === i ? null : o))}
            onDrop={(e) => {
              e.preventDefault();
              if (drag && drag !== p.id) editor.movePage(drag, i);
              setDrag(null);
              setOver(null);
            }}
            onClick={(e) => click(p.id, e)}
            onDoubleClick={() => open(p.id)}
            tabIndex={0}
            onKeyDown={(e) => e.key === 'Enter' && open(p.id)}
            aria-label={p.title ? t('Page {n}: {title}', { n: i + 1, title: p.title }) : t('Page {n}', { n: i + 1 })}
          >
            <div className="pg-thumb" style={{ height: thumbH }}><PageThumb pageId={p.id} w={w} h={h} height={thumbH} /></div>
            <div className="pg-label">{i + 1}{p.title ? ` · ${p.title}` : ''}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
