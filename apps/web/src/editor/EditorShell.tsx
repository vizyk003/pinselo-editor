/** Editor shell (docs/plan/04-ux-spec.md §3): header, rail + object panel, contextual toolbar, canvas, pages, drawers. */
import type { HocuspocusProvider } from '@hocuspocus/provider';
import { BRAND } from '@oc/config';
import { bodyPlainText, Editor, parseEmbedUrl, sortedPages, videoSlots, type ClipboardPayload, type DesignDoc, type TextEl, type TextService } from '@oc/engine';
import {
  Accessibility, Paintbrush, ChevronLeft, Cloud, CloudOff, Download, History, Keyboard, LayoutTemplate, Layers, MessageSquare, MoreHorizontal, Play, Redo2, Scaling,
  Shapes, Share2, Type, Undo2, Upload, Copy as CopyIcon, Search, LayoutGrid, Images, BookmarkPlus, Database, Globe, PenTool, Sparkles, Palette, MonitorPlay, Clapperboard,
  FolderInput, FolderOpen, Plus, Star, Trash2, ChevronDown,
} from 'lucide-react';
import { formatDims, type Unit } from '@oc/engine';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import { api, type DesignSummary, type Folder, type Role } from '../api';
import { editorFilesApi } from '../api/editorFiles';
import { useAuth } from '../App';
import { Avatar, MenuItem, PopoverButton, useToast } from '../components/ui';
import type { Engine } from '../lib/engine';
import { Canvas } from './Canvas';
import { Ctx, useDoc, useEd, useEditorState, type Drawer, type EditorCtx, type Presence } from './context';
import { AiAvatar, AiPill, AiUndoBar, isAi } from './AiActivity';
import { ExportDialog, ResizeDialog, ShareDialog, ShortcutsDialog } from './Dialogs';
import { CommentsDrawer, LayersDrawer, useComments, VersionsDrawer } from './Drawers';
import { AccessibilityDrawer, Announcer, CanvasMirror } from './A11y';
import { blocksBareShortcut } from '../lib/appearance';
import { addText, insertEmbed, uploadAndInsert } from './insert';
import { NotesBar, PageStrip, StatusBar } from './PageStrip';
import { DrawPanel, ElementsPanel, LayersPanel, TemplatesPanel, TextPanel, UploadsPanel } from './Panels';
import { AiPanel } from './AiPanel';
import { useEditWithAi } from './EditWithAi';
import { BrandPanel } from './BrandPanel';
import { Present } from './Present';
import { Timeline } from './Timeline';
import { VideoPanel } from './VideoPanel';
import { StylesPanel } from './StylesPanel';
import { CommandPalette } from './CommandPalette';
import { NotificationsBell } from '../components/Notifications';
import { Toolbar } from './Toolbar';
import { FindReplaceDialog } from './FindReplace';
import { LibraryPanel } from './LibraryPanel';
import { EditorScheduleDialog } from '../social/EditorSchedule';
import { CalendarClock } from 'lucide-react';
import { PublishTemplateDialog } from './PublishTemplate';
import { BulkCreateDialog } from './BulkCreate';
import { PublishSiteDialog } from './PublishSite';
import { contentApi } from '../api';
import { PageGridView } from './PageGrid';
import { getViewPrefs, setViewPrefs, useViewPrefs } from './viewPrefs';
import { t, tk, tn } from '../i18n';
import { CreateDialog } from '../pages/home/CreateDialog';
import { FolderPicker } from '../components/FolderPicker';
import { ProjectsPanel } from './panels/ProjectsPanel';
import { fileActions, folderName, type IntentDialog } from './fileMenu';
import { overlayOpen, routesToCanvas } from './keyRouting';

/** Video anywhere (elements, frames, grid cells) or audio: the design gets the timeline. */
const hasMedia = (doc: DesignDoc) => Boolean(doc.meta.audio?.length) || Object.values(doc.elements).some((e) => !e.deleted && videoSlots(e).length > 0);

type PanelId = 'templates' | 'styles' | 'ai' | 'elements' | 'library' | 'text' | 'brand' | 'uploads' | 'projects' | 'video' | 'draw' | 'layers';

const PANELS: Array<{ id: PanelId; label: string; icon: typeof Shapes }> = [
  { id: 'templates', label: tk('Design'), icon: LayoutTemplate },
  { id: 'styles', label: tk('Styles'), icon: Paintbrush },
  { id: 'ai', label: tk('AI'), icon: Sparkles },
  { id: 'elements', label: tk('Elements'), icon: Shapes },
  { id: 'library', label: tk('Library'), icon: Images },
  { id: 'text', label: tk('Text'), icon: Type },
  { id: 'brand', label: tk('Brand'), icon: Palette },
  { id: 'uploads', label: tk('Uploads'), icon: Upload },
  { id: 'projects', label: tk('Projects'), icon: FolderOpen },
  { id: 'video', label: tk('Video'), icon: Clapperboard },
  { id: 'draw', label: tk('Draw'), icon: PenTool },
  { id: 'layers', label: tk('Layers'), icon: Layers },
];

