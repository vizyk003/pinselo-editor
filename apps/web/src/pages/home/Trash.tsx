/**
 * Trash (g22): designs and uploads you've deleted, each with a "deleted in N days" countdown to the worker's
 * 30-day purge, Restore and Delete forever, bulk select, Empty trash and a grid/list toggle.
 * Designs keep their existing trash API (GET /api/designs?filter=trash, POST /api/designs/:id/restore,
 * DELETE /api/designs/:id?permanent=1); uploads use /api/assets/trash, restore and batch.
 */
import { formatDims, type Unit } from '@oc/engine';
import { LayoutTemplate, Lock, MoreHorizontal, RotateCcw, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useState, type ReactNode } from 'react';
import { useSearchParams } from 'react-router';
import { api, ApiError, thumbUrl, type DesignSummary, type Folder as FolderT } from '../../api';
import { uploadsApi, type TrashedUpload } from '../../api/uploads';
import { tabPanelProps, tabProps } from '../../components/tabs';
import { EmptyState, MenuItem, PageHeader, PopoverButton, timeAgo, useToast, ViewToggle } from '../../components/ui';
import { fmtDate, t, tn } from '../../i18n';
import { TypeChips, UploadThumb, uploadMeta, useViewMode } from './Uploads';
import {
  appendPage, BATCH_MAX, daysLeft, EMPTY_COUNTS, PAGE_SIZE, pruneSelection, selectionState, toggleId, TRASH_DAYS,
  type UploadCounts, type UploadType,
} from './uploadsModel';
import './uploads.css';

type Tab = 'designs' | 'uploads';

/**
 * One row of either tab, as the shared grid/list renders it. `held`: taken down or reported, so on hold for review —
 * neither restorable nor deletable by the owner.
 */
interface TrashItem { id: string; name: string; sub: string; trashedAt: string; thumb: (mini: boolean) => ReactNode; extra?: string; held?: boolean }

const TABS = ['designs', 'uploads'] as const;

export function TrashPage({ folders }: { folders: FolderT[] }) {
  const [params, setParams] = useSearchParams();
  const tab: Tab = params.get('tab') === 'uploads' ? 'uploads' : 'designs';
  const [view, setView] = useViewMode('trash');
  const setTab = (next: Tab) => setParams(next === 'designs' ? {} : { tab: next }, { replace: true });
  const tabsId = useId();
  return (
    <>
      <PageHeader title={t('Trash')}>
        <div className="grow" />
        <ViewToggle mode={view} onChange={setView} />
      </PageHeader>
      <div className="content">
        <div className="up-head">
          <div className="tabs" role="tablist" aria-label={t('Trash')}>
            <button {...tabProps(tabsId, TABS, tab, setTab, 'designs')} className={tab === 'designs' ? 'active' : ''}>{t('Designs')}</button>
            <button {...tabProps(tabsId, TABS, tab, setTab, 'uploads')} className={tab === 'uploads' ? 'active' : ''}>{t('Uploads')}</button>
          </div>
          <p className="muted" style={{ margin: 0 }}>{tn(TRASH_DAYS, 'Items in the trash are deleted forever after {n} day. Restore them before then to keep them.', 'Items in the trash are deleted forever after {n} days. Restore them before then to keep them.')}</p>
        </div>
        <div {...tabPanelProps(tabsId, tab)} className="trash-panel">
          {tab === 'designs' ? <DesignTrash view={view} folders={folders} key="designs" /> : <UploadTrash view={view} key="uploads" />}
        </div>
      </div>
    </>
  );
}

// ------------------------------------------------------------------ designs

