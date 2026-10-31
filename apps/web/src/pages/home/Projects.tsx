/**
 * Projects hub (plan 11 g17/g20): nested folders with breadcrumbs, a Recents carousel, search within Projects, type /
 * owner filters and sort, a grid/list toggle (remembered per user), '+ Add new' (folder, design, upload) and bulk
 * select (checkboxes, Shift-click ranges, select all) with bulk Star / Move / Trash. `DesignList` gives Starred,
 * Shared, Trash and team pages the same controls.
 */
import {
  ChevronLeft, ChevronRight, ExternalLink, Folder, FolderInput, FolderPlus, MoreHorizontal, Pencil, Plus, RotateCcw, Search, Star, Trash2, Upload, Users, X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router';
import { api, ApiError, brandHub, type DesignSummary, type Folder as FolderT } from '../../api';
import { projectsApi, type DesignSort, type ProjectFolder } from '../../api/projects';
import { useAuth } from '../../App';
import { FolderPickerDialog } from '../../components/FolderPicker';
import { FilterChip, OWNER_OPTIONS } from '../../components/UnifiedSearch';
import { EmptyState, MenuItem, Modal, PageHeader, PopoverButton, SearchField, timeAgo, useToast, ViewToggle } from '../../components/ui';
import { isRtl, t, tk, tn } from '../../i18n';
import { useWorkspaces } from '../Team';
import { CreateDialog } from './CreateDialog';
import { DesignGrid, ListHead, type Selection } from './Designs';
import {
  canMoveFolderTo, childFolders, DEFAULT_LIST, designKey, filterFolders, folderKey, folderPath, listStateFromParams, listStateToParams, loadViewMode,
  saveViewMode, sortFolders, splitSelection, subtreeIds, toggleSelection, type ListState, type ViewMode,
} from './projectsModel';
import { MEDIA_ACCEPT, IMPORT_ACCEPT, useStartFromFile } from './StartFromUpload';
import './projects.css';

const TYPE_OPTIONS: Array<{ value: string; label: string }> = [
  { value: '', label: tk('All types') },
  { value: 'folder', label: tk('Folders') },
  { value: 'graphic', label: tk('Graphics') },
  { value: 'presentation', label: tk('Presentations') },
  { value: 'video', label: tk('Videos') },
  { value: 'doc', label: tk('Docs') },
  { value: 'whiteboard', label: tk('Whiteboards') },
  { value: 'sheet', label: tk('Sheets') },
  { value: 'website', label: tk('Websites') },
  { value: 'email', label: tk('Emails') },
  { value: 'print', label: tk('Print') },
];
const SORT_OPTIONS: Array<{ value: DesignSort; label: string }> = [
  { value: 'edited', label: tk('Last edited') },
  { value: 'oldest', label: tk('Oldest') },
  { value: 'az', label: tk('Name A–Z') },
  { value: 'za', label: tk('Name Z–A') },
];
const RECENTS = 12;

const isProjectFolder = (f: FolderT): f is ProjectFolder => 'can_manage' in f;
const errText = (e: unknown) => (e instanceof ApiError && e.message !== e.code ? e.message : (e as Error).message || t('Failed'));

// ------------------------------------------------------------------ shared list state

/** Filters and sort in the URL (?q=&type=&owner=&sort=), so a filtered view can be linked and survives reloads. */
function useListState(): [ListState, (patch: Partial<ListState>) => void] {
  const [params, setParams] = useSearchParams();
  const list = useMemo(() => listStateFromParams(params), [params]);
  const set = useCallback((patch: Partial<ListState>) => setParams((p) => listStateToParams({ ...listStateFromParams(p), ...patch }, p), { replace: true }), [setParams]);
  return [list, set];
}

function useViewMode(): [ViewMode, (v: ViewMode) => void] {
  const uid = useAuth().user?.id;
  const [view, setView] = useState<ViewMode>(() => loadViewMode(uid));
  return [view, (v) => { setView(v); saveViewMode(uid, v); }];
}

/** Checkbox selection over the items in display order; cleared when `resetKey` changes (another folder or filter). */
function useSelection(order: string[], resetKey: string) {
  const [sel, setSel] = useState<Set<string>>(() => new Set());
  const anchor = useRef<string | null>(null);
  useEffect(() => {
    setSel(new Set());
    anchor.current = null;
  }, [resetKey]);
  // Items that left the list (moved, trashed) leave the selection too.
  useEffect(() => setSel((s) => {
    const keep = new Set([...s].filter((k) => order.includes(k)));
    return keep.size === s.size ? s : keep;
  }), [order]);
  const selection: Selection = useMemo(() => ({
    active: sel.size > 0,
    has: (k) => sel.has(k),
    toggle: (k, shift) => {
      setSel((s) => toggleSelection(order, s, k, { shift, anchor: anchor.current }));
      if (!shift) anchor.current = k;
    },
  }), [sel, order]);
  const clear = useCallback(() => setSel(new Set()), []);
  const all = useCallback(() => setSel(new Set(order)), [order]);
  // Escape clears the selection; Ctrl/Cmd+A selects everything shown (outside text fields and dialogs). Selection
  // checkboxes count as outside: keyboard users tick items with Space and then use the shortcuts from there.
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.target instanceof Element && e.target.closest('input:not([type=checkbox]):not([type=radio]), textarea, select, [contenteditable], .modal, .popover')) return;
      if (e.key === 'Escape' && sel.size) clear();
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'a' && order.length) {
        e.preventDefault();
        all();
      }
    };
    window.addEventListener('keydown', k);
    return () => window.removeEventListener('keydown', k);
  }, [sel.size, order.length, clear, all]);
  return { sel, selection, clear, all };
}

