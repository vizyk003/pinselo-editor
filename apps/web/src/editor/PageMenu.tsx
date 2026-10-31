/**
 * Page thumbnail context menu (g97): right-click (or Shift+F10 / the context-menu key) on a page in the page strip.
 * Rename (Page.title), add, duplicate, hide, lock, move, delete, copy a link to the page, download just that page,
 * apply its background to every page, and resize.
 */
import { sortedPages, type DocOp, type PageId } from '@oc/engine';
import { ArrowLeft, ArrowRight, Copy, Download, Eye, EyeOff, Link2, Lock, Paintbrush, Pencil, Plus, Scaling, Trash2, Unlock } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { MenuItem, Modal, Popover, useToast } from '../components/ui';
import { t } from '../i18n';
import { useEd } from './context';
import { requestOnePageDownload } from './downloadIntent';
import { togglePageLock } from './ElementMenu';

/** A link that opens the design on page `n` (1-based); the editor reads `?page=`. */
export const pageLink = (designId: string, n: number) => `${location.origin}/design/${designId}?page=${n}`;

export function usePageMenu(): { open: (pageId: PageId, x: number, y: number) => void; node: ReactNode } {
  const [at, setAt] = useState<{ pageId: PageId; x: number; y: number } | null>(null);
  const [anchor, setAnchor] = useState<HTMLDivElement | null>(null);
  const [renaming, setRenaming] = useState<PageId | null>(null);
  const node = (
    <>
      {at && <div ref={setAnchor} style={{ position: 'fixed', left: at.x, top: at.y, width: 1, height: 1 }} />}
      {at && anchor && (
        <Popover anchor={anchor} onClose={() => setAt(null)} placement="point" className="ctx-menu" menu>
          <PageMenu pageId={at.pageId} close={() => setAt(null)} rename={() => { setRenaming(at.pageId); setAt(null); }} />
        </Popover>
      )}
      {renaming && <RenamePage pageId={renaming} onClose={() => setRenaming(null)} />}
    </>
  );
  return { open: (pageId, x, y) => setAt({ pageId, x, y }), node };
}

function PageMenu({ pageId, close, rename }: { pageId: PageId; close: () => void; rename: () => void }) {
  const { editor, design, openDialog } = useEd();
  const toast = useToast();
  const pages = sortedPages(editor.doc);
  const i = pages.findIndex((p) => p.id === pageId);
  const page = pages[i];
  if (!page) return null;
  const run = (fn: () => void) => () => {
    fn();
    close();
  };
  const locked = Boolean(page.locked);
  const copyBackground = () => {
    const ops: DocOp[] = pages.filter((p) => p.id !== pageId).map((p) => ({ op: 'page', action: 'update', pageId: p.id, patch: { background: page.background } }));
    if (ops.length) editor.apply(ops);
    toast(t('Background applied to all pages'));
  };
  return (
    <div className="menu" role="menu" aria-label={t('Page {n}', { n: i + 1 })}>
      <MenuItem icon={<Pencil size={16} />} label={t('Rename page')} onClick={rename} />
      <MenuItem icon={<Plus size={16} />} label={t('Add page')} onClick={run(() => editor.addPage(pageId))} />
      <MenuItem icon={<Copy size={16} />} label={t('Duplicate page')} onClick={run(() => editor.duplicatePage(pageId))} />
      <MenuItem icon={page.hidden ? <Eye size={16} /> : <EyeOff size={16} />} label={page.hidden ? t('Show page') : t('Hide page')}
        onClick={run(() => editor.updatePage(pageId, { hidden: page.hidden ? undefined : true }))} />
      <MenuItem icon={locked ? <Unlock size={16} /> : <Lock size={16} />} label={locked ? t('Unlock page') : t('Lock page')} onClick={run(() => togglePageLock(editor, pageId))} />
      <MenuItem icon={<ArrowLeft size={16} />} label={t('Move page left')} disabled={i === 0} onClick={run(() => editor.movePage(pageId, i - 1))} />
      <MenuItem icon={<ArrowRight size={16} />} label={t('Move page right')} disabled={i >= pages.length - 1} onClick={run(() => editor.movePage(pageId, i + 1))} />
      <div className="hsep" />
      <MenuItem icon={<Link2 size={16} />} label={t('Copy link to page')} onClick={run(() => {
        void navigator.clipboard.writeText(pageLink(design.id, i + 1)).then(() => toast(t('Link copied')), () => toast(t('Could not copy the link')));
      })} />
      <MenuItem icon={<Download size={16} />} label={t('Download this page')} onClick={run(() => {
        editor.setActivePage(pageId, true);
        requestOnePageDownload();
        openDialog('export');
      })} />
      <MenuItem icon={<Paintbrush size={16} />} label={t('Apply background to all pages')} disabled={pages.length < 2} onClick={run(copyBackground)} />
      <MenuItem icon={<Scaling size={16} />} label={t('Resize design')} onClick={run(() => openDialog('resize'))} />
      <div className="hsep" />
      <MenuItem icon={<Trash2 size={16} />} label={t('Delete page')} danger disabled={pages.length <= 1 || locked} onClick={run(() => editor.deletePage(pageId))} />
    </div>
  );
}

function RenamePage({ pageId, onClose }: { pageId: PageId; onClose: () => void }) {
  const { editor } = useEd();
  const [v, setV] = useState(editor.doc.pages[pageId]?.title ?? '');
  return (
    <Modal title={t('Rename page')} onClose={onClose} size="sm">
      <form className="col" onSubmit={(e) => { e.preventDefault(); editor.updatePage(pageId, { title: v.trim().slice(0, 120) || undefined }); onClose(); }}>
        <label className="field">{t('Page title')}
          <input className="input" value={v} maxLength={120} onChange={(e) => setV(e.target.value)} autoFocus />
        </label>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button type="button" className="btn" onClick={onClose}>{t('Cancel')}</button>
          <button className="btn primary">{t('Save')}</button>
        </div>
      </form>
    </Modal>
  );
}