/** Panels that work without edit rights: browsing projects and the layer list read the design without changing it. */
const READ_ONLY_PANELS: ReadonlySet<PanelId> = new Set(['layers', 'projects']);

const SURFACE_LABEL: Record<string, string> = {
  graphic: tk('Graphic'), presentation: tk('Presentation'), print: tk('Print'), video: tk('Video'), whiteboard: tk('Whiteboard'),
  doc: tk('Doc'), website: tk('Website'), email: tk('Email'),
};

/** Same file types as the Uploads panel. */
const UPLOAD_ACCEPT = 'image/png,image/jpeg,image/webp,image/gif,image/svg+xml,video/*,audio/*';

const CLIP_MARK = 'oc-clipboard:';

type Dialog = 'share' | 'export' | 'resize' | 'shortcuts' | 'find' | 'grid' | 'publish' | 'bulk' | 'site' | null;

export function EditorShell({ editor, engine, provider, design, intent }: {
  editor: Editor; engine: Engine; provider: HocuspocusProvider; design: DesignSummary & { role: Role };
  /** A dialog to open right away, from a one-shot URL intent (?download=1, ?publishTemplate=1; see EditorPage). */
  intent?: IntentDialog | null;
}) {
  const [params] = useSearchParams();
  const [drawer, setDrawer] = useState<Drawer>(null);
  const [dialog, setDialog] = useState<Dialog>(params.get('share') ? 'share' : intent ?? null);
  const [presenting, setPresenting] = useState(false);
  const role = design.role;
  const readOnly = role === 'view' || role === 'comment';

  // Text geometry for the editor comes from the engine's font registry: the same oc-text engine (and layout
  // cache) the canvas draws with, so measured heights and the caret match the drawn text.
  useEffect(() => {
    editor.textService = engine.fonts.service;
    // Brand fonts uploaded to the design's workspace (collaborators from other workspaces need them too).
    void contentApi.designFonts(design.id).then((r) => {
      for (const f of r.families) if (!engine.fonts.has(f.family)) engine.fonts.addFamily(f);
      editor.remeasureText();
    }).catch(() => {});
    // Faces arriving late change metrics: re-measure auto-height text. Measuring alone (text that is not drawn)
    // also reports missing faces: load them after each change.
    const off = engine.fonts.onLoad(() => editor.remeasureText());
    const offE = editor.subscribe(() => engine.fonts.pump());
    return () => {
      off();
      offE();
    };
  }, [editor, engine, design.id]);

  const ctx: EditorCtx = useMemo(() => ({
    editor, engine, provider, design, role, readOnly, drawer, setDrawer,
    openDialog: setDialog, setPresenting,
  }), [editor, engine, provider, design, role, readOnly, drawer]);

  return (
    <Ctx.Provider value={ctx}>
      <ShellInner presenting={presenting} setPresenting={setPresenting} dialog={dialog} setDialog={setDialog} />
    </Ctx.Provider>
  );
}