function ListControls({ list, setList, view, setView, owner = true, folderType = true, total, selected, onSelectAll }: {
  list: ListState; setList: (p: Partial<ListState>) => void; view: ViewMode; setView: (v: ViewMode) => void;
  owner?: boolean; folderType?: boolean; total: number; selected: number; onSelectAll: (on: boolean) => void;
}) {
  return (
    <div className="pj-toolbar" role="group" aria-label={t('Filters')}>
      <FilterChip label={t('Type')} value={list.type} defaultValue="" options={folderType ? TYPE_OPTIONS : TYPE_OPTIONS.filter((o) => o.value !== 'folder')} onChange={(v) => setList({ type: v })} />
      {owner && <FilterChip label={t('Owner')} value={list.owner} defaultValue="anyone" options={OWNER_OPTIONS} onChange={(v) => setList({ owner: v })} />}
      <FilterChip label={t('Order')} value={list.sort} defaultValue="edited" options={SORT_OPTIONS} onChange={(v) => setList({ sort: v })} />
      <div className="grow" />
      {total > 0 && (
        <label className="row faint pj-count">
          <input type="checkbox" className="pj-check" style={{ opacity: 1 }} checked={selected > 0 && selected === total}
            ref={(el) => { if (el) el.indeterminate = selected > 0 && selected < total; }} onChange={(e) => onSelectAll(e.target.checked)} />
          {t('Select all')}
        </label>
      )}
      <ViewToggle mode={view} onChange={setView} />
    </div>
  );
}

/** Search box that writes `q` to the URL a moment after typing stops. */
function SearchBox({ value, onChange }: { value: string; onChange: (q: string) => void }) {
  const [q, setQ] = useState(value);
  // What this box last wrote to the URL: when that lands (a transition, possibly after more typing) it must not
  // overwrite the box. Any other change of `value` (back/forward, a link) replaces what's typed.
  const sent = useRef(value);
  useEffect(() => {
    if (value === sent.current) return;
    sent.current = value;
    setQ(value);
  }, [value]);
  useEffect(() => {
    if (q === value) return;
    const h = setTimeout(() => {
      sent.current = q;
      onChange(q);
    }, 250);
    return () => clearTimeout(h);
  }, [q, value, onChange]);
  return (
    <SearchField value={q} onChange={setQ} placeholder={t('Search')} onClear={() => setQ('')}
      onKeyDown={(e) => { if (e.key === 'Escape' && q) { e.stopPropagation(); setQ(''); } }} />
  );
}

// ------------------------------------------------------------------ bulk actions

