/**
 * Canvas chrome in HTML over the scene: the header above each page with its title and page actions, '+ Add page'
 * under the last page (g96), and the floating quick bar above the selection (g94).
 */
import type { PageId, PageSlot, Vec2 } from '@oc/engine';
import { ChevronDown, ChevronUp, Copy, Lock, MessageSquarePlus, MoreHorizontal, Plus, Trash2, Unlock } from 'lucide-react';
import { useEffect, useState, type ReactNode, type SyntheticEvent } from 'react';
import { MenuItem, PopoverButton } from '../components/ui';
import { t } from '../i18n';
import { useEd } from './context';
import { togglePageLock } from './ElementMenu';
import './editorDepth.css';
import { userLock } from './dropTarget';

/** Keep pointer and menu events on this chrome away from the canvas below (no marquee, no deselect). */
const shield = {
  onPointerDown: (e: SyntheticEvent) => e.stopPropagation(),
  onPointerUp: (e: SyntheticEvent) => e.stopPropagation(),
  onDoubleClick: (e: SyntheticEvent) => e.stopPropagation(),
  onContextMenu: (e: SyntheticEvent) => e.stopPropagation(),
  onKeyDown: (e: SyntheticEvent) => e.stopPropagation(),
};

const HEAD_H = 30;

export function PageHeaders({ slots, toScreen, zoom, height }: {
  slots: PageSlot[]; toScreen: (p: Vec2) => [number, number]; zoom: number;
  /** Visible height of the canvas: headers outside it aren't rendered. */
  height: number;
}) {
  const { editor, readOnly } = useEd();
  const last = slots[slots.length - 1];
  return (
    <>
      {slots.map((s) => {
        const [x, y] = toScreen([s.rect.x, s.rect.y]);
        const w = s.rect.w * zoom;
        const h = s.rect.h * zoom;
        if (y + h < -HEAD_H || y - HEAD_H > height) return null;
        return <PageHeader key={s.page.id} pageId={s.page.id} index={s.index} count={slots.length} left={x} top={y - HEAD_H - 4} width={w} readOnly={readOnly} />;
      })}
      {last && !readOnly && (() => {
        const [x, y] = toScreen([last.rect.x, last.rect.y + last.rect.h]);
        const w = last.rect.w * zoom;
        if (y > height) return null;
        return (
          <button className="page-add-below" style={{ left: x + Math.max(0, (w - Math.min(w, 360)) / 2), top: y + 12, width: Math.min(w, 360) }} {...shield}
            onClick={() => editor.addPage(last.page.id)}>
            <Plus size={16} /> {t('Add page')}
          </button>
        );
      })()}
    </>
  );
}