function ShellInner({ presenting, setPresenting, dialog, setDialog }: {
  presenting: boolean; setPresenting: (b: boolean) => void;
  dialog: Dialog; setDialog: (d: Dialog) => void;
}) {
  const { editor, provider, design, readOnly, drawer, setDrawer } = useEd();
  const toast = useToast();
  // Phones start with the canvas visible (panels are bottom sheets there).
  const [panel, setPanel] = useState<PanelId | null>(readOnly || matchMedia('(max-width: 720px)').matches ? null : 'templates');
  const [presence, setPresence] = useState<Presence[]>([]);
  const [status, setStatus] = useState<'connected' | 'connecting' | 'disconnected'>('connected');
  const [unsynced, setUnsynced] = useState(0);
  const [notesOpen, setNotesOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [stripOpen, setStripOpen] = useState(true);
  const [timelineOpen, setTimelineOpenRaw] = useState(() => hasMedia(editor.doc));
  const setTimelineOpen = useCallback((open: boolean) => {
    setTimelineOpenRaw(open);
    if (open) editor.seek(editor.state.playhead ?? editor.pageStart(editor.state.activePageId));
    else editor.seek(null);
  }, [editor]);
  useEffect(() => {
    if (timelineOpen && editor.state.playhead === null) editor.seek(editor.pageStart(editor.state.activePageId));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Open the timeline automatically when the first video or audio arrives — once, so closing it sticks. The flag is
  // set before opening: opening seeks, which notifies subscribers synchronously (re-entering here).
  const hadMedia = useRef(timelineOpen);
  useEffect(() => editor.subscribe(() => {
    const has = hasMedia(editor.doc);
    if (has === hadMedia.current) return;
    hadMedia.current = has;
    if (has) setTimelineOpen(true);
  }), [editor, setTimelineOpen]);
  const { comments, reload: reloadComments } = useComments(design.id, provider.awareness);
  // A link to one page (?page=N, "Copy link to page", g97) opens the design there.
  const [params] = useSearchParams();
  useEffect(() => {
    const n = Number(params.get('page'));
    const p = Number.isInteger(n) && n > 0 ? sortedPages(editor.doc)[n - 1] : undefined;
    if (p) requestAnimationFrame(() => editor.setActivePage(p.id, true));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const state = useEditorState();

  // ---------------------------------------------------------------- presence (awareness)
  useEffect(() => {
    const aw = provider.awareness;
    if (!aw) return;
    const update = () => {
      const list: Presence[] = [];
      aw.getStates().forEach((s, clientId) => {
        if (clientId === aw.clientID || !s.user) return;
        list.push({ clientId, user: s.user, cursor: s.cursor, selection: s.selection, pageId: s.pageId });
      });
      setPresence(list);
    };
    aw.on('change', update);
    update();
    const onStatus = ({ status: st }: { status: 'connected' | 'connecting' | 'disconnected' }) => setStatus(st);
    // The provider emits this from its yjs update handler, before it sends the update: a throw here (React refuses
    // state updates once a burst of input exceeds its nested-update limit) would drop that update and stall every
    // later one (SYNC-01). So only the saved/unsaved flag is kept, set from a timer (one per burst), never throwing.
    let flagTimer: ReturnType<typeof setTimeout> | undefined;
    const onUnsynced = () => {
      if (flagTimer !== undefined) return;
      flagTimer = setTimeout(() => {
        flagTimer = undefined;
        try {
          setUnsynced(provider.hasUnsyncedChanges ? 1 : 0);
        } catch (e) {
          console.error(e);
          setTimeout(onUnsynced, 50);
        }
      }, 0);
    };
    provider.on('status', onStatus);
    provider.on('unsyncedChanges', onUnsynced);
    return () => {
      clearTimeout(flagTimer);
      aw.off('change', update);
      provider.off('status', onStatus);
      provider.off('unsyncedChanges', onUnsynced);
    };
  }, [provider]);

  // DATA-01: warn before leaving while edits have not reached the server (offline, or still sending).
  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (readOnly || !provider.hasUnsyncedChanges) return;
      e.preventDefault();
      e.returnValue = '';
    };
    addEventListener('beforeunload', onBeforeUnload);
    return () => removeEventListener('beforeunload', onBeforeUnload);
  }, [provider, readOnly]);

  useEffect(() => {
    provider.awareness?.setLocalStateField('selection', state.selection);
    provider.awareness?.setLocalStateField('pageId', state.activePageId);
  }, [provider, state.selection, state.activePageId]);

  // ---------------------------------------------------------------- clipboard (system clipboard, HTML fallback)
  useEffect(() => {
    const inField = (tgt: EventTarget | null) => tgt instanceof HTMLInputElement || (tgt instanceof HTMLTextAreaElement && !tgt.classList.contains('ime-proxy'));
    const onCopy = (e: ClipboardEvent, cut = false) => {
      if (inField(e.target)) return;
      if (editor.state.editingText) {
        const text = editor.selectedText();
        e.clipboardData?.setData('text/plain', text);
        e.preventDefault();
        if (cut && text) editor.deleteText(-1);
        return;
      }
      const payload = cut && !readOnly ? editor.cut() : editor.copy();
      if (!payload) return;
      const json = JSON.stringify(payload);
      e.clipboardData?.setData('text/html', `<meta charset="utf-8"><!--${CLIP_MARK}${btoa(unescape(encodeURIComponent(json)))}--><span>${BRAND.name} design elements</span>`);
      e.clipboardData?.setData('text/plain', `${BRAND.name} elements`);
      e.preventDefault();
    };
    const onPaste = async (e: ClipboardEvent) => {
      if (inField(e.target) || readOnly) return;
      const dt = e.clipboardData;
      if (!dt) return;
      if (editor.state.editingText) {
        e.preventDefault();
        editor.typeText(dt.getData('text/plain').replace(/\r\n?/g, '\n'));
        return;
      }
      const html = dt.getData('text/html');
      const m = html && new RegExp(`<!--${CLIP_MARK}([A-Za-z0-9+/=]+)-->`).exec(html);
      if (m) {
        e.preventDefault();
        const payload = JSON.parse(decodeURIComponent(escape(atob(m[1]!)))) as ClipboardPayload;
        editor.paste(payload);
        return;
      }
      const files = [...dt.files].filter((f) => f.type.startsWith('image/'));
      if (files.length) {
        e.preventDefault();
        for (const f of files) await uploadAndInsert(editor, f).catch((x) => toast(t('Paste failed: {message}', { message: (x as Error).message })));
        return;
      }
      // Windows and old Mac line ends (TXT-04): the edit-mode path normalises them too.
      const text = dt.getData('text/plain').replace(/\r\n?/g, '\n');
      if (text.trim()) {
        e.preventDefault();
        // A YouTube/Spotify/Maps/… link becomes an embed (A11).
        if (/^https?:\/\/\S+$/.test(text.trim()) && parseEmbedUrl(text.trim())) {
          try {
            if (await insertEmbed(editor, text.trim())) return;
          } catch (x) {
            toast((x as Error).message);
            return;
          }
        }
        addText(editor, text.trim().slice(0, 5000), { size: 28 });
      }
    };
    const copy = (e: ClipboardEvent) => onCopy(e);
    const cut = (e: ClipboardEvent) => onCopy(e, true);
    const paste = (e: ClipboardEvent) => void onPaste(e);
    document.addEventListener('copy', copy);
    document.addEventListener('cut', cut);
    document.addEventListener('paste', paste);
    const key = (e: KeyboardEvent) => {
      if (inField(e.target) || editor.state.editingText) return;
      // Canvas shortcuts (undo, ⌘A, ⌘G, ⌘[, Delete, arrows, …) keep working when focus is on a toolbar button,
      // a popover or nowhere, not only on the canvas, which handles its own keys (K-01).
      if (!readOnly && !editor.state.presenting && routesToCanvas(e) && !overlayOpen(document) && !blocksBareShortcut(e) && editor.handleKey(e.key, { shift: e.shiftKey, alt: e.altKey, meta: e.metaKey || e.ctrlKey, ctrl: e.ctrlKey }, e.code)) {
        e.preventDefault();
        return;
      }
      if (e.key === '?' && !e.metaKey) setDialog('shortcuts');
      else if (e.key === '/' && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        setPaletteOpen(true);
      } else if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k' && editor.state.selection.length === 1 && !readOnly) {
        // ⌘K: link the selected element (Canva).
        e.preventDefault();
        const cur = editor.selected()[0]?.link;
        const url = prompt(t('Link to (URL)'), cur?.kind === 'url' ? cur.url : 'https://');
        if (url !== null) editor.updateSelected({ link: url.trim() && url.trim() !== 'https://' ? { kind: 'url', url: /^https?:\/\//.test(url.trim()) ? url.trim() : `https://${url.trim()}` } : null }, false, false);
      }
      else if (e.code === 'Space' && editor.state.playhead !== null && !e.metaKey) {
        e.preventDefault();
        editor.togglePlay();
      } else if (e.key.toLowerCase() === 's' && !e.metaKey && !e.ctrlKey && editor.state.playhead !== null && !readOnly) editor.splitAtPlayhead();
      else if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        setDialog('find');
      } else if (e.shiftKey && !e.metaKey && !e.ctrlKey && e.code === 'KeyR') {
        e.preventDefault();
        setViewPrefs({ rulers: !getViewPrefs().rulers });
      } else if ((e.metaKey || e.ctrlKey) && e.altKey && e.code === 'KeyP') {
        e.preventDefault();
        setPresenting(true);
      }
    };
    window.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('copy', copy);
      document.removeEventListener('cut', cut);
      document.removeEventListener('paste', paste);
      window.removeEventListener('keydown', key);
    };
  }, [editor, readOnly, setDialog, setPresenting, toast]);

  const commentPins = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of comments) if (!c.resolved && !c.parent_id && c.element_id) m.set(c.element_id, (m.get(c.element_id) ?? 0) + 1);
    return m;
  }, [comments]);

  const togglePanel = useCallback((id: PanelId) => setPanel((p) => (p === id ? null : id)), []);

  // File ▸ Upload files (g88): the Uploads panel opens alongside the file picker; each file is uploaded and placed like
  // a pick in that panel, which then remounts to list the new uploads.
  const uploadInput = useRef<HTMLInputElement>(null);
  const [uploadsKey, setUploadsKey] = useState(0);
  const pickUploads = useCallback(() => {
    setPanel('uploads');
    uploadInput.current?.click();
  }, []);
  const uploadFiles = async (files: File[]) => {
    if (!files.length) return;
    toast(tn(files.length, 'Uploading {n} file…', 'Uploading {n} files…'));
    for (const f of files) {
      try {
        await uploadAndInsert(editor, f);
      } catch (e) {
        toast(`${f.name}: ${(e as Error).message}`);
      }
    }
    setUploadsKey((k) => k + 1);
  };

  return (
    <div className="editor">
      <Header status={status} unsynced={unsynced} presence={presence} openComments={() => setDrawer(drawer === 'comments' ? null : 'comments')}
        commentCount={comments.filter((c) => !c.resolved && !c.parent_id).length} setDialog={setDialog} setPresenting={setPresenting} onUpload={pickUploads} />
      <AiUndoBar aiActive={presence.some((p) => isAi(p.user))} />
      <input ref={uploadInput} type="file" accept={UPLOAD_ACCEPT} multiple hidden aria-hidden tabIndex={-1}
        onChange={(e) => { const files = [...(e.target.files ?? [])]; e.target.value = ''; void uploadFiles(files); }} />
      <div className="editor-body">
        <Rail panel={panel} togglePanel={togglePanel} readOnly={readOnly} />
        {panel ? (
          <section className="panel" aria-label={t(PANELS.find((p) => p.id === panel)?.label ?? '')}>
            <div className="panel-head"><h3>{PANELS.find((p) => p.id === panel)?.label === 'Design' ? t('Templates') : t(PANELS.find((p) => p.id === panel)?.label ?? '')}</h3>
              <button className="btn ghost sm icon" onClick={() => setPanel(null)} aria-label={t('Collapse panel')}><ChevronLeft size={16} /></button>
            </div>
            <div className="panel-body">
              {panel === 'templates' && <TemplatesPanel />}
              {panel === 'styles' && <StylesPanel />}
              {panel === 'elements' && <ElementsPanel />}
              {panel === 'library' && <LibraryPanel />}
              {panel === 'text' && <TextPanel />}
              {panel === 'uploads' && <UploadsPanel key={uploadsKey} />}
              {panel === 'projects' && <ProjectsPanel />}
              {panel === 'draw' && <DrawPanel />}
              {panel === 'ai' && <AiPanel />}
              {panel === 'video' && <VideoPanel />}
              {panel === 'brand' && <BrandPanel />}
              {panel === 'layers' && <LayersPanel />}
            </div>
          </section>
        ) : <div />}
        <div className="stage">
          {/* A fixed-height slot: a toolbar that wraps to more rows overlays the canvas instead of pushing it down. */}
          <div className="ctx-slot"><Toolbar /></div>
          <CanvasMirror />
          <Canvas presence={presence} commentPins={commentPins} />
          <Announcer />
          <div>
            {notesOpen && <NotesBar />}
            {timelineOpen ? <Timeline onClose={() => setTimelineOpen(false)} /> : stripOpen && <PageStrip />}
            <StatusBar notesOpen={notesOpen} setNotesOpen={setNotesOpen} stripOpen={stripOpen} setStripOpen={setStripOpen} timelineOpen={timelineOpen} setTimelineOpen={setTimelineOpen} />
          </div>
          {drawer === 'comments' && <CommentsDrawer comments={comments} reload={reloadComments} onClose={() => setDrawer(null)} />}
          {drawer === 'versions' && <VersionsDrawer onClose={() => setDrawer(null)} />}
          {drawer === 'layers' && <LayersDrawer onClose={() => setDrawer(null)} />}
          {drawer === 'a11y' && <AccessibilityDrawer onClose={() => setDrawer(null)} />}
        </div>
      </div>
      {dialog === 'share' && <ShareDialog onClose={() => setDialog(null)} />}
      {dialog === 'export' && <ExportDialog onClose={() => setDialog(null)} />}
      {dialog === 'resize' && <ResizeDialog onClose={() => setDialog(null)} />}
      {dialog === 'shortcuts' && <ShortcutsDialog onClose={() => setDialog(null)} />}
      {dialog === 'find' && <FindReplaceDialog onClose={() => setDialog(null)} />}
      {dialog === 'grid' && <PageGridView onClose={() => setDialog(null)} />}
      {dialog === 'publish' && <PublishTemplateDialog onClose={() => setDialog(null)} />}
      {dialog === 'bulk' && <BulkCreateDialog onClose={() => setDialog(null)} />}
      {dialog === 'site' && <PublishSiteDialog onClose={() => setDialog(null)} />}
      {presenting && <Present onExit={() => setPresenting(false)} />}
      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} actions={{
        openDialog: setDialog, openPanel: (p) => setPanel(p as PanelId), openDrawer: setDrawer, present: () => setPresenting(true),
      }} />}
    </div>
  );
}