function BulkBar({ sel, designs, folders, trash, onClear, onDone }: {
  sel: Set<string>; designs: DesignSummary[]; folders: FolderT[]; trash?: boolean; onClear: () => void; onDone: () => void;
}) {
  const toast = useToast();
  const [moving, setMoving] = useState(false);
  const [busy, setBusy] = useState(false);
  const { designs: dIds, folders: fIds } = splitSelection(sel);
  const pFolders = folders.filter(isProjectFolder);
  const selFolders = pFolders.filter((f) => fIds.includes(f.id));
  const selDesigns = designs.filter((d) => dIds.includes(d.id));
  const allStarred = selDesigns.every((d) => d.starred) && selFolders.every((f) => f.starred);
  const run = async (work: () => Promise<number>, ok?: string) => {
    setBusy(true);
    try {
      const failed = await work();
      if (failed) toast(t('Some items couldn’t be changed.'));
      else if (ok) toast(ok);
    } catch (e) {
      toast(errText(e));
    } finally {
      setBusy(false);
      onDone();
    }
  };
  /** Folder changes go one by one; returns how many failed. */
  const eachFolder = async (fn: (id: string) => Promise<unknown>, ids = fIds) => {
    let failed = 0;
    for (const id of ids) await fn(id).catch(() => failed++);
    return failed;
  };
  // Deleting a folder deletes its subfolders, so a selected subfolder of another selected folder is already covered.
  const topFolders = fIds.filter((id) => !fIds.some((other) => other !== id && subtreeIds(pFolders, other).has(id)));
  const bulk = async (action: Parameters<typeof projectsApi.bulk>[1], folderId?: string | null) => (dIds.length ? (await projectsApi.bulk(dIds, action, folderId)).failed.length : 0);

  const star = () => run(async () => (await bulk(allStarred ? 'unstar' : 'star')) + (await eachFolder((id) => projectsApi.updateFolder(id, { starred: !allStarred }))));
  const move = (target: string | null) => {
    setMoving(false);
    const name = target ? pFolders.find((f) => f.id === target)?.name ?? t('Folder') : t('Projects');
    void run(async () => (await bulk('move', target)) + (await eachFolder((id) => projectsApi.updateFolder(id, { parentId: target }))), t('Moved to {folder}', { folder: name }));
  };
  const remove = () => {
    if (fIds.length && !confirm(tn(fIds.length, 'Delete {n} folder? Designs inside are kept.', 'Delete {n} folders? Designs inside are kept.'))) return;
    void run(async () => (await bulk('trash')) + (await eachFolder((id) => projectsApi.deleteFolder(id), topFolders)), t('Moved to trash'));
  };
  const destroy = () => {
    if (!confirm(tn(dIds.length, 'Delete {n} design forever? This can’t be undone.', 'Delete {n} designs forever? This can’t be undone.'))) return;
    void run(() => bulk('delete'), t('Delete forever'));
  };
  // A move target must take every selected item: designs can be filed there, and no folder goes inside itself.
  const canTake = (target: ProjectFolder) => (!dIds.length || target.can_file) && (!fIds.length || (target.can_add && fIds.every((id) => canMoveFolderTo(pFolders, id, target.id))));
  return (
    <div className="pj-bulk" role="toolbar" aria-label={tn(sel.size, '{n} selected', '{n} selected')}>
      <span className="pj-bulk-n" aria-live="polite">{tn(sel.size, '{n} selected', '{n} selected')}</span>
      {trash ? (
        <>
          <button className="btn sm" disabled={busy} onClick={() => void run(() => bulk('restore'), t('Design restored'))}><RotateCcw size={16} /> {t('Restore')}</button>
          <button className="btn sm danger" disabled={busy} onClick={destroy}><Trash2 size={16} /> {t('Delete forever')}</button>
        </>
      ) : (
        <>
          <button className="btn sm" disabled={busy} onClick={() => void star()}><Star size={16} fill={allStarred ? 'currentColor' : 'none'} /> {allStarred ? t('Unstar') : t('Star')}</button>
          <button className="btn sm" disabled={busy} onClick={() => setMoving(true)}><FolderInput size={16} /> {t('Move to folder')}</button>
          <button className="btn sm danger" disabled={busy} onClick={remove}><Trash2 size={16} /> {t('Move to trash')}</button>
        </>
      )}
      <button className="btn sm icon" aria-label={t('Clear')} title={t('Clear')} onClick={onClear}><X size={16} /></button>
      {moving && <FolderPickerDialog onClose={() => setMoving(false)} onPick={move} canPick={canTake} />}
    </div>
  );
}

// ------------------------------------------------------------------ folders

