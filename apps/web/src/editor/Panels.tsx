/** Object panel contents (docs/plan/04-ux-spec.md §3.3). */
import { templatePages } from '@oc/engine';
import { childrenOf, makeText, sortedPages, type Element, type ElementId } from '@oc/engine';
import { Video, Music, Eye, EyeOff, Group, Image as ImageIcon, Lock, Shapes, Trash2, Type, Upload, Minus, Square, Circle, QrCode } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { api, ApiError, assetUrl, plansEnabled, templateThumbUrl, type TemplateSummary, type UploadedAsset } from '../api';
import { useAuth } from '../App';
import { tabPanelProps, tabProps } from '../components/tabs';
import { SearchField, useScrollFade, useToast } from '../components/ui';
import { SwatchInput } from './ColorPicker';
import { getLocale, t, tk } from '../i18n';
import { useDoc, useEd, useEditorState } from './context';
import { TemplateDetailsPopover } from './TemplateDetails';
import { recentTemplates, recommendTemplates, recordRecentTemplate } from '../lib/templates';
import { assetRef, insertAssetAt, uploadAndInsert } from './insert';
import { setAssetDrag } from './dropTarget';
import { Camera, ChevronLeft, FolderUp } from 'lucide-react';
import { uploadsApi, type UploadPage } from '../api/uploads';
import { filesFromDrop, folderInputRef, uploadMany } from '../lib/uploadFiles';
import { uploadSummary } from '../pages/home/Uploads';
import { fmtBytes, fmtDuration, uploadTypeOf, type UploadType } from '../pages/home/uploadsModel';
import { VideoPanel } from './VideoPanel';
import { shapeLabel } from './ElementsPanel';
import { EraserTile, SignatureSection } from './Signature';

// ------------------------------------------------------------------ templates

export function TemplatesPanel() {
  const { editor } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const [q, setQ] = useState('');
  const [list, setList] = useState<TemplateSummary[] | null>(null);
  const me = useAuth().user?.id ?? null;
  const [recent, setRecent] = useState(() => recentTemplates(me));
  // Clicking a template opens its details (g25); Apply in the popover adds it.
  const [picked, setPicked] = useState<{ tpl: TemplateSummary; anchor: HTMLElement } | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => api.templates(q ? { q } : {}).then((r) => setList(r.templates)), 200);
    return () => clearTimeout(timer);
  }, [q]);
  const { w, h } = doc.meta.dims;
  const ratio = w / h;
  // Templates with a similar aspect ratio first.
  const sorted = list ? [...list].sort((a, b) => Math.abs(Math.log(a.width / a.height / ratio)) - Math.abs(Math.log(b.width / b.height / ratio))) : null;

  // Recently used and "More templates for you" (g26), shown above the full list when not searching.
  const known = new Map((list ?? []).map((x) => [x.id, x]));
  const recentShown = recent.flatMap((r) => known.get(r.id) ?? []).slice(0, 4);
  const originId = doc.meta.templateOrigin?.templateId;
  const originCategory = originId ? (known.get(originId) ?? recent.find((r) => r.id === originId))?.category : null;
  const forYou = list && !q
    ? recommendTemplates(list, { surface: doc.meta.surface, w, h, category: originCategory }, {
      exclude: [...recentShown.map((x) => x.id), ...(originId ? [originId] : [])], recentCategories: recent.map((r) => r.category), limit: 4,
    })
    : [];

  const apply = async (tpl: TemplateSummary) => {
    let template;
    try {
      ({ template } = await api.template(tpl.id));
    } catch (e) {
      // A Pro template on the free plan: the global 402 handler already shows the upgrade dialog.
      if (!(e instanceof ApiError && e.status === 402)) toast(t('Could not apply template'));
      return;
    }
    const imported = templatePages(template.doc, { w, h });
    const pages = sortedPages(editor.doc);
    const active = editor.state.activePageId;
    const activeEmpty = childrenOf(editor.doc, active).length === 0;
    const ops: Parameters<typeof editor.apply>[0] = [];
    let after = active;
    for (const ip of imported) {
      ops.push({ op: 'page', action: 'add', page: { ...ip.page }, after });
      after = ip.page.id;
      for (const el of ip.elements) ops.push({ op: 'add', el, parent: el.parentId });
    }
    if (activeEmpty && pages.length >= 1) ops.push({ op: 'page', action: 'delete', pageId: active });
    for (const a of Object.values(template.doc.assets ?? {})) ops.unshift({ op: 'asset', asset: a });
    const r = editor.apply(ops);
    if (r.ok) {
      editor.setActivePage(imported[0]!.page.id, true);
      setRecent(recordRecentTemplate(me, tpl));
      toast(imported.length > 1
        ? t('Added “{title}” ({n} pages)', { title: tpl.title, n: imported.length })
        : t('Added “{title}”', { title: tpl.title }));
    } else toast(t('Could not apply template'));
  };

  const tiles = (items: TemplateSummary[], key: string) => (
    <div className="tiles two">
      {items.map((tpl) => (
        <button
          key={`${key}:${tpl.id}`} className="tile" style={{ aspectRatio: 'auto', minHeight: 120, position: 'relative' }} aria-haspopup="dialog"
          onClick={(e) => setPicked({ tpl, anchor: e.currentTarget })} title={`${tpl.title} · ${tpl.category}${tpl.premium && plansEnabled() ? ' · Pro' : ''}`}
        >
          <img src={templateThumbUrl(tpl.id)} alt={tpl.title} style={{ objectFit: 'contain' }} loading="lazy" />
          {tpl.premium && plansEnabled() && <span className="premium-badge">{t('PRO')}</span>}
          {tpl.team && <span className="premium-badge team">{t('TEAM')}</span>}
        </button>
      ))}
    </div>
  );

  return (
    <>
      <SearchField value={q} onChange={setQ} placeholder={t('Search templates')} onClear={() => setQ('')} />
      {!sorted && <div className="empty">{t('Loading…')}</div>}
      {!q && recentShown.length > 0 && <><h4>{t('Recently used')}</h4>{tiles(recentShown, 'recent')}</>}
      {!q && forYou.length > 0 && <><h4>{t('More templates for you')}</h4>{tiles(forYou, 'rec')}</>}
      {!q && sorted && (recentShown.length > 0 || forYou.length > 0) && <h4>{t('All templates')}</h4>}
      {sorted && tiles(sorted, 'all')}
      {sorted && !sorted.length && <div className="empty">{t('No templates found.')}</div>}
      {picked && (
        <TemplateDetailsPopover
          tpl={picked.tpl} anchor={picked.anchor} onClose={() => setPicked(null)}
          onApply={() => { setPicked(null); void apply(picked.tpl); }}
        />
      )}
    </>
  );
}