/**
 * The panel rail. When the column is too short for every panel (a laptop screen), the ones that don't fit move
 * into a More menu at the bottom instead of being cut off. On phones the rail is a scrolling bottom bar.
 */
function Rail({ panel, togglePanel, readOnly }: { panel: PanelId | null; togglePanel: (id: PanelId) => void; readOnly: boolean }) {
  const ref = useRef<HTMLElement>(null);
  const [fit, setFit] = useState(PANELS.length);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const cs = getComputedStyle(el);
      if (cs.flexDirection === 'row') return setFit(PANELS.length);
      const item = el.querySelector('button');
      const gap = parseFloat(cs.rowGap) || 0;
      const step = (item?.offsetHeight ?? 64) + gap;
      const room = el.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom) + gap;
      const n = Math.floor(room / step);
      setFit(n >= PANELS.length ? PANELS.length : Math.max(1, n - 1));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const shown = PANELS.slice(0, fit);
  const extra = PANELS.slice(fit);
  return (
    <nav className="rail" aria-label={t('Panels')} ref={ref}>
      {shown.map(({ id, label, icon: Icon }) => (
        <button key={id} className={panel === id ? 'active' : ''} onClick={() => togglePanel(id)} disabled={readOnly && !READ_ONLY_PANELS.has(id)} aria-pressed={panel === id}>
          <span className="ic"><Icon size={20} /></span>
          {t(label)}
        </button>
      ))}
      {extra.length > 0 && (
        <PopoverButton menu placement="right" className={extra.some((p) => p.id === panel) ? 'active' : ''} title={t('More panels')}
          label={<><span className="ic"><MoreHorizontal size={20} /></span>{t('More')}</>}
          content={(close) => (
            <div className="menu" role="menu">
              {extra.map(({ id, label, icon: Icon }) => (
                <MenuItem key={id} icon={<Icon size={16} />} label={t(label)} checked={panel === id} disabled={readOnly && !READ_ONLY_PANELS.has(id)}
                  onClick={() => { close(); togglePanel(id); }} />
              ))}
            </div>
          )} />
      )}
    </nav>
  );
}