function FolderTile({ f, folders, view, selection, sub, onChanged }: {
  f: FolderT; folders: FolderT[]; view: ViewMode; selection?: Selection; sub?: string; onChanged: () => void;
}) {
  const nav = useNavigate();
  const toast = useToast();
  const me = useAuth().user?.id;
  const [dialog, setDialog] = useState<'rename' | 'move' | null>(null);
  const pf = isProjectFolder(f) ? f : null;
  const key = folderKey(f.id);
  const selected = selection?.has(key) ?? false;
  const starred = Boolean(pf?.starred);
  const pFolders = folders.filter(isProjectFolder);
  const onClick = (e: MouseEvent) => {
    if (selection && (selection.active || e.shiftKey || e.metaKey || e.ctrlKey)) selection.toggle(key, e.shiftKey);
    else nav(`/projects/${f.id}`);
  };
  const act = async (work: Promise<unknown>, ok?: string) => {
    try {
      await work;
      if (ok) toast(ok);
    } catch (e) {
      toast(errText(e));
    }
    onChanged();
  };
  const items = tn(f.designs + (pf?.subfolders ?? 0), '{n} item', '{n} items');
  const icon = f.team ? <Users size={18} /> : <Folder size={18} />;
  const check = selection && (
    <input type="checkbox" className="pj-check" checked={selected} aria-label={f.name}
      onChange={() => {}} onClick={(e) => { e.stopPropagation(); selection.toggle(key, e.shiftKey); }} />
  );
  const actions = (
    <div className="pj-actions" onClick={(e) => e.stopPropagation()}>
      {pf && (
        <button type="button" className={`btn sm icon ghost pj-star ${starred ? 'on' : ''}`} aria-pressed={starred} title={starred ? t('Unstar') : t('Star')} aria-label={starred ? t('Unstar') : t('Star')}
          onClick={() => void act(projectsApi.updateFolder(f.id, { starred: !starred }))}>
          <Star size={16} fill={starred ? 'currentColor' : 'none'} />
        </button>
      )}
      <PopoverButton className="btn sm icon ghost" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={(close) => (
        <div className="menu" role="menu">
          <MenuItem icon={<ExternalLink size={16} />} label={t('Open in new tab')} onClick={() => { close(); window.open(`/projects/${f.id}`, '_blank', 'noopener'); }} />
          {pf && <MenuItem icon={<Star size={16} />} label={starred ? t('Unstar') : t('Star')} onClick={() => { close(); void act(projectsApi.updateFolder(f.id, { starred: !starred })); }} />}
          {pf?.can_manage && <MenuItem icon={<Pencil size={16} />} label={t('Rename')} onClick={() => { close(); setDialog('rename'); }} />}
          {pf?.can_manage && <MenuItem icon={<FolderInput size={16} />} label={t('Move to folder')} onClick={() => { close(); setDialog('move'); }} />}
          {pf?.can_manage && <div className="hsep" />}
          {pf?.can_manage && (
            <MenuItem icon={<Trash2 size={16} />} label={t('Delete')} danger onClick={() => {
              close();
              if (confirm(t('Delete this folder? Designs inside are kept.'))) void act(projectsApi.deleteFolder(f.id));
            }} />
          )}
        </div>
      )} />
    </div>
  );
  const dialogs = (
    <>
      {dialog === 'rename' && <RenameFolderDialog f={f} onClose={() => setDialog(null)} onDone={() => { setDialog(null); onChanged(); }} />}
      {dialog === 'move' && (
        <FolderPickerDialog value={f.parent_id} onClose={() => setDialog(null)} canPick={(x) => x.can_add && canMoveFolderTo(pFolders, f.id, x.id)}
          onPick={(target) => { setDialog(null); if (target !== f.parent_id) void act(projectsApi.updateFolder(f.id, { parentId: target }), t('Moved to {folder}', { folder: target ? pFolders.find((x) => x.id === target)?.name ?? t('Folder') : t('Projects') })); }} />
      )}
    </>
  );
  const owner = pf ? (pf.owner_id === me ? t('You') : pf.owner_name) : '';
  // The name is the tile's keyboard entry point (a real button: Enter and Space); the tile itself is only a bigger
  // mouse target, so the checkbox, star and menu stay separate controls next to it.
  const name = (cls: string) => (
    <button type="button" className={`pj-open ${cls} pj-ellipsis`} title={f.name} onClick={(e) => { e.stopPropagation(); onClick(e); }}>{f.name}</button>
  );
  if (view === 'list') {
    return (
      <>
        <div className={`pj-row ${selected ? 'selected' : ''}`} onClick={onClick}>
          {check ?? <span />}
          <div className="pj-thumb">{icon}</div>
          <div style={{ minWidth: 0 }}>
            {name('pj-title')}
            <div className="pj-sub pj-ellipsis">{[items, sub ?? f.team].filter(Boolean).join(' · ')}</div>
          </div>
          <span className="pj-col pj-ellipsis">{t('Folder')}</span>
          <span className="pj-col pj-ellipsis">{owner}</span>
          <span className="pj-col pj-ellipsis">{pf ? timeAgo(pf.created_at) : ''}</span>
          {actions}
        </div>
        {dialogs}
      </>
    );
  }
  return (
    <>
      <div className={`pj-folder ${selected ? 'selected' : ''}`} onClick={onClick} title={f.name}>
        <span className="pj-ficon">{icon}</span>
        {check}
        <span className="grow" style={{ minWidth: 0 }}>
          {name('pj-fname')}
          <span className="pj-fsub pj-ellipsis" style={{ display: 'block' }}>{[items, sub ?? f.team].filter(Boolean).join(' · ')}</span>
        </span>
        {actions}
      </div>
      {dialogs}
    </>
  );
}