// ------------------------------------------------------------------ elements and text (ElementsPanel.tsx, TextPanel.tsx)

export { ElementsPanel } from './ElementsPanel';
export { TextPanel } from './TextPanel';

// ------------------------------------------------------------------ draw

const BRUSH_COLORS = ['#1f2328', '#ef4444', '#f59e0b', '#10b981', '#3b82f6', '#8b5cf6', '#ec4899', '#ffffff'];

export function DrawPanel() {
  const { editor } = useEd();
  const state = useEditorState();
  useEffect(() => {
    editor.setTool('draw');
    return () => editor.setTool('select');
  }, [editor]);
  const b = state.brush;
  const erasing = state.tool === 'erase';
  return (
    <>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Draw directly on the page. Press Esc or close this panel to go back to selecting.')}</p>
      <h4>{t('Brush')}</h4>
      <div className="tiles">
        {(['pen', 'marker', 'highlighter'] as const).map((k) => (
          <button key={k} className="tile" aria-pressed={state.tool === 'draw' && b.kind === k} style={{ flexDirection: 'column', gap: 4, borderColor: state.tool === 'draw' && b.kind === k ? 'var(--accent)' : undefined }} onClick={() => { editor.setTool('draw'); editor.setBrush({ kind: k }); }}>
            <svg viewBox="0 0 60 30" style={{ width: '80%', height: '40%' }} aria-hidden>
              <path d="M4 22 Q18 2 30 16 T56 10" fill="none" stroke={b.color} strokeOpacity={k === 'highlighter' ? 0.4 : 1} strokeWidth={k === 'pen' ? 3 : k === 'marker' ? 6 : 12} strokeLinecap={k === 'highlighter' ? 'square' : 'round'} />
            </svg>
            <span style={{ fontSize: 12, textTransform: 'capitalize' }}>{{ pen: t('Pen'), marker: t('Marker'), highlighter: t('Highlighter') }[k]}</span>
          </button>
        ))}
        <EraserTile active={erasing} onClick={() => editor.setTool(erasing ? 'draw' : 'erase')} />
      </div>
      <h4>{t('Paths')}</h4>
      <div className="tiles">
        <button className="tile" aria-pressed={state.tool === 'pen'} style={{ flexDirection: 'column', gap: 4, borderColor: state.tool === 'pen' ? 'var(--accent)' : undefined }}
          onClick={() => editor.setTool(state.tool === 'pen' ? 'draw' : 'pen')}>
          <svg viewBox="0 0 60 30" style={{ width: '80%', height: '40%' }} aria-hidden>
            <path d="M6 24 C16 4 30 4 36 16 S50 26 54 8" fill="none" stroke="currentColor" strokeWidth={2} />
            {[[6, 24], [36, 16], [54, 8]].map(([x, y]) => <rect key={x} x={x! - 3} y={y! - 3} width={6} height={6} fill="var(--surface)" stroke="var(--accent)" strokeWidth={1.5} />)}
          </svg>
          <span style={{ fontSize: 12 }}>{t('Vector pen')}</span>
        </button>
      </div>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Click to add points, drag to curve, click the first point to close. Double-click or press Enter to finish. Shortcut: P.')}</p>
      <h4>{t('Colour')}</h4>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        {BRUSH_COLORS.map((c) => (
          <button key={c} onClick={() => editor.setBrush({ color: c })} aria-label={c} style={{ width: 30, height: 30, borderRadius: '50%', background: c, border: b.color === c ? '3px solid var(--accent)' : '1px solid var(--border-strong)', cursor: 'pointer' }} />
        ))}
        <SwatchInput value={b.color} onChange={(c) => editor.setBrush({ color: c })} title={t('Custom colour')} doc={editor.doc} />
      </div>
      <label className="field">{t('Weight ({value})', { value: b.size })}
        <input type="range" min={1} max={40} value={b.size} onChange={(e) => editor.setBrush({ size: +e.target.value })} />
      </label>
      <SignatureSection />
    </>
  );
}

