/**
 * Dashboard Uploads (g19): every file you've uploaded, with name search, a type filter, sort, grid or list view,
 * sizes and dimensions, per-item Details / Download / Rename / Use in a design / Move to trash, bulk trash, and
 * upload by button, folder or drag and drop (through the regular upload path, so quota and safety checks apply).
 * Uploads have no folders (assets carry no folder), so there is no "Move to folder".
 */
import { ChevronRight, Download, FileImage, FolderUp, Info, MoreHorizontal, Music, Pencil, Search, Sparkles, Trash2, UploadCloud } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { api, assetUrl } from '../../api';
import { downloadUpload, uploadsApi, type UploadRow } from '../../api/uploads';
import { MenuItem, Modal, PageHeader, PopoverButton, SearchField, timeAgo, useToast, ViewToggle } from '../../components/ui';
import { fmtDate, getLocale, t, tk, tn } from '../../i18n';
import { filesFromDrop, folderInputRef, uploadMany, type UploadOutcome } from '../../lib/uploadFiles';
import { mediaDesignSize, startsVideo, titleFromFile } from './startMedia';
import {
  appendPage, DEFAULT_UPLOAD_FILTERS, EMPTY_COUNTS, fmtBytes, fmtDuration, PAGE_SIZE, pruneSelection, selectionState, toggleId,
  UPLOAD_SORTS, UPLOAD_TYPES, uploadTypeOf, type UploadCounts, type UploadFilters, type UploadSort, type UploadType,
} from './uploadsModel';
import './uploads.css';

export const TYPE_LABELS: Record<'all' | UploadType, string> = { all: tk('All'), images: tk('Images'), videos: tk('Videos'), audio: tk('Audio'), files: tk('Files') };
const ONE_TYPE: Record<UploadType, string> = { images: tk('Image'), videos: tk('Video'), audio: tk('Audio'), files: tk('File') };
const SORT_LABELS: Record<UploadSort, string> = { newest: tk('Newest first'), oldest: tk('Oldest first'), name: tk('Name (A–Z)'), size: tk('Largest first') };

/** Grid or list, remembered per page in this browser. */
export function useViewMode(key: string): ['grid' | 'list', (v: 'grid' | 'list') => void] {
  const storageKey = `oc:view:${key}`;
  const [mode, setMode] = useState<'grid' | 'list'>(() => {
    try {
      return localStorage.getItem(storageKey) === 'list' ? 'list' : 'grid';
    } catch {
      return 'grid';
    }
  });
  const set = useCallback((v: 'grid' | 'list') => {
    setMode(v);
    try {
      localStorage.setItem(storageKey, v);
    } catch {
      /* storage blocked */
    }
  }, [storageKey]);
  return [mode, set];
}


/** Type filter chips with counts; "Files" only shows when there are any (today's uploads are all images, video or audio). */
export function TypeChips({ value, counts, onChange }: { value: 'all' | UploadType; counts: UploadCounts; onChange: (v: 'all' | UploadType) => void }) {
  const types = (['all', ...UPLOAD_TYPES] as const).filter((ty) => ty !== 'files' || counts.files > 0 || value === 'files');
  return (
    <div className="up-chips" role="group" aria-label={t('Type')}>
      {types.map((ty) => (
        <button key={ty} className={`chip${value === ty ? ' on' : ''}`} aria-pressed={value === ty} onClick={() => onChange(ty)}>
          {t(TYPE_LABELS[ty])} <span className="n">{counts[ty]}</span>
        </button>
      ))}
    </div>
  );
}

