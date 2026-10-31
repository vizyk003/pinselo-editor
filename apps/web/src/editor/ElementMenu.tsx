/**
 * The element and page context menu (g95), shared by right-click on the canvas, Shift+F10 / the context-menu key and
 * the floating quick bar's '…' button (g94), plus the small dialogs its entries open (link, alt text, info).
 */
import { childrenOf, resolvePage, sortedPages, type DesignDoc, type Editor, type Element, type ElementId, type ImageEl, type PageId } from '@oc/engine';
import { coverCrop } from '../lib/contentRules';
import {
  AlignCenter, Accessibility, ArrowDown, ArrowDownToLine, ArrowUp, ArrowUpToLine, ClipboardPaste, Copy, Crop, Download, FlipHorizontal, FlipVertical,
  Group, ImageIcon, Info, Layers, Link2, Lock, MessageSquarePlus, Paintbrush, Plus, Scissors, Trash2, Ungroup, Unlock,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import { MenuItem, Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { paintCanvas } from '../lib/engine';
import type { Engine } from '../lib/engine';
import { describe } from './A11y';
import { useEd } from './context';
import { openImageEraser } from './ImageEraser';
import { VectorMenuItems } from './VectorTools';

// ---------------------------------------------------------------- dialogs opened from menus and the quick bar

type ElementDialog = 'link' | 'alt' | 'info';
const DIALOG_EVENT = 'oc:element-dialog';

/** Open one of the selection's dialogs (rendered by {@link ElementDialogs}). */
export function openElementDialog(d: ElementDialog) {
  window.dispatchEvent(new CustomEvent<ElementDialog>(DIALOG_EVENT, { detail: d }));
}

/** Element types that take alternative text (same set as the toolbar's button). */
export const ALT_TYPES = ['image', 'video', 'frame', 'shape', 'grid', 'chart', 'qr', 'draw', 'group'];

export function ElementDialogs() {
  const { editor } = useEd();
  const [open, setOpen] = useState<ElementDialog | null>(null);
  useEffect(() => {
    const on = (e: Event) => setOpen((e as CustomEvent<ElementDialog>).detail);
    window.addEventListener(DIALOG_EVENT, on);
    return () => window.removeEventListener(DIALOG_EVENT, on);
  }, []);
  const el = editor.selected()[0];
  if (!open || !el) return null;
  const close = () => setOpen(null);
  if (open === 'link') return <LinkDialog el={el} onClose={close} />;
  if (open === 'alt') return <AltTextDialog el={el} onClose={close} />;
  return <InfoDialog el={el} onClose={close} />;
}

function LinkDialog({ el, onClose }: { el: Element; onClose: () => void }) {
  const { editor } = useEd();
  const [url, setUrl] = useState(el.link?.kind === 'url' ? el.link.url : '');
  const save = (v: string) => {
    const u = v.trim();
    editor.updateSelected({ link: u ? { kind: 'url', url: /^https?:\/\//i.test(u) ? u : `https://${u}` } : null }, false, false);
    onClose();
  };
  return (
    <Modal title={t('Link')} onClose={onClose} size="sm">
      <form className="col" onSubmit={(e) => { e.preventDefault(); save(url); }}>
        <label className="field">{t('Link to (URL)')}
          <input className="input" type="text" inputMode="url" placeholder="https://" value={url} onChange={(e) => setUrl(e.target.value)} autoFocus />
        </label>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          {el.link && <button type="button" className="btn" onClick={() => save('')}>{t('Remove link')}</button>}
          <button className="btn primary">{t('Apply')}</button>
        </div>
      </form>
    </Modal>
  );
}

function AltTextDialog({ el, onClose }: { el: Element; onClose: () => void }) {
  const { editor } = useEd();
  const [v, setV] = useState(el.altText ?? '');
  const [decorative, setDecorative] = useState(Boolean(el.decorative));
  return (
    <Modal title={t('Alternative text')} onClose={onClose} size="sm">
      <form className="col" onSubmit={(e) => { e.preventDefault(); editor.updateSelected({ altText: v.trim() || null, decorative: decorative || null }, false, false); onClose(); }}>
        <label className="field">{t('Alternative text')}
          <textarea className="input" rows={3} maxLength={500} value={v} onChange={(e) => setV(e.target.value)} placeholder={t('Describe what this shows for people using screen readers')} autoFocus />
        </label>
        <label className="check">
          <input type="checkbox" checked={decorative} onChange={(e) => setDecorative(e.target.checked)} />
          {t('Decorative — screen readers skip it')}
        </label>
        <div className="row" style={{ justifyContent: 'flex-end' }}><button className="btn primary">{t('Save')}</button></div>
      </form>
    </Modal>
  );
}

function InfoDialog({ el, onClose }: { el: Element; onClose: () => void }) {
  const { editor } = useEd();
  const doc = editor.doc;
  const assetId = el.type === 'image' ? el.asset : el.type === 'video' ? el.asset : el.type === 'frame' ? el.content?.asset : undefined;
  const a = assetId ? doc.assets[assetId] : undefined;
  const rows: Array<[string, string]> = [[t('Element'), describe(doc, el, true)]];
  if (a) {
    if (a.name) rows.push([t('File'), a.name]);
    if (a.w && a.h) rows.push([t('Original size'), t('{w} × {h} px', { w: a.w, h: a.h })]);
    if (a.attribution) {
      rows.push([t('Source'), a.attribution.provider]);
      if (a.attribution.author) rows.push([t('Author'), a.attribution.author]);
      if (a.attribution.license) rows.push([t('Licence'), a.attribution.license]);
    }
  }
  if (el.link?.kind === 'url') rows.push([t('Link'), el.link.url]);
  if (el.altText) rows.push([t('Alternative text'), el.altText]);
  const src = a?.attribution?.sourceUrl ?? a?.attribution?.authorUrl;
  return (
    <Modal title={t('Info')} onClose={onClose} size="sm">
      <dl className="info-list">
        {rows.map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{v}</dd></div>)}
      </dl>
      {src && /^https:\/\//.test(src) && <a href={src} target="_blank" rel="noreferrer noopener">{t('View source')}</a>}
      <div className="row" style={{ justifyContent: 'flex-end' }}><button className="btn" onClick={onClose} autoFocus>{t('Close')}</button></div>
    </Modal>
  );
}

// ---------------------------------------------------------------- actions

/** The selected elements as a transparent PNG at 2× (what the canvas shows, cropped to the selection). */
export async function downloadSelection(editor: Editor, engine: Engine): Promise<boolean> {
  const sel = editor.selected();
  const box = editor.selectionBox();
  const pageId = editor.state.activePageId;
  const doc = editor.doc;
  const page = doc.pages[pageId];
  if (!sel.length || !box || !page) return false;
  // Keep only the selected subtrees on that page, over a transparent page.
  const keep = new Set<string>();
  const walk = (id: ElementId) => {
    keep.add(id);
    for (const c of childrenOf(doc, id)) walk(c.id);
  };
  for (const e of sel) {
    walk(e.id);
    // A selection inside a group keeps its ancestors (without their other children), so it draws where it is.
    for (let p = doc.elements[e.id]?.parentId; p && doc.elements[p as ElementId]; p = doc.elements[p as ElementId]?.parentId) keep.add(p);
  }
  const elements = Object.fromEntries(Object.entries(doc.elements).filter(([id]) => keep.has(id)));
  const only: DesignDoc = { ...doc, pages: { [pageId]: { ...page, background: [], hidden: false } } as DesignDoc['pages'], elements: elements as DesignDoc['elements'] };
  const scale = 2;
  const full = document.createElement('canvas');
  const { w, h } = doc.meta.dims;
  paintCanvas(engine, full, w * scale, h * scale, [0, 0, 0, 0], resolvePage(only, pageId), scale);
  const out = document.createElement('canvas');
  const x = Math.max(0, Math.floor(box.x * scale)), y = Math.max(0, Math.floor(box.y * scale));
  out.width = Math.max(1, Math.min(full.width - x, Math.ceil(box.w * scale)));
  out.height = Math.max(1, Math.min(full.height - y, Math.ceil(box.h * scale)));
  out.getContext('2d')?.drawImage(full, x, y, out.width, out.height, 0, 0, out.width, out.height);
  const blob = await new Promise<Blob | null>((r) => out.toBlob(r, 'image/png'));
  if (!blob) return false;
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `${(doc.meta.title || 'design').replace(/[\\/:*?"<>|]+/g, ' ').trim() || 'design'} – ${t('selection')}.png`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return true;
}

/** A picture becomes the page background (filled, keeping its crop) and leaves the page, in one undo step. */
export function setImageAsBackground(editor: Editor, el: ImageEl) {
  const pageId = editor.state.activePageId;
  editor.apply([
    // The image's crop, narrowed to the page's aspect so the background covers the page without stretching (g63).
    { op: 'page', action: 'update', pageId, patch: { background: [{ k: 'image', asset: el.asset, fit: 'fill', crop: coverCrop(el.crop, el.xf.w / el.xf.h, editor.doc.meta.dims.w / editor.doc.meta.dims.h) }] } },
    { op: 'delete', id: el.id },
  ]);
  editor.clearSelection();
}

/** Lock or unlock a page (g96); locking drops a selection on it. */
export function togglePageLock(editor: Editor, pageId: PageId) {
  const locked = Boolean(editor.doc.pages[pageId]?.locked);
  editor.updatePage(pageId, { locked: locked ? undefined : true });
  if (!locked && editor.state.activePageId === pageId) editor.clearSelection();
}

// ---------------------------------------------------------------- the menu

export function ElementMenu({ close }: { close: () => void }) {
  const { editor, engine, setDrawer } = useEd();
  const toast = useToast();
  const sel = editor.selected();
  const run = (fn: () => void) => () => {
    fn();
    close();
  };
  const pageId = editor.state.activePageId;
  const page = editor.doc.pages[pageId];
  if (!sel.length) {
    const pageLocked = Boolean(page?.locked);
    const many = sortedPages(editor.doc).length > 1;
    return (
      <div className="menu" role="menu">
        <MenuItem icon={<ClipboardPaste size={16} />} label={t('Paste')} kbd="⌘V" onClick={run(() => editor.paste())} disabled={pageLocked} />
        <MenuItem label={t('Select all')} kbd="⌘A" onClick={run(() => editor.selectAll())} disabled={pageLocked} />
        <div className="hsep" />
        <MenuItem icon={<Plus size={16} />} label={t('Add page')} onClick={run(() => editor.addPage(pageId))} />
        <MenuItem icon={<Copy size={16} />} label={t('Duplicate page')} onClick={run(() => editor.duplicatePage())} />
        <MenuItem icon={pageLocked ? <Unlock size={16} /> : <Lock size={16} />} label={pageLocked ? t('Unlock page') : t('Lock page')} onClick={run(() => togglePageLock(editor, pageId))} />
        <MenuItem icon={<Trash2 size={16} />} label={t('Delete page')} danger onClick={run(() => editor.deletePage())} disabled={!many || pageLocked} />
      </div>
    );
  }
  const locked = sel.some((e) => e.lock?.move);
  const single = sel.length === 1 ? sel[0]! : null;
  const image = single?.type === 'image' ? single : null;
  return (
    <div className="menu" role="menu">
      <MenuItem icon={<Copy size={16} />} label={t('Copy')} kbd="⌘C" onClick={run(() => editor.copy())} />
      <MenuItem icon={<ClipboardPaste size={16} />} label={t('Paste')} kbd="⌘V" onClick={run(() => editor.paste())} />
      <MenuItem icon={<Copy size={16} />} label={t('Duplicate')} kbd="⌘D" onClick={run(() => editor.duplicate())} />
      <MenuItem icon={<Trash2 size={16} />} label={t('Delete')} kbd="⌫" danger onClick={run(() => editor.deleteSelection())} disabled={locked} />
      <MenuItem icon={<Paintbrush size={16} />} label={t('Copy style')} kbd="⌥⌘C" onClick={run(() => editor.copyStyle())} />
      <div className="hsep" />
      <MenuItem icon={<MessageSquarePlus size={16} />} label={t('Comment')} onClick={run(() => setDrawer('comments'))} />
      {single && <MenuItem icon={<Link2 size={16} />} label={t('Link')} kbd="⌘K" onClick={run(() => openElementDialog('link'))} />}
      {single && ALT_TYPES.includes(single.type) && <MenuItem icon={<Accessibility size={16} />} label={t('Alternative text')} onClick={run(() => openElementDialog('alt'))} />}
      <div className="hsep" />
      <MenuItem icon={<ArrowUp size={16} />} label={t('Bring forward')} kbd="⌘]" onClick={run(() => editor.arrange('forward'))} />
      <MenuItem icon={<ArrowUpToLine size={16} />} label={t('Bring to front')} kbd="⌥⌘]" onClick={run(() => editor.arrange('front'))} />
      <MenuItem icon={<ArrowDown size={16} />} label={t('Send backward')} kbd="⌘[" onClick={run(() => editor.arrange('backward'))} />
      <MenuItem icon={<ArrowDownToLine size={16} />} label={t('Send to back')} kbd="⌥⌘[" onClick={run(() => editor.arrange('back'))} />
      <MenuItem icon={<Layers size={16} />} label={t('Show layers')} onClick={run(() => setDrawer('layers'))} />
      <div className="hsep" />
      {sel.length > 1 && <MenuItem icon={<Group size={16} />} label={t('Group')} kbd="⌘G" onClick={run(() => editor.group())} />}
      {sel.some((e) => e.type === 'group') && <MenuItem icon={<Ungroup size={16} />} label={t('Ungroup')} kbd="⇧⌘G" onClick={run(() => editor.ungroup())} />}
      <MenuItem icon={locked ? <Unlock size={16} /> : <Lock size={16} />} label={locked ? t('Unlock') : t('Lock')} kbd="⌥⇧L" onClick={run(() => editor.toggleLock())} />
      <MenuItem icon={<FlipHorizontal size={16} />} label={t('Flip horizontal')} onClick={run(() => editor.flip('x'))} />
      <MenuItem icon={<FlipVertical size={16} />} label={t('Flip vertical')} onClick={run(() => editor.flip('y'))} />
      <MenuItem icon={<AlignCenter size={16} />} label={t('Align to page: centre')} onClick={run(() => { editor.align('hcenter'); editor.align('vcenter'); })} />
      {single && (single.type === 'image' || (single.type === 'frame' && single.content)) && (
        <MenuItem icon={<Crop size={16} />} label={t('Crop')} onClick={run(() => editor.startCrop(single.id))} />
      )}
      {image && <MenuItem icon={<ImageIcon size={16} />} label={t('Set image as background')} onClick={run(() => setImageAsBackground(editor, image))} disabled={locked} />}
      {image && <MenuItem icon={<Scissors size={16} />} label={t('Erase parts of the image')} onClick={run(() => openImageEraser(image.id))} disabled={Boolean(image.lock?.content)} />}
      <VectorMenuItems sel={sel} run={run} />
      <div className="hsep" />
      <MenuItem icon={<Download size={16} />} label={t('Download selection (PNG)')} onClick={run(() => void downloadSelection(editor, engine).then((ok) => !ok && toast(t('Nothing to download'))))} />
      {single && <MenuItem icon={<Info size={16} />} label={t('Info')} onClick={run(() => openElementDialog('info'))} />}
    </div>
  );
}
