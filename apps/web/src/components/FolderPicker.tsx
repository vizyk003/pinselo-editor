/**
 * Folder picker (plan 11 g17/g18): a modal tree of the user's folders and their teams' folders, with search and
 * 'New folder'. `FolderPicker` is the shared entry point (move a design, choose where something goes);
 * `FolderPickerDialog` adds the options the Projects page needs to move folders.
 */
import { ChevronDown, ChevronRight, Folder, FolderPlus, Home as HomeIcon, Users } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { ApiError } from '../api';
import { projectsApi, type ProjectFolder } from '../api/projects';
import { useAuth } from '../App';
import { isRtl, t } from '../i18n';
import { childFolders, folderPath, matchesQuery, sortFolders } from '../pages/home/projectsModel';
import { Modal, SearchField, useToast } from './ui';
import '../pages/home/projects.css';

export function FolderPicker({ value, onPick, onClose, title }: { value?: string | null; onPick: (folderId: string | null) => void; onClose: () => void; title?: string }) {
  return <FolderPickerDialog value={value} onPick={onPick} onClose={onClose} title={title} />;
}

export function FolderPickerDialog({ value, onPick, onClose, title, canPick, allowRoot = true }: {
  value?: string | null; onPick: (folderId: string | null) => void; onClose: () => void; title?: string;
  /** Folders that can be chosen (the others show, disabled). Default: the folders the user can put designs in. */
  canPick?: (f: ProjectFolder) => boolean;
  /** Whether the top level ('Projects', i.e. no folder) can be chosen. */
  allowRoot?: boolean;
}) {
  const toast = useToast();
  const me = useAuth().user?.id;
  const [folders, setFolders] = useState<ProjectFolder[] | null>(null);
  const [q, setQ] = useState('');
  /** The highlighted target: a folder id, null for the top level, undefined for nothing yet. */
  const [sel, setSel] = useState<string | null | undefined>(value ?? (allowRoot ? null : undefined));
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [naming, setNaming] = useState<string | null>(null);
  /** The row that holds the tree's single Tab stop ('' = the top level), once the user has moved focus in the tree. */
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const tree = useRef<HTMLDivElement>(null);
  const pickable = useCallback((f: ProjectFolder) => (canPick ? canPick(f) : f.can_file), [canPick]);

  const load = useCallback(() => projectsApi.folders().then((r) => { setFolders(r.folders); return r.folders; }), []);
  useEffect(() => {
    // Opens on the current folder: its ancestors start expanded.
    void load().then((all) => setOpen(new Set(folderPath(all, value).slice(0, -1).map((f) => f.id)))).catch(() => setFolders([]));
  }, [load, value]);

  const byParent = useMemo(() => {
    const m = new Map<string | null, ProjectFolder[]>();
    if (!folders) return m;
    for (const f of sortFolders(childFolders(folders, null), 'az')) m.set(null, [...(m.get(null) ?? []), f]);
    for (const f of sortFolders(folders, 'az')) if (f.parent_id && folders.some((p) => p.id === f.parent_id)) m.set(f.parent_id, [...(m.get(f.parent_id) ?? []), f]);
    return m;
  }, [folders]);

  // Rows in display order: the tree (expanded branches only), or the search matches with their paths. `pos` / `size`
  // place each row among its siblings for assistive tech (the tree is rendered flat, so the levels are explicit).
  const rows = useMemo(() => {
    if (!folders) return [];
    if (q.trim()) {
      const hits = sortFolders(folders.filter((f) => matchesQuery(f.name, q)), 'az');
      return hits.map((f, i) => ({ f, depth: 0, pos: i + 1, size: hits.length }));
    }
    const out: Array<{ f: ProjectFolder; depth: number; pos: number; size: number }> = [];
    const walk = (parent: string | null, depth: number) => {
      const kids = byParent.get(parent) ?? [];
      kids.forEach((f, i) => {
        out.push({ f, depth, pos: i + 1, size: kids.length });
        if (open.has(f.id)) walk(f.id, depth + 1);
      });
    };
    walk(null, allowRoot ? 1 : 0);
    return out;
  }, [folders, q, byParent, open, allowRoot]);
  const showRoot = allowRoot && !q.trim();
  // Roving tabindex: one Tab stop in the tree (the focused row, else the highlighted one, else the first).
  const keys = [...(showRoot ? [''] : []), ...rows.map((r) => r.f.id)];
  const tabKey = focusKey !== null && keys.includes(focusKey) ? focusKey : sel !== undefined && keys.includes(sel ?? '') ? sel ?? '' : keys[0];

  const current = value ? folders?.find((f) => f.id === value) : undefined;
  const selFolder = sel ? folders?.find((f) => f.id === sel) : undefined;
  // 'New folder' goes inside the highlighted folder when the user may add folders there, else at the top level.
  const newParent = selFolder?.can_add ? selFolder.id : null;
  const canConfirm = sel === null ? allowRoot : Boolean(selFolder && pickable(selFolder));
  const toggle = (id: string) => setOpen((s) => {
    const n = new Set(s);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    return n;
  });

  const createFolder = async (name: string) => {
    try {
      const { id } = await projectsApi.createFolder({ name, parentId: newParent });
      await load();
      if (newParent) setOpen((s) => new Set(s).add(newParent));
      setSel(id);
      setQ('');
    } catch (e) {
      toast(e instanceof ApiError && e.message !== e.code ? e.message : t('Failed'));
    }
    setNaming(null);
  };

  // Tree keys (WAI-ARIA tree pattern): Up/Down/Home/End move between rows; Right opens a folder, then goes to its first
  // subfolder; Left closes it, then goes to its parent (mirrored right-to-left); Enter picks.
  const onKeyDown = (e: KeyboardEvent) => {
    const items = [...(tree.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (i < 0) return;
    const item = items[i]!;
    const id = item.dataset.id || null;
    const depth = Number(item.dataset.depth);
    const expanded = item.getAttribute('aria-expanded');
    const focus = (n: number) => items[Math.max(0, Math.min(items.length - 1, n))]?.focus();
    const [inKey, outKey] = isRtl() ? ['ArrowLeft', 'ArrowRight'] : ['ArrowRight', 'ArrowLeft'];
    if (e.key === 'ArrowDown') focus(i + 1);
    else if (e.key === 'ArrowUp') focus(i - 1);
    else if (e.key === 'Home') focus(0);
    else if (e.key === 'End') focus(items.length - 1);
    else if (e.key === inKey) {
      if (expanded === 'false' && id) toggle(id);
      else if (expanded === 'true' && Number(items[i + 1]?.dataset.depth) > depth) focus(i + 1);
    } else if (e.key === outKey) {
      if (expanded === 'true') {
        if (id) toggle(id);
      } else {
        for (let j = i - 1; j >= 0; j--) {
          if (Number(items[j]!.dataset.depth) < depth) {
            items[j]!.focus();
            break;
          }
        }
      }
    } else if (e.key === 'Enter') {
      // Picks the focused row (its click hasn't updated the highlight yet).
      const f = id ? folders?.find((x) => x.id === id) : undefined;
      if (id ? f && pickable(f) : allowRoot) onPick(id);
    } else return;
    e.preventDefault();
  };

  return (
    <Modal title={title ?? t('Move to folder')} onClose={onClose} size="md">
      {current && <p className="faint" style={{ margin: 0, fontSize: 13 }}>{t('✓ In {folder}', { folder: folderPath(folders ?? [], current.id).map((f) => f.name).join(' / ') })}</p>}
      <SearchField value={q} onChange={setQ} placeholder={t('Search folders')} autoFocus
        onKeyDown={(e) => {
          // Down from the search box goes into the tree.
          if (e.key !== 'ArrowDown') return;
          const row = tree.current?.querySelector<HTMLElement>('[role="treeitem"][tabindex="0"]');
          if (row) { e.preventDefault(); row.focus(); }
        }} />
      <div className="pj-tree" role="tree" aria-label={t('Folders')} ref={tree} onKeyDown={onKeyDown}>
        {!folders ? <div className="empty">{t('Loading…')}</div> : (
          <>
            {showRoot && (
              <button type="button" role="treeitem" aria-selected={sel === null} aria-level={1} aria-setsize={1} aria-posinset={1} aria-expanded={rows.length ? true : undefined}
                data-id="" data-depth={0} tabIndex={tabKey === '' ? 0 : -1} onFocus={() => setFocusKey('')}
                className={`pj-tree-row ${sel === null ? 'on' : ''}`} style={{ paddingInlineStart: 8 }}
                onClick={() => setSel(null)} onDoubleClick={() => onPick(null)}>
                <span className="pj-tree-tw" /><HomeIcon size={16} /> <span className="grow">{t('Projects')}</span>
              </button>
            )}
            {rows.map(({ f, depth, pos, size }) => {
              const kids = byParent.has(f.id) && !q.trim();
              const ok = pickable(f);
              const path = q.trim() ? folderPath(folders, f.parent_id).map((p) => p.name).join(' / ') : '';
              return (
                <div key={f.id} role="none" className="pj-tree-item">
                  <button type="button" role="treeitem" aria-selected={sel === f.id} aria-expanded={kids ? open.has(f.id) : undefined} aria-disabled={!ok || undefined}
                    aria-level={depth + 1} aria-setsize={size} aria-posinset={pos} data-id={f.id} data-depth={depth}
                    tabIndex={tabKey === f.id ? 0 : -1} onFocus={() => setFocusKey(f.id)}
                    className={`pj-tree-row ${sel === f.id ? 'on' : ''} ${ok ? '' : 'off'}`} style={{ paddingInlineStart: 8 + depth * 18 }}
                    onClick={() => ok && setSel(f.id)} onDoubleClick={() => ok && onPick(f.id)}>
                    <span className="pj-tree-tw" onClick={(e) => { if (kids) { e.stopPropagation(); toggle(f.id); } }} aria-hidden>
                      {kids && (open.has(f.id) ? <ChevronDown size={14} /> : <ChevronRight size={14} />)}
                    </span>
                    {f.team ? <Users size={16} /> : <Folder size={16} />}
                    <span className="grow pj-ellipsis">{f.name}{path && <span className="faint"> · {path}</span>}</span>
                    {f.team && <span className="faint pj-ellipsis" style={{ fontSize: 12, maxWidth: 140 }}>{f.team}</span>}
                    {f.id === value && <span className="faint" aria-hidden>✓</span>}
                  </button>
                </div>
              );
            })}
            {q.trim() && !rows.length && <div className="empty">{t('No results')}</div>}
          </>
        )}
      </div>
      {naming !== null && (
        <form className="row" onSubmit={(e) => { e.preventDefault(); if (naming.trim()) void createFolder(naming.trim()); }}>
          <FolderPlus size={16} className="faint" />
          <input className="input grow" value={naming} maxLength={100} autoFocus placeholder={t('Folder name')} aria-label={t('Folder name')}
            onChange={(e) => setNaming(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setNaming(null); } }} />
          <button className="btn" type="submit" disabled={!naming.trim()}>{t('Create')}</button>
        </form>
      )}
      <div className="row">
        <button className="btn ghost" type="button" onClick={() => setNaming('')} disabled={naming !== null || !folders || (Boolean(selFolder) && !selFolder!.can_add)}>
          <FolderPlus size={16} /> {t('New folder')}
        </button>
        <div className="grow" />
        <button className="btn" type="button" onClick={onClose}>{t('Cancel')}</button>
        <button className="btn primary pj-ellipsis" type="button" style={{ maxWidth: 240 }} disabled={!canConfirm} onClick={() => onPick(sel ?? null)}>
          {sel === undefined ? t('Move to folder') : t('Move to {folder}', { folder: selFolder?.name ?? t('Projects') })}
        </button>
      </div>
    </Modal>
  );
}