/** Thumbnail of an upload: the image itself, a video's poster frame, or a note icon for audio. */
export function UploadThumb({ a, mini }: { a: Pick<UploadRow, 'id' | 'kind' | 'name'>; mini?: boolean }) {
  const [ok, setOk] = useState(true);
  if (a.kind === 'audio') return mini ? <Music size={18} /> : <span className="up-audio"><Music size={28} /><span>{a.name}</span></span>;
  if (!ok) return <FileImage size={mini ? 18 : 28} className="faint" />;
  // Graphics (SVG) show whole; photos and video frames fill the tile.
  return <img src={a.kind === 'video' ? `${assetUrl(a.id)}/poster` : assetUrl(a.id)} alt="" loading="lazy" className={a.kind === 'svg' ? 'up-img contain' : 'up-img'} onError={() => setOk(false)} />;
}

/** A file size in the current locale ("1.2 MB", "1,2 MB"). */
export const sizeText = (bytes: number) => fmtBytes(bytes, getLocale());

/** "PNG · 1920 × 1080 · 1.2 MB" (and the duration of video and audio). */
export function uploadMeta(a: Pick<UploadRow, 'mime' | 'kind' | 'w' | 'h' | 'bytes' | 'durationMs' | 'animated'>): string {
  const parts = [formatOf(a.mime)];
  if (a.kind !== 'audio' && a.w && a.h) parts.push(`${a.w} × ${a.h}`);
  if ((a.kind === 'video' || a.kind === 'audio') && !a.animated && a.durationMs) parts.push(fmtDuration(a.durationMs));
  parts.push(sizeText(Number(a.bytes)));
  return parts.join(' · ');
}

const formatOf = (mime: string) => ({ 'image/svg+xml': 'SVG', 'video/quicktime': 'MOV', 'audio/mpeg': 'MP3', 'audio/mp4': 'M4A' } as Record<string, string>)[mime]
  ?? (mime.split('/')[1] ?? mime).toUpperCase();

/** Shared outcome toast for multi-file uploads. */
export function uploadSummary(r: UploadOutcome): string {
  const parts: string[] = [];
  if (r.assets.length) parts.push(tn(r.assets.length, '{n} file uploaded', '{n} files uploaded'));
  if (r.skipped) parts.push(tn(r.skipped, '{n} file skipped (unsupported type)', '{n} files skipped (unsupported type)'));
  if (r.failed.length) parts.push(r.failed.length === 1 ? t('{name}: {message}', { name: r.failed[0]!.name, message: r.failed[0]!.message }) : tn(r.failed.length, '{n} file failed', '{n} files failed'));
  return parts.join(' · ') || t('Nothing to upload. Choose images, videos or audio.');
}