function FolderSection({ folders, all, view, selection, onChanged, paths }: {
  folders: FolderT[]; all: FolderT[]; view: ViewMode; selection?: Selection; onChanged: () => void; paths?: boolean;
}) {
  if (!folders.length) return null;
  // While searching, each match shows where it is.
  const sub = (f: FolderT) => (paths ? folderPath(all, f.parent_id).map((p) => p.name).join(' / ') || undefined : undefined);
  const tiles = folders.map((f) => <FolderTile key={f.id} f={f} folders={all} view={view} selection={selection} sub={sub(f)} onChanged={onChanged} />);
  return (
    <section>
      <div className="section-head"><h2>{t('Folders')}</h2></div>
      {view === 'list' ? <div className="pj-list"><ListHead dateLabel={t('Created')} />{tiles}</div> : <div className="pj-folders">{tiles}</div>}
    </section>
  );
}

function RenameFolderDialog({ f, onClose, onDone }: { f: FolderT; onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const [name, setName] = useState(f.name);
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('Rename folder')} onClose={onClose} size="sm">
      <form className="col" style={{ gap: 14 }} onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim()) return;
        setBusy(true);
        try {
          await projectsApi.updateFolder(f.id, { name: name.trim() });
          onDone();
        } catch (err) {
          toast(errText(err));
          setBusy(false);
        }
      }}>
        <label className="field">{t('Folder name')}<input className="input" value={name} maxLength={100} autoFocus onChange={(e) => setName(e.target.value)} /></label>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button type="button" className="btn" onClick={onClose}>{t('Cancel')}</button>
          <button type="submit" className="btn primary" disabled={busy || !name.trim() || name.trim() === f.name}>{t('Save')}</button>
        </div>
      </form>
    </Modal>
  );
}

/** New folder (replaces the old prompt()): inside the current folder, or at the top level, personal or for a team. */
function NewFolderDialog({ parent, folders, onClose, onCreated }: { parent: ProjectFolder | null; folders: FolderT[]; onClose: () => void; onCreated: (id: string) => void }) {
  const toast = useToast();
  const teams = useWorkspaces().list.filter((w) => !w.personal && w.role !== 'member');
  const [name, setName] = useState('');
  const [where, setWhere] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('New folder')} onClose={onClose} size="sm">
      <form className="col" style={{ gap: 14 }} onSubmit={async (e) => {
        e.preventDefault();
        if (!name.trim()) return;
        setBusy(true);
        try {
          const { id } = await projectsApi.createFolder({ name: name.trim(), ...(parent ? { parentId: parent.id } : where ? { workspaceId: where } : {}) });
          onCreated(id);
        } catch (err) {
          toast(errText(err));
          setBusy(false);
        }
      }}>
        <label className="field">{where ? t('Team folder name') : t('Folder name')}
          <input className="input" value={name} maxLength={100} autoFocus onChange={(e) => setName(e.target.value)} />
        </label>
        {parent ? (
          <p className="faint" style={{ margin: 0, fontSize: 13 }}><Folder size={14} style={{ verticalAlign: -2 }} /> {folderPath(folders, parent.id).map((f) => f.name).join(' / ')}</p>
        ) : teams.length > 0 && (
          <label className="field">{t('Location')}
            <select className="input" value={where} onChange={(e) => setWhere(e.target.value)}>
              <option value="">{t('Personal folder')}</option>
              {teams.map((w) => <option key={w.id} value={w.id}>{t('Shared with {team}', { team: w.name })}</option>)}
            </select>
          </label>
        )}
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button type="button" className="btn" onClick={onClose}>{t('Cancel')}</button>
          <button type="submit" className="btn primary" disabled={busy || !name.trim()}>{t('Create')}</button>
        </div>
      </form>
    </Modal>
  );
}

// ------------------------------------------------------------------ Projects

/** Starts a design inside the folder from an uploaded photo or video, or imports a PDF, PowerPoint, Word or Markdown file into it. */
function useUploadHere(folderId: string | null) {
  const { start, status } = useStartFromFile();
  const ref = useRef<HTMLInputElement>(null);
  const input = <input ref={ref} type="file" hidden accept={`${MEDIA_ACCEPT},${IMPORT_ACCEPT}`} onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ''; if (f) void start(f, undefined, folderId); }} />;
  return { open: () => ref.current?.click(), input, busy: Boolean(status) };
}