function PageHeader({ pageId, index, count, left, top, width, readOnly }: {
  pageId: PageId; index: number; count: number; left: number; top: number; width: number; readOnly: boolean;
}) {
  const { editor } = useEd();
  const page = editor.doc.pages[pageId];
  const [title, setTitle] = useState(page?.title ?? '');
  useEffect(() => setTitle(page?.title ?? ''), [page?.title]);
  if (!page) return null;
  const locked = Boolean(page.locked);
  const label = t('Page {n}', { n: index + 1 });
  const commit = () => {
    const v = title.trim().slice(0, 120);
    if (v !== (page.title ?? '')) editor.updatePage(pageId, { title: v || undefined });
  };
  const compact = width < 300;
  const buttons: Array<{ key: string; icon: ReactNode; label: string; onClick: () => void; disabled?: boolean; pressed?: boolean }> = [
    { key: 'up', icon: <ChevronUp size={16} />, label: t('Move page up'), onClick: () => editor.movePage(pageId, index - 1), disabled: index === 0 },
    { key: 'down', icon: <ChevronDown size={16} />, label: t('Move page down'), onClick: () => editor.movePage(pageId, index + 1), disabled: index >= count - 1 },
    { key: 'lock', icon: locked ? <Lock size={16} /> : <Unlock size={16} />, label: locked ? t('Unlock page') : t('Lock page'), onClick: () => togglePageLock(editor, pageId), pressed: locked },
    { key: 'dup', icon: <Copy size={16} />, label: t('Duplicate page'), onClick: () => editor.duplicatePage(pageId) },
    { key: 'del', icon: <Trash2 size={16} />, label: t('Delete page'), onClick: () => editor.deletePage(pageId), disabled: count <= 1 || locked },
    { key: 'add', icon: <Plus size={16} />, label: t('Add page'), onClick: () => editor.addPage(pageId) },
  ];
  return (
    <div className={`page-head${locked ? ' locked' : ''}`} style={{ left, top, width }} role="group" aria-label={page.title ? t('Page {n} – {title}', { n: index + 1, title: page.title }) : label} {...shield}>
      <span className="page-head-n">{label}{locked && <Lock size={12} aria-label={t('Locked')} />}{page.hidden && <span className="page-head-hidden">{t('(hidden)')}</span>}</span>
      {readOnly ? (
        page.title ? <span className="page-head-title">{page.title}</span> : null
      ) : (
        <input className="page-head-title" value={title} placeholder={compact ? '' : t('Add page title')} aria-label={t('Page title')} maxLength={120}
          onChange={(e) => setTitle(e.target.value)} onBlur={commit}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
            if (e.key === 'Escape') { setTitle(page.title ?? ''); (e.target as HTMLInputElement).blur(); }
          }} />
      )}
      {!readOnly && (compact ? (
        <PopoverButton menu className="btn ghost sm icon" title={t('Page actions')} label={<MoreHorizontal size={16} />} content={(close) => (
          <div className="menu" role="menu">
            {buttons.map((b) => <MenuItem key={b.key} icon={b.icon} label={b.label} disabled={b.disabled} onClick={() => { close(); b.onClick(); }} />)}
          </div>
        )} />
      ) : (
        <span className="page-head-actions">
          {buttons.map((b) => (
            <button key={b.key} className={`btn ghost sm icon${b.pressed ? ' active' : ''}`} title={b.label} aria-label={b.label} disabled={b.disabled}
              aria-pressed={b.pressed === undefined ? undefined : b.pressed} onClick={b.onClick}>{b.icon}</button>
          ))}
        </span>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- floating quick bar (g94)

export function QuickBar({ box, wrapWidth, onMore }: {
  /** The selection's screen-space bounds inside the canvas. */
  box: { x0: number; y0: number; x1: number; y1: number };
  wrapWidth: number;
  onMore: (anchor: HTMLElement) => void;
}) {
  const { editor, setDrawer } = useEd();
  const sel = editor.selected();
  const locked = sel.some((e) => userLock(e)?.move);
  const W = 196;
  const left = Math.max(8, Math.min((box.x0 + box.x1) / 2 - W / 2, wrapWidth - W - 8));
  // Above the selection; below it (past the rotate handle) when there is no room above.
  const top = box.y0 - 52 >= 4 ? box.y0 - 52 : box.y1 + 44;
  return (
    <div className="quick-bar" role="toolbar" aria-label={t('Selection actions')} style={{ left, top }} {...shield}>
      <button className="btn ghost sm icon" title={t('Comment')} aria-label={t('Comment')} onClick={() => setDrawer('comments')}><MessageSquarePlus size={16} /></button>
      <button className={`btn ghost sm icon${locked ? ' active' : ''}`} title={locked ? t('Unlock') : t('Lock')} aria-label={locked ? t('Unlock') : t('Lock')} aria-pressed={locked}
        onClick={() => editor.toggleLock()}>{locked ? <Lock size={16} /> : <Unlock size={16} />}</button>
      <button className="btn ghost sm icon" title={t('Duplicate (⌘D)')} aria-label={t('Duplicate')} onClick={() => editor.duplicate()}><Copy size={16} /></button>
      <button className="btn ghost sm icon" title={t('Delete')} aria-label={t('Delete')} disabled={locked} onClick={() => editor.deleteSelection()}><Trash2 size={16} /></button>
      <button className="btn ghost sm icon" title={t('More actions')} aria-label={t('More actions')} aria-haspopup="menu" onClick={(e) => onMore(e.currentTarget)}><MoreHorizontal size={16} /></button>
    </div>
  );
}