const hasFiles = (e: React.DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');

export function UploadsPage() {
  const toast = useToast();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const [filters, setFilters] = useState<UploadFilters>(() => ({ ...DEFAULT_UPLOAD_FILTERS, q: params.get('q') ?? '' }));
  const [q, setQ] = useState(filters.q);
  const [view, setView] = useViewMode('uploads');
  const [rows, setRows] = useState<UploadRow[] | null>(null);
  const [total, setTotal] = useState(0);
  const [counts, setCounts] = useState<UploadCounts>(EMPTY_COUNTS);
  const [usage, setUsage] = useState<{ used: number; quota: number } | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [details, setDetails] = useState<UploadRow | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [over, setOver] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const depth = useRef(0);
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement | null>(null);
  const seq = useRef(0);

  // A search link (?q=, e.g. an upload picked in unified search) replaces the box while the page is open.
  const urlQ = params.get('q') ?? '';
  useEffect(() => setQ(urlQ), [urlQ]);
  // Search as you type (debounced).
  useEffect(() => {
    const timer = setTimeout(() => setFilters((f) => (f.q === q ? f : { ...f, q })), 250);
    return () => clearTimeout(timer);
  }, [q]);

  const load = useCallback(async (offset = 0) => {
    const mine = ++seq.current;
    const r = await uploadsApi.list({ ...filters, limit: PAGE_SIZE, offset });
    if (mine !== seq.current) return;
    setRows((cur) => (offset && cur ? appendPage(cur, r.assets) : r.assets));
    setTotal(r.total);
    setCounts(r.counts);
    setUsage({ used: r.usedBytes, quota: r.quotaBytes });
  }, [filters]);

  useEffect(() => {
    setRows(null);
    load().catch(() => toast(t('Couldn’t load your uploads.')));
  }, [load, toast]);

  // Selection follows what is listed.
  const ids = useMemo(() => (rows ?? []).map((r) => r.id), [rows]);
  useEffect(() => setSelected((s) => (s.size ? pruneSelection(s, ids) : s)), [ids]);
  const selState = selectionState(selected, ids);

  const upload = async (files: File[]) => {
    if (!files.length || progress) return;
    const r = await uploadMany(files, (done, n) => setProgress({ done, total: n }));
    setProgress(null);
    toast(uploadSummary(r));
    void load();
  };

  const trash = async (list: string[]) => {
    if (!list.length) return;
    try {
      const done = await uploadsApi.batch('trash', list);
      setSelected(new Set());
      toast(tn(done.length, '{n} upload moved to trash', '{n} uploads moved to trash'));
    } catch {
      toast(t('Couldn’t move that to the trash. Try again.'));
    }
    void load();
  };

  const rename = async (a: UploadRow, name: string) => {
    setRenaming(null);
    const next = name.trim();
    if (!next || next === a.name) return;
    try {
      await uploadsApi.rename(a.id, next);
      setRows((cur) => cur?.map((r) => (r.id === a.id ? { ...r, name: next } : r)) ?? cur);
    } catch {
      toast(t('Couldn’t rename it.'));
    }
  };

  /** A new design of the media's size with it placed full-bleed (the `?place=` intent of g15). */
  const useInDesign = async (a: UploadRow) => {
    try {
      const size = mediaDesignSize(a.w, a.h, a.kind);
      const { id } = await api.createDesign({ ...(startsVideo(a) ? { preset: 'video' } : {}), ...size, unit: 'px', title: titleFromFile(a.name) });
      nav(`/design/${id}?place=${encodeURIComponent(a.id)}`);
    } catch {
      toast(t('Couldn’t create the design.'));
    }
  };

  const actions = (a: UploadRow) => (close: () => void) => (
    <div className="menu" role="menu">
      <MenuItem icon={<Info size={16} />} label={t('Details')} onClick={() => { close(); setDetails(a); }} />
      <MenuItem icon={<Download size={16} />} label={t('Download')} onClick={() => { close(); downloadUpload(a.id); }} />
      <MenuItem icon={<Pencil size={16} />} label={t('Rename')} onClick={() => { close(); setRenaming(a.id); }} />
      {a.kind !== 'audio' && <MenuItem icon={<Sparkles size={16} />} label={t('Use in a design')} onClick={() => { close(); void useInDesign(a); }} />}
      <div className="hsep" />
      <MenuItem icon={<Trash2 size={16} />} label={t('Move to trash')} danger onClick={() => { close(); void trash([a.id]); }} />
    </div>
  );

  const filtering = Boolean(filters.q.trim()) || filters.type !== 'all';
  const pct = usage && usage.quota ? Math.min(100, (usage.used / usage.quota) * 100) : 0;

  return (
    <>
      <PageHeader className="up-topbar" title={<><Link to="/projects">{t('Projects')}</Link><ChevronRight size={20} className="up-crumb-sep" aria-hidden /><span>{t('Uploads')}</span></>}>
        <div className="grow" />
        <button className="btn" onClick={() => folderInput.current?.click()} disabled={Boolean(progress)}><FolderUp size={16} /> {t('Upload folder')}</button>
        <button className="btn primary" onClick={() => fileInput.current?.click()} disabled={Boolean(progress)}>
          <UploadCloud size={16} /> {progress ? t('Uploading {done} of {total}…', { done: progress.done, total: progress.total }) : t('Upload files')}
        </button>
        <input ref={fileInput} type="file" multiple hidden accept="image/png,image/jpeg,image/webp,image/gif,image/svg+xml,video/*,audio/*"
          onChange={(e) => { const f = [...(e.target.files ?? [])]; e.target.value = ''; void upload(f); }} />
        <input ref={(el) => { folderInput.current = el; folderInputRef(el); }} type="file" multiple hidden
          onChange={(e) => { const f = [...(e.target.files ?? [])]; e.target.value = ''; void upload(f); }} />
      </PageHeader>
      <div
        className="content home-drop-area"
        onDragEnter={(e) => { if (!hasFiles(e)) return; e.preventDefault(); depth.current++; setOver(true); }}
        onDragOver={(e) => { if (!hasFiles(e)) return; e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }}
        onDragLeave={() => { if (--depth.current <= 0) { depth.current = 0; setOver(false); } }}
        onDrop={(e) => {
          if (!hasFiles(e)) return;
          e.preventDefault();
          depth.current = 0;
          setOver(false);
          void filesFromDrop(e.dataTransfer).then(upload);
        }}
      >
        <div className="up-head">
        <div className="up-toolbar">
          <SearchField className="up-search" value={q} onChange={setQ} placeholder={t('Search your uploads')} onClear={() => setQ('')} />
          <select className="input" aria-label={t('Sort')} value={filters.sort} onChange={(e) => setFilters((f) => ({ ...f, sort: e.target.value as UploadSort }))}>
            {UPLOAD_SORTS.map((s) => <option key={s} value={s}>{t(SORT_LABELS[s])}</option>)}
          </select>
          <div className="grow" />
          <ViewToggle mode={view} onChange={setView} />
        </div>
        <TypeChips value={filters.type} counts={counts} onChange={(type) => setFilters((f) => ({ ...f, type }))} />
        {usage && (
          <div className="up-usage">
            <div className={`bar${pct >= 95 ? ' full' : ''}`} role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100} aria-label={t('Storage used')}><span style={{ width: `${pct}%` }} /></div>
            <span>{t('{used} of {quota} used', { used: sizeText(usage.used), quota: sizeText(usage.quota) })}</span>
          </div>
        )}
        </div>

        {selected.size > 0 && (
          <div className="up-selbar" role="region" aria-label={t('Selection')}>
            <input type="checkbox" className="up-check" aria-label={t('Select all')} checked={selState === 'all'}
              ref={(el) => { if (el) el.indeterminate = selState === 'some'; }}
              onChange={() => setSelected(selState === 'all' ? new Set() : new Set(ids))} />
            <span className="grow">{tn(selected.size, '{n} selected', '{n} selected')}</span>
            <button className="btn sm danger" onClick={() => void trash([...selected])}><Trash2 size={14} /> {t('Move to trash')}</button>
            <button className="btn sm ghost" onClick={() => setSelected(new Set())}>{t('Clear')}</button>
          </div>
        )}

        {!rows ? <div className="empty">{t('Loading…')}</div>
          : !rows.length ? (
            filtering ? <div className="empty">{t('No uploads match your search.')}</div> : (
              <button className="home-dropzone" onClick={() => fileInput.current?.click()}>
                <UploadCloud size={32} />
                <strong>{t('Upload your first files')}</strong>
                <span className="muted">{t('Images, videos and audio you upload appear here. You can also drop files or a folder on this page.')}</span>
              </button>
            )
          ) : view === 'grid' ? (
            <div className={`grid up-grid${selected.size ? ' selecting' : ''}`}>
              {rows.map((a) => (
                <UploadCard key={a.id} a={a} selected={selected.has(a.id)} onSelect={() => setSelected((s) => toggleId(s, a.id))}
                  onOpen={() => (selected.size ? setSelected((s) => toggleId(s, a.id)) : setDetails(a))}
                  renaming={renaming === a.id} onRename={(n) => void rename(a, n)} menu={actions(a)} />
              ))}
            </div>
          ) : (
            <UploadTable rows={rows} selected={selected} setSelected={setSelected} selState={selState} ids={ids}
              renaming={renaming} onRename={(a, n) => void rename(a, n)} onOpen={setDetails} menu={actions} />
          )}
        {rows && rows.length < total && (
          <button className="btn" style={{ alignSelf: 'center' }} disabled={loadingMore}
            onClick={() => { setLoadingMore(true); load(rows.length).finally(() => setLoadingMore(false)); }}>
            {loadingMore ? t('Loading…') : t('Load more')}
          </button>
        )}
        {over && <div className="drop-hint">{t('Drop files or a folder to upload')}</div>}
      </div>
      {details && <UploadDetails a={details} onClose={() => setDetails(null)} onUse={details.kind !== 'audio' ? () => void useInDesign(details) : undefined} />}
    </>
  );
}