export function ProjectsPage({ folders, reload }: { folders: FolderT[]; reload: () => unknown }) {
  const { folderId = null } = useParams();
  const nav = useNavigate();
  const toast = useToast();
  const me = useAuth().user?.id;
  const teams = useWorkspaces().list;
  const [list, setList] = useListState();
  const [view, setView] = useViewMode();
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const [recent, setRecent] = useState<DesignSummary[] | null>(null);
  const [missing, setMissing] = useState(false);
  const [dialog, setDialog] = useState<'folder' | 'design' | 'rename' | 'move' | null>(null);
  const track = useRef<HTMLDivElement>(null);
  const pFolders = useMemo(() => folders.filter(isProjectFolder), [folders]);
  const folder = folderId ? pFolders.find((f) => f.id === folderId) ?? null : null;
  const searching = Boolean(list.q.trim());
  const filtered = searching || list.type !== '' || list.owner !== 'anyone';
  // Loose designs at the top level are all the user's own, so an owner filter there looks across all of Projects.
  const deep = searching || (!folderId && list.owner !== 'anyone');
  const upload = useUploadHere(folderId);

  const load = useCallback(async () => {
    if (list.type === 'folder') return setDesigns([]);
    try {
      const r = await projectsApi.designs({ filter: 'folder', folder: folderId ?? undefined, q: list.q, surface: list.type, sort: list.sort, owner: list.owner, deep });
      setMissing(false);
      setDesigns(r.designs);
    } catch (e) {
      // A folder that's gone, isn't the user's, or isn't a folder id at all.
      setMissing(e instanceof ApiError && (e.status === 404 || e.status === 400));
      setDesigns([]);
    }
  }, [folderId, list.q, list.type, list.sort, list.owner, deep]);
  useEffect(() => {
    setDesigns(null);
    void load();
  }, [load]);
  const showRecents = !folderId && !filtered;
  const loadRecent = useCallback(() => api.designs({ filter: 'recent' }).then((r) => setRecent(r.designs.slice(0, RECENTS))).catch(() => setRecent([])), []);
  useEffect(() => {
    if (showRecents) void loadRecent();
  }, [showRecents, loadRecent]);
  const refresh = useCallback(() => {
    void reload();
    void load();
    if (showRecents) void loadRecent();
  }, [reload, load, showRecents, loadRecent]);

  // Folders shown: this level's children, or (searching) matches anywhere below this level.
  const shownFolders = useMemo(() => {
    const scope = searching ? pFolders.filter((f) => f.id !== folderId && (!folderId || subtreeIds(pFolders, folderId).has(f.id))) : childFolders(pFolders, folderId);
    return sortFolders(filterFolders(scope, list, me), list.sort);
  }, [pFolders, folderId, searching, list, me]);
  const order = useMemo(() => [...shownFolders.map((f) => folderKey(f.id)), ...(designs ?? []).map((d) => designKey(d.id))], [shownFolders, designs]);
  const { sel, selection, clear, all } = useSelection(order, `${folderId}|${list.q}|${list.type}|${list.owner}|${list.sort}`);

  const path = folderPath(pFolders, folderId);
  // Inside a folder (once it's loaded), what the user may add to it; at the top level, anything.
  const canAddFolder = folderId ? Boolean(folder?.can_add) : true;
  const canAddDesign = folderId ? Boolean(folder?.can_file) : true;
  const team = folder?.team ? teams.find((w) => w.id === folder.workspace_id) : undefined;
  // Brand controls (A8): members always, and designers when the team requires it, start team designs from its templates.
  const [requireTemplate, setRequireTemplate] = useState(false);
  useEffect(() => {
    setRequireTemplate(false);
    if (team?.role === 'designer') void brandHub.controls(team.id).then((r) => setRequireTemplate(Boolean(r.controls.requireTemplate))).catch(() => {});
  }, [team?.id, team?.role]);
  const templatesOnly = team?.role === 'member' || (team?.role === 'designer' && requireTemplate);
  // A photo or video upload starts a blank design, which such a team would refuse (after storing the file).
  const canUpload = canAddDesign && !templatesOnly;

  const crumbs = (
    <nav className="pj-crumbs" aria-label={t('Folders')}>
      {folderId ? <Link to="/projects">{t('Projects')}</Link> : <h2>{t('Projects')}</h2>}
      {path.map((f, i) => (
        <span key={f.id} className="row" style={{ gap: 2 }}>
          <span className="pj-sep" aria-hidden><ChevronRight size={16} /></span>
          {i < path.length - 1 ? <Link to={`/projects/${f.id}`}>{f.name}</Link> : <h2 aria-current="page">{f.team ? <Users size={18} /> : <Folder size={18} />} {f.name}</h2>}
        </span>
      ))}
      {folderId && !folder && <><span className="pj-sep" aria-hidden><ChevronRight size={16} /></span><h2>{t('Folder')}</h2></>}
    </nav>
  );
  const folderMenu = folder && (
    <>
      <button type="button" className={`btn ghost icon pj-star ${folder.starred ? 'on' : ''}`} aria-pressed={folder.starred} title={folder.starred ? t('Unstar') : t('Star')} aria-label={folder.starred ? t('Unstar') : t('Star')}
        onClick={() => void projectsApi.updateFolder(folder.id, { starred: !folder.starred }).then(() => reload(), (e) => toast(errText(e)))}>
        <Star size={18} fill={folder.starred ? 'currentColor' : 'none'} />
      </button>
      {folder.can_manage && (
        <PopoverButton className="btn ghost icon" title={t('More actions')} label={<MoreHorizontal size={18} />} menu content={(close) => (
          <div className="menu" role="menu">
            <MenuItem icon={<Pencil size={16} />} label={t('Rename folder')} onClick={() => { close(); setDialog('rename'); }} />
            <MenuItem icon={<FolderInput size={16} />} label={t('Move to folder')} onClick={() => { close(); setDialog('move'); }} />
            <div className="hsep" />
            <MenuItem icon={<Trash2 size={16} />} label={t('Delete')} danger onClick={async () => {
              close();
              if (!confirm(t('Delete this folder? Designs inside are kept.'))) return;
              try {
                await projectsApi.deleteFolder(folder.id);
                await reload();
                nav(folder.parent_id ? `/projects/${folder.parent_id}` : '/projects');
              } catch (e) {
                toast(errText(e));
              }
            }} />
          </div>
        )} />
      )}
    </>
  );
  const addNew = (
    <PopoverButton className="btn primary" title={t('Add new')} label={<><Plus size={16} /> {t('Add new')}</>} menu content={(close) => (
      <div className="menu" role="menu">
        <MenuItem icon={<FolderPlus size={16} />} label={t('Folder')} disabled={!canAddFolder} onClick={() => { close(); setDialog('folder'); }} />
        <MenuItem icon={<Plus size={16} />} label={t('Design')} disabled={!canAddDesign} onClick={() => { close(); setDialog('design'); }} />
        <MenuItem icon={<Upload size={16} />} label={upload.busy ? t('Uploading…') : t('Upload')} disabled={!canUpload || upload.busy} onClick={() => { close(); upload.open(); }} />
      </div>
    )} />
  );

  const nothing = designs !== null && !designs.length && !shownFolders.length;
  // Next goes towards the end of the row: leftwards when the page is right-to-left.
  const scrollRecents = (dir: 1 | -1) => {
    const el = track.current;
    el?.querySelector('.pj-track')?.scrollBy({ left: dir * (isRtl() ? -1 : 1) * el.clientWidth * 0.8, behavior: 'smooth' });
  };
  return (
    <>
      <div className="topbar pj-topbar">
        {crumbs}
        {folderMenu}
        <div className="grow" />
        <SearchBox value={list.q} onChange={(q) => setList({ q })} />
        {addNew}
      </div>
      <div className={`content ${selection.active ? 'pj-selecting' : ''}`}>
        <ListControls list={list} setList={setList} view={view} setView={setView} total={order.length} selected={sel.size} onSelectAll={(on) => (on ? all() : clear())} />

        {showRecents && recent && recent.length > 0 && (
          <section>
            <div className="section-head">
              <h2>{t('Recent designs')}</h2>
              <div className="row" style={{ gap: 4 }}>
                <button className="btn ghost sm icon pj-nav" aria-label={t('Previous')} title={t('Previous')} onClick={() => scrollRecents(-1)}><ChevronLeft size={16} /></button>
                <button className="btn ghost sm icon pj-nav" aria-label={t('Next')} title={t('Next')} onClick={() => scrollRecents(1)}><ChevronRight size={16} /></button>
              </div>
            </div>
            <div ref={track}><DesignGrid designs={recent} reload={refresh} folders={folders} empty="" view="row" /></div>
          </section>
        )}

        <FolderSection folders={shownFolders} all={pFolders} view={view} selection={selection} onChanged={refresh} paths={searching} />

        {missing ? <EmptyState icon={<Folder size={22} />}>{t('Nothing here yet.')}</EmptyState>
          : nothing ? (
            <EmptyState icon={filtered ? <Search size={22} /> : <Folder size={22} />}
              action={folderId && !filtered && canAddDesign && <button className="btn" onClick={() => setDialog('design')}><Plus size={16} /> {t('New design here')}</button>}>
              {filtered ? t('Nothing matches these filters.') : folderId ? t('This folder is empty.') : t('No loose designs.')}
            </EmptyState>
          )
            : (designs === null || designs.length > 0) && (
              <section>
                <div className="section-head"><h2>{folderId || filtered ? t('Designs') : t('Designs not in a folder')}</h2></div>
                <DesignGrid designs={designs} reload={refresh} folders={folders} empty="" view={view} selection={selection} />
              </section>
            )}

        {sel.size > 0 && <BulkBar sel={sel} designs={designs ?? []} folders={folders} onClear={clear} onDone={() => { clear(); refresh(); }} />}
      </div>
      {upload.input}
      {dialog === 'folder' && (
        <NewFolderDialog parent={folder} folders={pFolders} onClose={() => setDialog(null)}
          onCreated={() => { setDialog(null); void reload(); }} />
      )}
      {dialog === 'design' && (
        <CreateDialog onClose={() => setDialog(null)} folderId={folderId ?? undefined} workspaceId={team?.id} templatesOnly={templatesOnly} />
      )}
      {dialog === 'rename' && folder && <RenameFolderDialog f={folder} onClose={() => setDialog(null)} onDone={() => { setDialog(null); void reload(); }} />}
      {dialog === 'move' && folder && (
        <FolderPickerDialog value={folder.parent_id} onClose={() => setDialog(null)} canPick={(x) => x.can_add && canMoveFolderTo(pFolders, folder.id, x.id)}
          onPick={async (target) => {
            setDialog(null);
            if (target === folder.parent_id) return;
            try {
              await projectsApi.updateFolder(folder.id, { parentId: target });
              await reload();
              toast(t('Moved to {folder}', { folder: target ? pFolders.find((x) => x.id === target)?.name ?? t('Folder') : t('Projects') }));
            } catch (e) {
              toast(errText(e));
            }
          }} />
      )}
    </>
  );
}