function DesignTrash({ view, folders }: { view: 'grid' | 'list'; folders: FolderT[] }) {
  const toast = useToast();
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const load = useCallback(() => api.designs({ filter: 'trash' }).then((r) => setDesigns(r.designs)), []);
  useEffect(() => {
    load().catch(() => setDesigns([]));
  }, [load]);
  const folderName = useMemo(() => new Map(folders.map((f) => [f.id, f.name])), [folders]);
  const items = useMemo<TrashItem[] | null>(() => designs?.map((d) => ({
    id: d.id, name: d.title, trashedAt: d.trashed_at ?? d.updated_at,
    sub: `${formatDims(d.width, d.height, d.unit as Unit)} · ${tn(d.page_count, '{n} page', '{n} pages')}`,
    extra: d.folder_id ? folderName.get(d.folder_id) : undefined,
    held: Boolean((d as DesignSummary & { held?: boolean }).held),
    thumb: () => <DesignThumb d={d} />,
  })) ?? null, [designs, folderName]);

  /** Runs `fn` on every id, carrying on past failures (say, one already deleted in another tab); returns how many worked. */
  const each = async (ids: string[], fn: (id: string) => Promise<unknown>) => {
    let done = 0;
    try {
      for (const id of ids) await fn(id).then(() => done++, () => {});
    } finally {
      void load();
    }
    if (done < ids.length) toast(t('Some items couldn’t be changed.'));
    return done;
  };
  const restore = async (ids: string[]) => {
    const done = await each(ids, (id) => api.restoreDesign(id));
    if (done) toast(done === 1 && ids.length === 1 ? t('Design restored') : tn(done, '{n} design restored', '{n} designs restored'));
  };
  const remove = async (ids: string[]) => {
    const done = await each(ids, (id) => api.trashDesign(id, true));
    if (done > 1) toast(tn(done, '{n} design deleted forever', '{n} designs deleted forever'));
  };
  /** Every trashed design, not just the listed ones (the list stops at 200). Designs on hold stay. */
  const emptyAll = async () => {
    let gone = 0;
    try {
      for (let i = 0; i < 50; i++) {
        const batch = (await api.designs({ filter: 'trash' })).designs.filter((d) => !(d as DesignSummary & { held?: boolean }).held);
        if (!batch.length) break;
        let done = 0;
        for (const d of batch) await api.trashDesign(d.id, true).then(() => done++, () => {});
        gone += done;
        if (!done) break;
      }
    } finally {
      void load();
    }
    toast(tn(gone, '{n} design deleted forever', '{n} designs deleted forever'));
  };
  return (
    <TrashList items={items} view={view} onRestore={restore} onDelete={remove} empty={t('No designs in the trash.')}
      onEmpty={designs?.length ? emptyAll : undefined} emptyConfirm={t('Delete all designs in the trash forever? This can’t be undone.')} />
  );
}

function DesignThumb({ d }: { d: DesignSummary }) {
  const [ok, setOk] = useState(true);
  return ok ? <img src={thumbUrl(d.id, d.updated_at)} alt="" loading="lazy" onError={() => setOk(false)} /> : <LayoutTemplate size={28} className="faint" />;
}

// ------------------------------------------------------------------ uploads

function UploadTrash({ view }: { view: 'grid' | 'list' }) {
  const toast = useToast();
  const [type, setType] = useState<'all' | UploadType>('all');
  const [rows, setRows] = useState<TrashedUpload[] | null>(null);
  const [total, setTotal] = useState(0);
  const [counts, setCounts] = useState<UploadCounts>(EMPTY_COUNTS);
  const load = useCallback(async (offset = 0) => {
    const r = await uploadsApi.trash({ type, limit: PAGE_SIZE, offset });
    setRows((cur) => (offset && cur ? appendPage(cur, r.assets) : r.assets));
    setTotal(r.total);
    setCounts(r.counts);
  }, [type]);
  useEffect(() => {
    setRows(null);
    load().catch(() => setRows([]));
  }, [load]);
  const items = useMemo<TrashItem[] | null>(() => rows?.map((a) => ({
    id: a.id, name: a.name, trashedAt: a.trashed_at, sub: uploadMeta(a), held: Boolean(a.held), thumb: (mini) => <UploadThumb a={a} mini={mini} />,
  })) ?? null, [rows]);

  const restore = async (ids: string[]) => {
    try {
      const done = await uploadsApi.batch('restore', ids);
      toast(tn(done.length, '{n} upload restored', '{n} uploads restored'));
    } finally {
      void load();
    }
  };
  const remove = async (ids: string[]) => {
    try {
      const done = await uploadsApi.batch('delete', ids);
      if (done.length > 1) toast(tn(done.length, '{n} upload deleted forever', '{n} uploads deleted forever'));
    } finally {
      void load();
    }
  };
  /** Everything in the uploads trash, not just the loaded page. */
  const empty = async () => {
    let gone = 0;
    for (let i = 0; i < 50; i++) {
      const r = await uploadsApi.trash({ limit: BATCH_MAX });
      const ids = r.assets.filter((a) => !a.held).map((a) => a.id);
      if (!ids.length) break;
      const done = await uploadsApi.batch('delete', ids);
      gone += done.length;
      if (!done.length) break;
    }
    toast(tn(gone, '{n} upload deleted forever', '{n} uploads deleted forever'));
    void load();
  };
  return (
    <>
      <TypeChips value={type} counts={counts} onChange={setType} />
      <TrashList items={items} view={view} onRestore={restore} onDelete={remove}
        onEmpty={counts.all > 0 ? empty : undefined} emptyConfirm={t('Delete all uploads in the trash forever? This can’t be undone.')}
        more={rows && rows.length < total ? () => load(rows.length) : undefined}
        empty={type === 'all' ? t('No uploads in the trash.') : t('Nothing of this type in the trash.')} />
    </>
  );
}