/** Inline rename field (Enter or blur saves, Escape cancels). */
export function RenameField({ value, onDone }: { value: string; onDone: (name: string) => void }) {
  const [v, setV] = useState(value);
  // Escape then the blur of unmounting must not save the edit.
  const finished = useRef(false);
  const finish = (name: string) => {
    if (finished.current) return;
    finished.current = true;
    onDone(name);
  };
  return (
    <input className="input" style={{ height: 28 }} value={v} autoFocus maxLength={200} aria-label={t('Name')}
      onFocus={(e) => e.target.select()} onChange={(e) => setV(e.target.value)} onBlur={() => finish(v)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); else if (e.key === 'Escape') finish(value); }} />
  );
}

function UploadCard({ a, selected, onSelect, onOpen, renaming, onRename, menu }: {
  a: UploadRow; selected: boolean; onSelect: () => void; onOpen: () => void; renaming: boolean; onRename: (n: string) => void; menu: (close: () => void) => ReactNode;
}) {
  return (
    <div className={`card${selected ? ' selected' : ''}`}>
      <div className="thumb">
        {/* The preview is a real button (Enter and Space); the selection checkbox sits next to it, not inside. */}
        <button type="button" className="up-open" onClick={onOpen} aria-label={t('Details of {name}', { name: a.name })}>
          <UploadThumb a={a} />
          {(a.kind === 'video' || a.kind === 'audio') && !a.animated && a.durationMs ? <span className="up-badge">{fmtDuration(a.durationMs)}</span> : a.animated ? <span className="up-badge">GIF</span> : null}
        </button>
        <label className="pick">
          <input type="checkbox" className="up-check" checked={selected} onChange={onSelect} aria-label={t('Select {name}', { name: a.name })} />
        </label>
      </div>
      <div className="meta">
        <div className="grow" style={{ minWidth: 0 }}>
          {renaming ? <RenameField value={a.name} onDone={onRename} /> : <div className="t" title={a.name}>{a.name}</div>}
          <div className="s">{uploadMeta(a)}</div>
        </div>
      </div>
      <div className="more">
        <PopoverButton className="btn sm icon" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={menu} />
      </div>
    </div>
  );
}