function Header({ status, unsynced, presence, openComments, commentCount, setDialog, setPresenting, onUpload }: {
  status: string; unsynced: number; presence: Presence[]; openComments: () => void; commentCount: number;
  setDialog: (d: Dialog) => void; setPresenting: (b: boolean) => void;
  /** File ▸ Upload files: opens the Uploads panel and the file picker. */
  onUpload: () => void;
}) {
  const ai = useEditWithAi();
  const { editor, design, role, readOnly, setDrawer } = useEd();
  const doc = useDoc();
  useEditorState();
  const nav = useNavigate();
  const { user } = useAuth();
  const toast = useToast();
  const [title, setTitle] = useState(doc.meta.title);
  const [scheduling, setScheduling] = useState(false);
  // File menu (g88): starred and folder change from here, so they are tracked locally after the design loaded.
  const [starred, setStarred] = useState(design.starred);
  const [folderId, setFolderId] = useState(design.folder_id);
  const [folders, setFolders] = useState<Folder[] | null>(null);
  const [creating, setCreating] = useState(false);
  const [moving, setMoving] = useState(false);
  const can = fileActions(design, role, user?.id);
  const loadFolders = useCallback(() => editorFilesApi.folders().then((f) => { setFolders(f); return f; }).catch(() => null), []);
  const toggleStar = async () => {
    const next = !starred;
    setStarred(next);
    try {
      await editorFilesApi.setStarred(design.id, next);
      toast(next ? t('Added to Starred') : t('Removed from Starred'));
    } catch {
      setStarred(!next);
      toast(t('Couldn’t update Starred.'));
    }
  };
  const moveTo = async (next: string | null) => {
    setMoving(false);
    if (next === folderId) return;
    if (next === null && !can.unfile) return toast(t('Only the owner can take this design out of its folder.'));
    try {
      await editorFilesApi.moveTo(design.id, next);
      setFolderId(next);
      const name = next ? folderName(await loadFolders(), next) : null;
      toast(next ? (name ? t('Moved to {folder}', { folder: name }) : t('Moved to the folder')) : t('Removed from folder'));
    } catch {
      toast(t('Couldn’t move the design.'));
    }
  };
  const trash = async () => {
    if (!confirm(t('Move “{title}” to the trash? You can restore it from the trash for 30 days.', { title: doc.meta.title }))) return;
    try {
      await editorFilesApi.trash(design.id);
      toast(t('Moved to trash'));
      nav('/');
    } catch {
      toast(t('Couldn’t move the design to the trash.'));
    }
  };
  useEffect(() => setTitle(doc.meta.title), [doc.meta.title]);
  const saveTitle = async () => {
    const next = title.trim();
    if (!next || next === doc.meta.title) return setTitle(doc.meta.title);
    editor.apply([{ op: 'meta', patch: { title: next } }]);
    await api.updateDesign(design.id, { title: next }).catch(() => {});
  };
  const people = useMemo(() => {
    const seen = new Map<string, Presence['user']>();
    for (const p of presence) seen.set(p.user.id, p.user);
    return [...seen.values()].filter((u) => u.id !== user?.id);
  }, [presence, user]);
  const canUndo = editor.store.undo.undoStack.length > 0;
  const canRedo = editor.store.undo.redoStack.length > 0;
  return (
    <header className="editor-header">
      <button className="btn ghost sm" onClick={() => nav('/')} aria-label={t('Home')}><ChevronLeft size={16} /><span className="hide-mobile">{t('Home')}</span></button>
      <PopoverButton className="btn ghost sm" title={t('File')} label={<>{t('File')} <ChevronDown size={14} className="hide-mobile" /></>} menu content={(close) => (
        <div className="menu" role="menu">
          <FileInfo folderId={folderId} folders={folders} loadFolders={loadFolders} />
          <div className="hsep" />
          <MenuItem icon={<Plus size={16} />} label={t('Create new design')} onClick={() => { close(); setCreating(true); }} />
          {!readOnly && <MenuItem icon={<Upload size={16} />} label={t('Upload files')} onClick={() => { close(); onUpload(); }} />}
          <div className="hsep" />
          <MenuItem icon={<CopyIcon size={16} />} label={t('Make a copy')} onClick={async () => { close(); const r = await api.copyDesign(design.id); toast(t('Copy created')); nav(`/design/${r.id}`); }} />
          <MenuItem icon={<Star size={16} fill={starred ? 'currentColor' : 'none'} />} label={starred ? t('Unstar') : t('Star')} onClick={() => { close(); void toggleStar(); }} />
          {can.move && <MenuItem icon={<FolderInput size={16} />} label={t('Move to folder')} onClick={() => { close(); setMoving(true); }} />}
          {!readOnly && <MenuItem icon={<Scaling size={16} />} label={t('Resize')} onClick={() => { close(); setDialog('resize'); }} />}
          <MenuItem icon={<History size={16} />} label={t('Version history')} onClick={() => { close(); setDrawer('versions'); }} />
          <MenuItem icon={<Layers size={16} />} label={t('Layers')} onClick={() => { close(); setDrawer('layers'); }} />
          <MenuItem icon={<Accessibility size={16} />} label={t('Check accessibility')} onClick={() => { close(); setDrawer('a11y'); }} />
          <MenuItem icon={<Download size={16} />} label={t('Download')} onClick={() => { close(); setDialog('export'); }} />
          <MenuItem icon={<CalendarClock size={16} />} label={t('Schedule to social')} onClick={() => { close(); setScheduling(true); }} />
          {!readOnly && <MenuItem icon={<BookmarkPlus size={16} />} label={t('Publish as template')} onClick={() => { close(); setDialog('publish'); }} />}
          {!readOnly && <MenuItem icon={<Database size={16} />} label={t('Bulk create')} onClick={() => { close(); setDialog('bulk'); }} />}
          {!readOnly && <MenuItem icon={<Globe size={16} />} label={t('Publish as website')} onClick={() => { close(); setDialog('site'); }} />}
          <MenuItem icon={<Search size={16} />} label={t('Find and replace')} kbd="⌘F" onClick={() => { close(); setDialog('find'); }} />
          <MenuItem icon={<LayoutGrid size={16} />} label={t('Grid view')} onClick={() => { close(); setDialog('grid'); }} />
          <div className="hsep" />
          <ViewToggles />
          <div className="hsep" />
          <MenuItem icon={<Keyboard size={16} />} label={t('Keyboard shortcuts')} kbd="?" onClick={() => { close(); setDialog('shortcuts'); }} />
          {can.trash && (
            <>
              <div className="hsep" />
              <MenuItem icon={<Trash2 size={16} />} label={t('Move to trash')} danger onClick={() => { close(); void trash(); }} />
            </>
          )}
        </div>
      )} />
      {creating && <CreateDialog onClose={() => setCreating(false)} />}
      {moving && <FolderPicker value={folderId} title={t('Move to folder')} onClose={() => setMoving(false)} onPick={(f) => void moveTo(f)} />}
      {!readOnly && (
        <>
          <button className="btn ghost sm icon" title={t('Undo') + ' (⌘Z)'} onClick={() => editor.undo()} disabled={!canUndo}><Undo2 size={17} /></button>
          <button className="btn ghost sm icon" title={t('Redo') + ' (⇧⌘Z)'} onClick={() => editor.redo()} disabled={!canRedo}><Redo2 size={17} /></button>
        </>
      )}
      <span className="status hide-mobile" title={status === 'connected' ? t('All changes saved') : t('Offline — changes are kept on this device and sync when you reconnect')}>
        {status === 'connected' ? <Cloud size={15} /> : <CloudOff size={15} />}
        {status === 'connected' ? (unsynced ? t('Saving…') : t('Saved')) : t('Offline')}
      </span>
      <AiPill presence={presence} />
      <div className="grow" />
      <input className="title-input" value={title} disabled={readOnly} onChange={(e) => setTitle(e.target.value)} onBlur={saveTitle}
        onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()} aria-label={t('Design title')} />
      <div className="grow" />
      <div className="row" style={{ gap: 0 }}>
        {people.slice(0, 5).map((u) => <span key={u.id} style={{ marginLeft: -6 }}>{isAi(u) ? <AiAvatar user={u} /> : <Avatar name={u.name} color={u.color} />}</span>)}
        {people.length > 5 && <span className="avatar" style={{ background: '#475569', marginLeft: -6 }}>+{people.length - 5}</span>}
      </div>
      <NotificationsBell variant="header" />
      <button className="btn ghost sm icon hide-mobile header-count" onClick={openComments} title={t('Comments')} aria-label={t('Comments')}><MessageSquare size={16} />{commentCount > 0 && <span className="badge">{commentCount}</span>}</button>
      <button className="btn ghost sm hide-mobile" onClick={() => setPresenting(true)} title={t('Present') + ' (⌥⌘P)'}><Play size={16} /> {t('Present')}</button>
      <button className="btn ghost sm icon hide-mobile" title={t('Presenter view (notes, timer, next slide) in a new window')} onClick={() => window.open(`/design/${design.id}/presenter`, 'oc-presenter', 'width=1100,height=700')}><MonitorPlay size={16} /></button>
      <button className="btn ghost sm hide-mobile" onClick={() => setDialog('export')} title={t('Download')} aria-haspopup="dialog" data-download-anchor><Download size={16} /> {t('Download')}</button>
      <button className="btn ghost sm hide-mobile" onClick={() => setScheduling(true)} title={t('Schedule to social')}><CalendarClock size={16} /> {t('Schedule')}</button>
      {doc.meta.surface === 'website' && !readOnly && <button className="btn ghost sm hide-mobile" onClick={() => setDialog('site')}><Globe size={16} /> {t('Publish')}</button>}
      {/* Phones: the actions above live in one overflow menu, so the header fits without scrolling sideways. */}
      <PopoverButton menu className="btn ghost sm icon show-mobile" title={t('More actions')} label={<MoreHorizontal size={16} />} content={(close) => (
        <div className="menu" role="menu">
          <MenuItem icon={<MessageSquare size={16} />} label={commentCount > 0 ? `${t('Comments')} (${commentCount})` : t('Comments')} onClick={() => { close(); openComments(); }} />
          <MenuItem icon={<Play size={16} />} label={t('Present')} onClick={() => { close(); setPresenting(true); }} />
          <MenuItem icon={<Download size={16} />} label={t('Download')} onClick={() => { close(); setDialog('export'); }} />
          <MenuItem icon={<CalendarClock size={16} />} label={t('Schedule to social')} onClick={() => { close(); setScheduling(true); }} />
          {doc.meta.surface === 'website' && !readOnly && <MenuItem icon={<Globe size={16} />} label={t('Publish')} onClick={() => { close(); setDialog('site'); }} />}
        </div>
      )} />
      <button className="btn ghost sm" onClick={ai.run} title={ai.connected ? t('Copy a prompt for your AI that points it at this design') : t('Connect Claude, ChatGPT or Codex')} aria-haspopup={ai.connected ? undefined : 'dialog'}>
        <Sparkles size={16} aria-hidden /><span className="hide-narrow"> {t('Edit with AI')}</span>
      </button>
      {ai.element}
      <button className="btn sm white" onClick={() => setDialog('share')} aria-label={t('Share')}><Share2 size={16} /><span className="hide-narrow"> {t('Share')}</span></button>
      {scheduling && <EditorScheduleDialog onClose={() => setScheduling(false)} />}
    </header>
  );
}