// ------------------------------------------------------------------ Starred, Shared, Trash and team lists

export function DesignList({ title, filter, folders, hint, params, actions, reloadFolders }: {
  title: string; filter: string; folders: FolderT[]; hint?: string; params?: Record<string, string>; actions?: ReactNode;
  /** Starred: refresh the folder list after a folder is changed here. */
  reloadFolders?: () => unknown;
}) {
  const me = useAuth().user?.id;
  const [list, setList] = useListState();
  const [view, setView] = useViewMode();
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const key = JSON.stringify(params ?? {});
  const trash = filter === 'trash';
  // Starred also lists starred folders (g20); the other lists hold designs only.
  const withFolders = filter === 'starred';
  const load = useCallback(() => {
    if (list.type === 'folder') return Promise.resolve(setDesigns([]));
    return projectsApi.designs({ filter, ...(JSON.parse(key) as Record<string, string>), q: list.q, surface: list.type, sort: list.sort, owner: list.owner })
      .then((r) => setDesigns(r.designs), () => setDesigns([]));
  }, [filter, key, list.q, list.type, list.sort, list.owner]);
  useEffect(() => {
    setDesigns(null);
    void load();
  }, [load]);
  const refresh = useCallback(() => {
    void load();
    void reloadFolders?.();
  }, [load, reloadFolders]);
  const starredFolders = useMemo(() => (withFolders ? sortFolders(filterFolders(folders.filter((f) => isProjectFolder(f) && f.starred), list, me), list.sort) : []), [withFolders, folders, list, me]);
  const order = useMemo(() => [...starredFolders.map((f) => folderKey(f.id)), ...(designs ?? []).map((d) => designKey(d.id))], [starredFolders, designs]);
  const { sel, selection, clear, all } = useSelection(order, `${filter}|${key}|${list.q}|${list.type}|${list.owner}|${list.sort}`);
  const filtered = Boolean(list.q.trim()) || list.type !== '' || list.owner !== 'anyone';
  const nothing = designs !== null && !designs.length && !starredFolders.length;
  return (
    <>
      <PageHeader title={title}>
        <div className="grow" />
        <SearchBox value={list.q} onChange={(q) => setList({ q })} />
        {actions}
      </PageHeader>
      <div className={`content ${selection.active ? 'pj-selecting' : ''}`}>
        {hint && <p className="muted" style={{ margin: 0 }}>{hint}</p>}
        <ListControls list={list} setList={setList} view={view} setView={setView} owner={filter !== 'shared' && !trash} folderType={withFolders}
          total={order.length} selected={sel.size} onSelectAll={(on) => (on ? all() : clear())} />
        <FolderSection folders={starredFolders} all={folders} view={view} selection={selection} onChanged={refresh} />
        {nothing ? <EmptyState icon={filtered ? <Search size={22} /> : filter === 'starred' ? <Star size={22} /> : <Users size={22} />}>{filtered ? t('Nothing matches these filters.') : filter === 'starred' ? t('Star designs and folders to find them here.') : filter === 'shared' ? t('Designs people share with you appear here.') : t('Nothing here yet.')}</EmptyState> : (designs === null || designs.length > 0) && (
          <section>
            {withFolders && <div className="section-head"><h2>{t('Designs')}</h2></div>}
            <DesignGrid designs={designs} reload={refresh} folders={folders} trash={trash} empty="" view={view} selection={selection} />
          </section>
        )}
        {sel.size > 0 && <BulkBar sel={sel} designs={designs ?? []} folders={folders} trash={trash} onClear={clear} onDone={() => { clear(); refresh(); }} />}
      </div>
    </>
  );
}