function UploadTable({ rows, selected, setSelected, selState, ids, renaming, onRename, onOpen, menu }: {
  rows: UploadRow[]; selected: Set<string>; setSelected: (s: Set<string> | ((s: Set<string>) => Set<string>)) => void; selState: 'none' | 'some' | 'all'; ids: string[];
  renaming: string | null; onRename: (a: UploadRow, n: string) => void; onOpen: (a: UploadRow) => void; menu: (a: UploadRow) => (close: () => void) => ReactNode;
}) {
  return (
    <table className="up-list">
      <thead>
        <tr>
          <th style={{ width: 28 }}>
            <input type="checkbox" className="up-check" aria-label={t('Select all')} checked={selState === 'all'}
              ref={(el) => { if (el) el.indeterminate = selState === 'some'; }}
              onChange={() => setSelected(selState === 'all' ? new Set() : new Set(ids))} />
          </th>
          <th className="thumbcell" aria-label={t('Preview')} />
          <th>{t('Name')}</th>
          <th className="wide">{t('Type')}</th>
          <th className="wide">{t('Dimensions')}</th>
          <th>{t('Size')}</th>
          <th className="wide">{t('Uploaded')}</th>
          <th style={{ width: 40 }} aria-label={t('Actions')} />
        </tr>
      </thead>
      <tbody>
        {rows.map((a) => (
          <tr key={a.id} className={selected.has(a.id) ? 'selected' : ''}>
            <td><input type="checkbox" className="up-check" checked={selected.has(a.id)} onChange={() => setSelected((s) => toggleId(s, a.id))} aria-label={t('Select {name}', { name: a.name })} /></td>
            <td className="thumbcell"><div className="mini"><UploadThumb a={a} mini /></div></td>
            <td>
              {renaming === a.id ? <RenameField value={a.name} onDone={(n) => onRename(a, n)} />
                : <button type="button" className="name up-name" title={a.name} onClick={() => onOpen(a)}>{a.name}</button>}
            </td>
            <td className="wide num">{t(ONE_TYPE[uploadTypeOf(a)])} · {formatOf(a.mime)}</td>
            <td className="wide num">{a.kind !== 'audio' && a.w && a.h ? `${a.w} × ${a.h}` : a.durationMs ? fmtDuration(a.durationMs) : '—'}</td>
            <td className="num">{sizeText(Number(a.bytes))}</td>
            <td className="wide num" title={fmtDate(a.created_at, { dateStyle: 'medium', timeStyle: 'short' })}>{timeAgo(a.created_at)}</td>
            <td><PopoverButton className="btn sm icon ghost" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={menu(a)} /></td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function UploadDetails({ a, onClose, onUse }: { a: UploadRow; onClose: () => void; onUse?: () => void }) {
  const src = assetUrl(a.id);
  const credit = a.attribution && (a.attribution.author || a.attribution.provider);
  return (
    <Modal title={a.name || t('Upload details')} onClose={onClose} size="lg">
      <div className="up-details">
        <div className="preview">
          {a.kind === 'video' ? <video src={src} poster={`${src}/poster`} controls={!a.animated} autoPlay={Boolean(a.animated)} loop={Boolean(a.animated)} muted={Boolean(a.animated)} playsInline />
            : a.kind === 'audio' ? <audio src={src} controls style={{ width: '90%' }} />
              : <img src={src} alt={a.name} />}
        </div>
        <div className="col" style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
          <dl>
            <dt>{t('Type')}</dt><dd>{t(ONE_TYPE[uploadTypeOf(a)])}{a.animated ? ' (GIF)' : ''}</dd>
            <dt>{t('Format')}</dt><dd>{formatOf(a.mime)} <span className="faint">({a.mime})</span></dd>
            {a.kind !== 'audio' && a.w > 0 && <><dt>{t('Dimensions')}</dt><dd>{`${a.w} × ${a.h} px`}</dd></>}
            {(a.kind === 'video' || a.kind === 'audio') && a.durationMs ? <><dt>{t('Duration')}</dt><dd>{fmtDuration(a.durationMs)}</dd></> : null}
            <dt>{t('Size')}</dt><dd>{sizeText(Number(a.bytes))}</dd>
            <dt>{t('Uploaded')}</dt><dd>{fmtDate(a.created_at, { dateStyle: 'medium', timeStyle: 'short' })}</dd>
            {credit && <><dt>{t('Credit')}</dt><dd>{[a.attribution?.author, a.attribution?.provider, a.attribution?.license].filter(Boolean).join(' · ')}</dd></>}
          </dl>
          <div className="row" style={{ flexWrap: 'wrap' }}>
            <button className="btn primary" onClick={() => downloadUpload(a.id)}><Download size={16} /> {t('Download')}</button>
            {onUse && <button className="btn" onClick={onUse}><Sparkles size={16} /> {t('Use in a design')}</button>}
          </div>
        </div>
      </div>
    </Modal>
  );
}