/** The File menu's info line (g88): what the design is, who made it and where it's filed. */
function FileInfo({ folderId, folders, loadFolders }: { folderId: string | null; folders: Folder[] | null; loadFolders: () => Promise<unknown> }) {
  const { design } = useEd();
  const doc = useDoc();
  // Folder names are fetched when the menu opens, so a rename elsewhere shows up.
  useEffect(() => {
    if (folderId) void loadFolders();
  }, [folderId, loadFolders]);
  const folder = folderName(folders, folderId);
  const { w, h, unit } = doc.meta.dims;
  const kind = SURFACE_LABEL[doc.meta.surface];
  return (
    <div style={{ padding: '4px 10px 2px', fontSize: 12, lineHeight: 1.5, maxWidth: 300 }}>
      <div style={{ fontWeight: 600, fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={doc.meta.title}>{doc.meta.title}</div>
      <div className="faint">
        {[kind ? t(kind) : doc.meta.surface, formatDims(w, h, unit as Unit), tn(Object.keys(doc.pages).length, '{n} page', '{n} pages')].join(' · ')}
      </div>
      <div className="faint" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {t('By {name}', { name: design.owner_name })}{folder ? ` · ${t('In {folder}', { folder })}` : ''}
      </div>
    </div>
  );
}

function ViewToggles() {
  const p = useViewPrefs();
  const row = (on: boolean, label: string, kbd: string | undefined, fn: () => void) => (
    <MenuItem checked={on} label={label} kbd={kbd} onClick={fn} />
  );
  return (
    <>
      {row(p.rulers, t('Show rulers and guides'), '⇧R', () => setViewPrefs({ rulers: !p.rulers }))}
      {row(p.margins, t('Show margins'), undefined, () => setViewPrefs({ margins: !p.margins }))}
      {row(p.bleed, t('Show print bleed'), undefined, () => setViewPrefs({ bleed: !p.bleed }))}
    </>
  );
}