// ------------------------------------------------------------------ shared grid / list

function countdown(trashedAt: string): string {
  const n = daysLeft(trashedAt);
  return n === 0 ? t('Deleted forever today') : tn(n, 'Deleted forever in {n} day', 'Deleted forever in {n} days');
}

function TrashList({ items, view, onRestore, onDelete, onEmpty, emptyConfirm, more, empty }: {
  items: TrashItem[] | null; view: 'grid' | 'list'; empty: string;
  onRestore: (ids: string[]) => Promise<void>; onDelete: (ids: string[]) => Promise<void>;
  onEmpty?: () => Promise<void>; emptyConfirm?: string; more?: () => Promise<void>;
}) {
  const toast = useToast();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const ids = useMemo(() => (items ?? []).map((i) => i.id), [items]);
  useEffect(() => setSelected((s) => (s.size ? pruneSelection(s, ids) : s)), [ids]);
  const state = selectionState(selected, ids);
  const run = async (fn: () => Promise<void>, keepSelection = false) => {
    setBusy(true);
    try {
      await fn();
      if (!keepSelection) setSelected(new Set());
    } catch (e) {
      // A plan limit already shows the upgrade prompt.
      if (!(e instanceof ApiError && e.status === 402)) toast(t('Couldn’t complete that. Try again.'));
    } finally {
      setBusy(false);
    }
  };
  const confirmDelete = (n: number) => confirm(tn(n, 'Delete {n} item forever? This can’t be undone.', 'Delete {n} items forever? This can’t be undone.'));
  const menu = (it: TrashItem) => (close: () => void) => (
    <div className="menu" role="menu">
      <MenuItem icon={<RotateCcw size={16} />} label={t('Restore')} disabled={it.held} onClick={() => { close(); void run(() => onRestore([it.id])); }} />
      <MenuItem icon={<Trash2 size={16} />} label={t('Delete forever')} danger disabled={it.held} onClick={() => { close(); void run(() => onDelete([it.id])); }} />
      {it.held && <p className="faint" style={{ margin: '4px 10px', fontSize: 12, maxWidth: 220 }}>{t('On hold while our team reviews a report. You can’t restore or delete it yet.')}</p>}
    </div>
  );
  // Bulk actions skip items on hold (the server refuses them anyway).
  const actionable = (ids: Iterable<string>) => [...ids].filter((id) => !items?.find((x) => x.id === id)?.held);
  const selectAll = (
    <input type="checkbox" className="up-check" aria-label={t('Select all')} checked={state === 'all'}
      ref={(el) => { if (el) el.indeterminate = state === 'some'; }}
      onChange={() => setSelected(state === 'all' ? new Set() : new Set(ids))} />
  );

  if (!items) return <div className="empty">{t('Loading…')}</div>;
  return (
    <>
      {(selected.size > 0 || (onEmpty && items.length > 0)) && (
        <div className={`up-selbar${selected.size ? '' : ' idle'}`} role="region" aria-label={t('Selection')}>
          {selected.size > 0 ? (
            <>
              {selectAll}
              <span className="grow">{tn(selected.size, '{n} selected', '{n} selected')}</span>
              <button className="btn sm" disabled={busy || !actionable(selected).length} onClick={() => void run(() => onRestore(actionable(selected)))}><RotateCcw size={14} /> {t('Restore')}</button>
              <button className="btn sm danger" disabled={busy || !actionable(selected).length} onClick={() => { const ids = actionable(selected); if (confirmDelete(ids.length)) void run(() => onDelete(ids)); }}><Trash2 size={14} /> {t('Delete forever')}</button>
              <button className="btn sm ghost" onClick={() => setSelected(new Set())}>{t('Clear')}</button>
            </>
          ) : (
            <>
              {selectAll}
              <span className="grow muted">{tn(items.length, '{n} item', '{n} items')}</span>
              <button className="btn sm danger" disabled={busy} onClick={() => { if (onEmpty && confirm(emptyConfirm ?? '')) void run(onEmpty); }}><Trash2 size={14} /> {t('Empty trash')}</button>
            </>
          )}
        </div>
      )}
      {!items.length ? <EmptyState icon={<Trash2 size={22} />}>{empty}</EmptyState> : view === 'grid' ? (
        <div className={`grid up-grid${selected.size ? ' selecting' : ''}`}>
          {items.map((it) => {
            const on = selected.has(it.id);
            const soon = daysLeft(it.trashedAt) <= 3;
            return (
              <div key={it.id} className={`card${on ? ' selected' : ''}`}>
                {/* Clicking the preview selects it too (a mouse shortcut); keyboard users have the checkbox. */}
                <div className="thumb" onClick={() => setSelected((s) => toggleId(s, it.id))}>
                  {it.thumb(false)}
                  {it.held && <span className="up-badge" title={t('On hold while our team reviews a report. You can’t restore or delete it yet.')}><Lock size={11} /> {t('On hold')}</span>}
                  <label className="pick" onClick={(e) => e.stopPropagation()}>
                    <input type="checkbox" className="up-check" checked={on} onChange={() => setSelected((s) => toggleId(s, it.id))} aria-label={t('Select {name}', { name: it.name })} />
                  </label>
                </div>
                <div className="meta">
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="t" title={it.name}>{it.name}</div>
                    <div className="s">{it.sub}</div>
                    <div className="s" style={soon ? { color: 'var(--danger)' } : undefined} title={t('Moved to trash {when}', { when: fmtDate(it.trashedAt, { dateStyle: 'medium', timeStyle: 'short' }) })}>{countdown(it.trashedAt)}</div>
                  </div>
                </div>
                <div className="more">
                  <PopoverButton className="btn sm icon" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={menu(it)} />
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <table className="up-list">
          <thead>
            <tr>
              <th style={{ width: 28 }}>{selectAll}</th>
              <th className="thumbcell" aria-label={t('Preview')} />
              <th>{t('Name')}</th>
              <th className="wide">{t('Details')}</th>
              <th className="wide">{t('Deleted')}</th>
              <th>{t('Deleted forever in')}</th>
              <th style={{ width: 40 }} aria-label={t('Actions')} />
            </tr>
          </thead>
          <tbody>
            {items.map((it) => {
              const n = daysLeft(it.trashedAt);
              return (
                <tr key={it.id} className={selected.has(it.id) ? 'selected' : ''}>
                  <td><input type="checkbox" className="up-check" checked={selected.has(it.id)} onChange={() => setSelected((s) => toggleId(s, it.id))} aria-label={t('Select {name}', { name: it.name })} /></td>
                  <td className="thumbcell"><div className="mini">{it.thumb(true)}</div></td>
                  <td>
                    <div className="name" title={it.name} style={{ cursor: 'default' }}>{it.name}</div>
                    {it.extra && <div className="faint" style={{ fontSize: 12 }}>{t('In {folder}', { folder: it.extra })}</div>}
                    {it.held && <div className="faint" style={{ fontSize: 12 }}><Lock size={11} /> {t('On hold')}</div>}
                  </td>
                  <td className="wide num">{it.sub}</td>
                  <td className="wide num" title={fmtDate(it.trashedAt, { dateStyle: 'medium', timeStyle: 'short' })}>{timeAgo(it.trashedAt)}</td>
                  <td className="num" style={n <= 3 ? { color: 'var(--danger)' } : undefined}>{n === 0 ? t('Today') : tn(n, '{n} day', '{n} days')}</td>
                  <td><PopoverButton className="btn sm icon ghost" title={t('More actions')} label={<MoreHorizontal size={16} />} menu content={menu(it)} /></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {more && <button className="btn" style={{ alignSelf: 'center' }} disabled={busy} onClick={() => void run(more, true)}>{busy ? t('Loading…') : t('Load more')}</button>}
    </>
  );
}