// ------------------------------------------------------------------ uploads

type UploadTab = Exclude<UploadType, 'files'>;
const UPLOAD_TABS: Array<{ id: UploadTab; label: string }> = [{ id: 'images', label: tk('Images') }, { id: 'videos', label: tk('Videos') }, { id: 'audio', label: tk('Audio') }];
const UPLOAD_TAB_IDS = UPLOAD_TABS.map((x) => x.id);
const dragsFiles = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');

/**
 * Uploads panel (g87): name search, Images / Videos / Audio tabs, a masonry grid, upload files (placed on the page) or a
 * whole folder (added to the library), drop files or folders here, and a Record entry. Click a tile or drag it onto the
 * page. `onRecord` opens the Video panel's recording tools; without it they open here.
 */
export function UploadsPanel({ onRecord }: { onRecord?: () => void } = {}) {
  const { editor } = useEd();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement | null>(null);
  const [tab, setTab] = useState<UploadTab>('images');
  const tabsId = useId();
  const tabsRow = useScrollFade();
  const [q, setQ] = useState('');
  const [query, setQuery] = useState('');
  const [data, setData] = useState<UploadPage | null>(null);
  const [busy, setBusy] = useState<{ done: number; total: number } | null>(null);
  const [recording, setRecording] = useState(false);
  const [over, setOver] = useState(false);
  const depth = useRef(0);
  const seq = useRef(0);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(q.trim()), 200);
    return () => clearTimeout(timer);
  }, [q]);
  const reload = useCallback(async (offset = 0) => {
    const mine = ++seq.current;
    const r = await uploadsApi.list({ q: query, type: tab, offset });
    if (mine !== seq.current) return;
    setData((cur) => (offset && cur ? { ...r, assets: [...cur.assets, ...r.assets] } : r));
  }, [query, tab]);
  useEffect(() => {
    setData(null);
    reload().catch(() => toast(t('Couldn’t load your uploads.')));
  }, [reload, toast]);

  /** The button uploads and places each file (as before); a folder or a drop only adds to the library. */
  const upload = async (files: File[], place: boolean) => {
    if (!files.length || busy) return;
    let last: UploadType | null = null;
    if (place) {
      setBusy({ done: 0, total: files.length });
      for (const [i, f] of files.entries()) {
        try {
          last = uploadTypeOf(await uploadAndInsert(editor, f));
        } catch (e) {
          toast(`${f.name}: ${(e as Error).message}`);
        }
        setBusy({ done: i + 1, total: files.length });
      }
    } else {
      const r = await uploadMany(files, (done, total) => setBusy({ done, total }));
      toast(uploadSummary(r));
      const a = r.assets.at(-1);
      if (a) last = uploadTypeOf(a);
    }
    setBusy(null);
    // Show what was just added.
    if (last && last !== 'files' && last !== tab) setTab(last);
    else void reload();
  };
  const remove = async (a: UploadedAsset) => {
    try {
      await uploadsApi.moveToTrash(a.id);
      toast(t('Moved to trash'));
    } catch {
      toast(t('Couldn’t move that to the trash. Try again.'));
    }
    void reload();
  };

  if (recording) {
    return (
      <>
        <button className="btn sm ghost" style={{ alignSelf: 'flex-start' }} onClick={() => setRecording(false)}><ChevronLeft size={16} /> {t('Back to uploads')}</button>
        <VideoPanel />
      </>
    );
  }

  const items = data?.assets ?? [];
  // Marks the drag as media so shapes, frames and grid cells light up as drop targets (drop to fill).
  const drag = (a: UploadedAsset) => (e: React.DragEvent) => setAssetDrag(e.dataTransfer, assetRef(a));
  const insert = (a: UploadedAsset) => insertAssetAt(editor, assetRef(a), undefined);
  const del = (a: UploadedAsset) => (
    <button className="btn sm icon up-del" title={t('Delete upload')} aria-label={t('Delete upload')} onClick={() => void remove(a)}><Trash2 size={12} /></button>
  );
  return (
    <div
      className={`up-panel-drop${over ? ' over' : ''}`}
      onDragEnter={(e) => { if (!dragsFiles(e)) return; e.preventDefault(); depth.current++; setOver(true); }}
      onDragOver={(e) => { if (!dragsFiles(e)) return; e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }}
      onDragLeave={() => { if (--depth.current <= 0) { depth.current = 0; setOver(false); } }}
      onDrop={(e) => {
        if (!dragsFiles(e)) return;
        e.preventDefault();
        depth.current = 0;
        setOver(false);
        void filesFromDrop(e.dataTransfer).then((f) => upload(f, false));
      }}
    >
      <div className="up-panel-actions">
        <button className="btn primary" onClick={() => input.current?.click()} disabled={Boolean(busy)}>
          <Upload size={16} /> {busy ? t('Uploading {n}…', { n: busy.total - busy.done }) : t('Upload files')}
        </button>
        <button className="btn icon" title={t('Upload a folder')} aria-label={t('Upload a folder')} onClick={() => folder.current?.click()} disabled={Boolean(busy)}><FolderUp size={16} /></button>
        <button className="btn icon" title={t('Record yourself or your screen')} aria-label={t('Record yourself or your screen')} onClick={() => (onRecord ? onRecord() : setRecording(true))}><Camera size={16} /></button>
      </div>
      <input ref={input} type="file" accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml,video/*,audio/*" multiple hidden
        onChange={(e) => { const f = [...(e.target.files ?? [])]; e.target.value = ''; void upload(f, true); }} />
      <input ref={(el) => { folder.current = el; folderInputRef(el); }} type="file" multiple hidden
        onChange={(e) => { const f = [...(e.target.files ?? [])]; e.target.value = ''; void upload(f, false); }} />
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Images, video (MP4, MOV, WebM) or audio (MP3, M4A, WAV) · or drag files onto the page.')}
        {data && ` ${t('{used} of {quota} used', { used: fmtBytes(data.usedBytes, getLocale()), quota: fmtBytes(data.quotaBytes, getLocale()) })}.`}</p>
      <SearchField value={q} onChange={setQ} placeholder={t('Search your uploads')} onClear={() => setQ('')} />
      <div className="lib-tabs" role="tablist" aria-label={t('Type')} ref={tabsRow}>
        {UPLOAD_TABS.map((x) => (
          <button key={x.id} {...tabProps(tabsId, UPLOAD_TAB_IDS, tab, setTab, x.id)} className={tab === x.id ? 'on' : ''}>
            {t(x.label)}{data ? ` · ${data.counts[x.id]}` : ''}
          </button>
        ))}
      </div>
      <div {...tabPanelProps(tabsId, tab)} className="col" style={{ gap: 10 }}>
      {!data ? <div className="spinner" />
        : !items.length ? <div className="empty"><ImageIcon size={28} /><br />{query ? t('No uploads match your search.') : t('Your uploads will appear here.')}</div>
          : tab === 'audio' ? (
            <div className="list">
              {items.map((a) => (
                <div key={a.id} className="item up-audio-row">
                  <button className="audio-row" style={{ display: 'flex', alignItems: 'center', gap: 10, flex: 1, minWidth: 0, padding: 0 }} title={a.name} draggable onDragStart={drag(a)} onClick={() => insert(a)}>
                    <Music size={16} />
                    <div className="main"><div className="t">{a.name}</div><div className="sub">{fmtBytes(Number(a.bytes), getLocale())}</div></div>
                    <span className="faint" style={{ fontSize: 12 }}>{a.durationMs ? fmtDuration(a.durationMs) : ''}</span>
                  </button>
                  {del(a)}
                </div>
              ))}
            </div>
          ) : (
            <div className="masonry">
              {items.map((a) => (
                <div key={a.id} className="m-tile">
                  <button className="up-ins" title={a.name} draggable onDragStart={drag(a)} onClick={() => insert(a)}>
                    <img src={a.kind === 'video' ? `${assetUrl(a.id)}/poster` : assetUrl(a.id)} alt={a.name} loading="lazy" style={{ aspectRatio: a.w && a.h ? `${a.w} / ${a.h}` : undefined }} />
                    {a.kind === 'video' && !a.animated && a.durationMs ? <span className="dur">{fmtDuration(a.durationMs)}</span> : null}
                  </button>
                  {del(a)}
                </div>
              ))}
            </div>
          )}
      {data && items.length < data.total && <button className="btn sm" onClick={() => void reload(items.length)}>{t('Load more')}</button>}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ layers

const TYPE_ICON: Record<string, typeof Square> = { shape: Shapes, text: Type, image: ImageIcon, frame: Square, group: Group, line: Minus, qr: QrCode, video: Video };
const TYPE_LABELS: Record<string, string> = {
  shape: tk('Shape'), text: tk('Text'), image: tk('Image'), frame: tk('Frame'), group: tk('Group'), line: tk('Line'), qr: tk('QR code'), video: tk('Video'),
  draw: tk('Drawing'), table: tk('Table'), chart: tk('Chart'), grid: tk('Photo grid'), form: tk('Form'),
};

function layerName(el: Element): string {
  if (el.name) return el.name;
  if (el.type === 'text') return el.body.paragraphs.map((p) => p.runs.map((r) => r.text).join('')).join(' ').slice(0, 40) || t('Text');
  if (el.type === 'shape') return shapeLabel(el.kind);
  return TYPE_LABELS[el.type] ? t(TYPE_LABELS[el.type]!) : el.type[0]!.toUpperCase() + el.type.slice(1);
}

export function LayersPanel() {
  const { editor } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const [drag, setDrag] = useState<ElementId | null>(null);
  const [over, setOver] = useState<ElementId | null>(null);
  const page = state.activePageId;
  const rows: Array<{ el: Element; depth: number }> = [];
  const walk = (parent: string, depth: number) => {
    const kids = childrenOf(doc, parent as ElementId);
    for (let i = kids.length - 1; i >= 0; i--) {
      const el = kids[i]!;
      rows.push({ el, depth });
      if (el.type === 'group') walk(el.id, depth + 1);
    }
  };
  walk(page, 0);
  if (!rows.length) return <div className="empty">{t('This page is empty. Add something from Elements or Text.')}</div>;
  return (
    <div className="col" style={{ gap: 2 }}>
      <p className="faint" style={{ margin: '0 0 6px', fontSize: 12 }}>{t('Top of the list is in front. Drag to reorder.')}</p>
      {rows.map(({ el, depth }) => {
        const Icon = TYPE_ICON[el.type] ?? Circle;
        const hidden = el.visible === false;
        return (
          <div key={el.id}
            className={`layer-row ${state.selection.includes(el.id) ? 'sel' : ''} ${over === el.id && drag !== el.id ? 'drop-before' : ''}`}
            style={{ paddingLeft: 8 + depth * 16, opacity: hidden ? 0.5 : 1 }}
            draggable={depth === 0}
            onDragStart={() => setDrag(el.id)}
            onDragOver={(e) => { e.preventDefault(); setOver(el.id); }}
            onDragEnd={() => { setDrag(null); setOver(null); }}
            onDrop={(e) => {
              e.preventDefault();
              if (drag && drag !== el.id && doc.elements[drag]?.parentId === el.parentId) editor.reorderTo(drag, el.id);
              setDrag(null);
              setOver(null);
            }}
            onClick={(e) => editor.select(e.shiftKey ? [...new Set([...state.selection, el.id])] : [el.id])}
            onDoubleClick={() => {
              const name = prompt(t('Layer name'), layerName(el));
              if (name !== null) editor.apply([{ op: 'update', id: el.id, patch: { name: name || null } }]);
            }}
          >
            <Icon size={15} className="faint" />
            <span className="name">{layerName(el)}</span>
            {el.lock?.move && <Lock size={13} className="faint" />}
            <button className="btn ghost sm icon" style={{ width: 24, height: 24 }} title={hidden ? t('Show') : t('Hide')}
              onClick={(e) => { e.stopPropagation(); editor.apply([{ op: 'update', id: el.id, patch: { visible: hidden ? null : false } }]); }}>
              {hidden ? <EyeOff size={13} /> : <Eye size={13} />}
            </button>
          </div>
        );
      })}
    </div>
  );
}
