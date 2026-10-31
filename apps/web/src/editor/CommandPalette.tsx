/**
 * Command palette (A17): press "/" (like Canva's Magic Shortcuts) or ⌘K-less search for any action —
 * add elements, page actions, alignment, panels, dialogs. Arrow keys + Enter, or click.
 */
import { makeShape, makeText, makeLine } from '@oc/engine';
import { useEffect, useMemo, useRef, useState } from 'react';
import { t, tk } from '../i18n';
import { useEd } from './context';
import { setViewPrefs, getViewPrefs } from './viewPrefs';

export interface PaletteActions {
  openDialog: (d: 'share' | 'export' | 'resize' | 'shortcuts' | 'find' | 'grid' | 'publish' | 'bulk' | 'site') => void;
  openPanel: (p: string) => void;
  openDrawer: (d: 'comments' | 'versions' | 'layers' | 'a11y') => void;
  present: () => void;
}

interface Cmd { label: string; keywords?: string; kbd?: string; run: () => void; when?: () => boolean }

export function CommandPalette({ onClose, actions }: { onClose: () => void; actions: PaletteActions }) {
  const { editor, readOnly } = useEd();
  const [q, setQ] = useState('');
  const [i, setI] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const s = () => Math.min(editor.doc.meta.dims.w, editor.doc.meta.dims.h) * 0.25;
  const has = () => editor.state.selection.length > 0;
  const cmds: Cmd[] = useMemo(() => [
    { label: tk('Add a heading'), keywords: 'text title', kbd: 'T', run: () => editor.insert([makeText({ xf: { x: 0, y: 0, w: s() * 2, h: s() * 0.4, rot: 0 }, text: t('Add a heading'), style: { size: Math.round(s() * 0.25), weight: 700 } })], undefined, { edit: true }) },
    { label: tk('Add body text'), keywords: 'text paragraph', run: () => editor.insert([makeText({ xf: { x: 0, y: 0, w: s() * 2, h: s() * 0.2, rot: 0 }, text: t('Add a little bit of body text'), style: { size: Math.round(s() * 0.1) } })], undefined, { edit: true }) },
    { label: tk('Add a rectangle'), keywords: 'shape square box', kbd: 'R', run: () => editor.insert([makeShape({ kind: 'rect', xf: { x: 0, y: 0, w: s(), h: s(), rot: 0 } })]) },
    { label: tk('Add a circle'), keywords: 'shape ellipse round', kbd: 'C', run: () => editor.insert([makeShape({ kind: 'ellipse', xf: { x: 0, y: 0, w: s(), h: s(), rot: 0 } })]) },
    { label: tk('Add a line'), keywords: 'divider stroke', kbd: 'L', run: () => editor.insert([makeLine({ xf: { x: 0, y: 0, w: s() * 1.5, h: 20, rot: 0 } })]) },
    { label: tk('Add a page'), keywords: 'new slide', kbd: '⌘↵', run: () => editor.addPage() },
    { label: tk('Duplicate page'), keywords: 'copy slide', run: () => editor.duplicatePage() },
    { label: tk('Delete page'), keywords: 'remove slide', kbd: '⌘⌫', run: () => editor.deletePage() },
    { label: tk('Duplicate'), keywords: 'copy clone', kbd: '⌘D', run: () => editor.duplicate(), when: has },
    { label: tk('Delete'), keywords: 'remove', kbd: '⌫', run: () => editor.deleteSelection(), when: has },
    { label: tk('Group'), kbd: '⌘G', run: () => editor.group(), when: () => editor.state.selection.length > 1 },
    { label: tk('Ungroup'), kbd: '⇧⌘G', run: () => editor.ungroup(), when: has },
    { label: tk('Lock / unlock'), run: () => editor.toggleLock(), when: has },
    { label: tk('Align left'), run: () => editor.align('left'), when: has },
    { label: tk('Align centre'), keywords: 'center middle', run: () => editor.align('hcenter'), when: has },
    { label: tk('Align right'), run: () => editor.align('right'), when: has },
    { label: tk('Align top'), run: () => editor.align('top'), when: has },
    { label: tk('Align middle'), keywords: 'vertical center', run: () => editor.align('vcenter'), when: has },
    { label: tk('Align bottom'), run: () => editor.align('bottom'), when: has },
    { label: tk('Distribute horizontally'), keywords: 'space evenly', run: () => editor.distribute('x'), when: () => editor.state.selection.length > 2 },
    { label: tk('Distribute vertically'), keywords: 'space evenly', run: () => editor.distribute('y'), when: () => editor.state.selection.length > 2 },
    { label: tk('Tidy up'), keywords: 'arrange grid', run: () => editor.tidy(), when: () => editor.state.selection.length > 1 },
    { label: tk('Bring to front'), kbd: '⌥⌘]', run: () => editor.arrange('front'), when: has },
    { label: tk('Send to back'), kbd: '⌥⌘[', run: () => editor.arrange('back'), when: has },
    { label: tk('Select all'), kbd: '⌘A', run: () => editor.selectAll() },
    { label: tk('Undo'), kbd: '⌘Z', run: () => editor.undo() },
    { label: tk('Redo'), kbd: '⇧⌘Z', run: () => editor.redo() },
    { label: tk('Zoom to fit'), kbd: '⌘0', run: () => editor.zoomToFit() },
    { label: tk('Show or hide rulers'), keywords: 'guides', kbd: '⇧R', run: () => setViewPrefs({ rulers: !getViewPrefs().rulers }) },
    { label: tk('Present'), keywords: 'slideshow play', kbd: '⌥⌘P', run: actions.present },
    { label: tk('Share'), run: () => actions.openDialog('share') },
    { label: tk('Download'), keywords: 'export pdf png mp4', run: () => actions.openDialog('export') },
    { label: tk('Resize'), keywords: 'magic resize dimensions', run: () => actions.openDialog('resize') },
    { label: tk('Find and replace'), kbd: '⌘F', run: () => actions.openDialog('find') },
    { label: tk('Grid view'), keywords: 'pages overview', run: () => actions.openDialog('grid') },
    { label: tk('Keyboard shortcuts'), kbd: '?', run: () => actions.openDialog('shortcuts') },
    { label: tk('Comments'), run: () => actions.openDrawer('comments') },
    { label: tk('Version history'), keywords: 'restore', run: () => actions.openDrawer('versions') },
    { label: tk('Layers'), run: () => actions.openDrawer('layers') },
    { label: tk('Check accessibility'), keywords: 'contrast alt text', run: () => actions.openDrawer('a11y') },
    { label: tk('Templates'), run: () => actions.openPanel('templates') },
    { label: tk('Styles'), keywords: 'palette colours fonts', run: () => actions.openPanel('styles') },
    { label: tk('Elements'), keywords: 'shapes graphics frames charts', run: () => actions.openPanel('elements') },
    { label: tk('Uploads'), keywords: 'images videos', run: () => actions.openPanel('uploads') },
    { label: tk('Projects'), keywords: 'folders designs insert pages', run: () => actions.openPanel('projects') },
    { label: tk('Brand'), keywords: 'kit logo colours', run: () => actions.openPanel('brand') },
    // eslint-disable-next-line react-hooks/exhaustive-deps
  ], [editor, actions]);
  const needle = q.trim().toLowerCase();
  const shown = cmds.filter((c) => (!c.when || c.when()) && (!readOnly || /Present|Download|Comments|Grid|Keyboard|Zoom|rulers|Version|Layers|Projects/.test(c.label)))
    .filter((c) => !needle || `${t(c.label)} ${c.label} ${c.keywords ?? ''}`.toLowerCase().includes(needle));
  useEffect(() => setI(0), [q]);
  useEffect(() => { list.current?.querySelector<HTMLElement>(`[data-i="${i}"]`)?.scrollIntoView({ block: 'nearest' }); }, [i]);
  const run = (c: Cmd | undefined) => { if (!c) return; onClose(); setTimeout(c.run, 0); };
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal cmdk" role="dialog" aria-modal="true" aria-label={t('Command palette')}>
        <input className="input" autoFocus placeholder={t('Type a command or search…')} value={q} onChange={(e) => setQ(e.target.value)} role="combobox" aria-expanded="true" aria-controls="cmdk-list" aria-activedescendant={`cmdk-${i}`}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') { e.preventDefault(); setI((x) => Math.min(shown.length - 1, x + 1)); }
            else if (e.key === 'ArrowUp') { e.preventDefault(); setI((x) => Math.max(0, x - 1)); }
            else if (e.key === 'Enter') { e.preventDefault(); run(shown[i]); }
            else if (e.key === 'Escape') onClose();
          }} />
        <div className="cmdk-list" id="cmdk-list" role="listbox" ref={list}>
          {shown.map((c, k) => (
            <div key={c.label} id={`cmdk-${k}`} data-i={k} role="option" aria-selected={k === i} className={`cmdk-item ${k === i ? 'on' : ''}`} onMouseEnter={() => setI(k)} onClick={() => run(c)}>
              <span className="grow">{t(c.label)}</span>{c.kbd && <span className="kbd">{c.kbd}</span>}
            </div>
          ))}
          {!shown.length && <div className="faint" style={{ padding: 12 }}>{t('No matching commands')}</div>}
        </div>
      </div>
    </div>
  );
}
