/**
 * Pure helpers of the Projects hub (plan 11 g17/g20): the folder tree (levels, breadcrumbs, valid move targets),
 * filters and sort orders, shift-click range selection, list state in the URL and the per-user grid/list preference.
 */
import type { DesignSort, OwnerFilter } from '../../api/projects';

export interface TreeFolder { id: string; name: string; parent_id: string | null; workspace_id: string; created_at?: string; owner_id?: string }

/**
 * The folders of one level: the children of `parentId`, or (null) the top level, which also takes in folders whose
 * parent the user can't see, so nothing becomes unreachable.
 */
export function childFolders<F extends TreeFolder>(folders: readonly F[], parentId: string | null): F[] {
  if (parentId) return folders.filter((f) => f.parent_id === parentId);
  const ids = new Set(folders.map((f) => f.id));
  return folders.filter((f) => !f.parent_id || !ids.has(f.parent_id));
}

/** The folder and its ancestors, top level first. Stops at a parent the user can't see (or a cycle). */
export function folderPath<F extends TreeFolder>(folders: readonly F[], id: string | null | undefined): F[] {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const path: F[] = [];
  const seen = new Set<string>();
  for (let f = id ? byId.get(id) : undefined; f && !seen.has(f.id); f = f.parent_id ? byId.get(f.parent_id) : undefined) {
    seen.add(f.id);
    path.unshift(f);
  }
  return path;
}

/** Ids of a folder and everything below it. */
export function subtreeIds(folders: readonly TreeFolder[], id: string): Set<string> {
  const out = new Set([id]);
  for (let grew = true; grew;) {
    grew = false;
    for (const f of folders) {
      if (f.parent_id && out.has(f.parent_id) && !out.has(f.id)) {
        out.add(f.id);
        grew = true;
      }
    }
  }
  return out;
}

/** Whether folder `id` can move under `target` (null = top level): same workspace, not into itself or below itself. */
export function canMoveFolderTo(folders: readonly TreeFolder[], id: string, target: string | null): boolean {
  const f = folders.find((x) => x.id === id);
  if (!f) return false;
  if (target === null) return true;
  const t = folders.find((x) => x.id === target);
  return Boolean(t) && t!.workspace_id === f.workspace_id && !subtreeIds(folders, id).has(target);
}

/** Case-insensitive match of every word of `q` in `text`. */
export function matchesQuery(text: string, q: string): boolean {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hay = text.toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** Folders in a list's sort order: by creation date for Last edited / Oldest (folders have no edit date), else by name. */
export function sortFolders<F extends TreeFolder>(folders: readonly F[], sort: DesignSort): F[] {
  const byName = (a: F, b: F) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true });
  const byDate = (a: F, b: F) => (a.created_at ?? '').localeCompare(b.created_at ?? '');
  const out = [...folders];
  switch (sort) {
    case 'az': return out.sort(byName);
    case 'za': return out.sort((a, b) => byName(b, a));
    case 'oldest': return out.sort((a, b) => byDate(a, b) || byName(a, b));
    default: return out.sort((a, b) => byDate(b, a) || byName(a, b));
  }
}

/** The type filter: '' (everything), 'folder' (folders only) or a design surface (designs of that kind only). */
export type TypeFilter = '' | 'folder' | string;

export interface ListState { q: string; sort: DesignSort; type: TypeFilter; owner: OwnerFilter }
export const DEFAULT_LIST: ListState = { q: '', sort: 'edited', type: '', owner: 'anyone' };
const SORTS: readonly DesignSort[] = ['edited', 'oldest', 'az', 'za'];
const OWNERS: readonly OwnerFilter[] = ['anyone', 'me', 'shared'];

/** List state from the page URL (`?q=&sort=&type=&owner=`); unknown values fall back to the defaults. */
export function listStateFromParams(p: URLSearchParams): ListState {
  const sort = p.get('sort') as DesignSort | null;
  const owner = p.get('owner') as OwnerFilter | null;
  const type = p.get('type') ?? '';
  return {
    q: p.get('q') ?? '',
    sort: sort && SORTS.includes(sort) ? sort : 'edited',
    type: /^[a-z]{1,20}$/.test(type) ? type : '',
    owner: owner && OWNERS.includes(owner) ? owner : 'anyone',
  };
}

/** The URL query for a list state, keeping unrelated parameters of `base`; defaults are left out. */
export function listStateToParams(s: ListState, base = new URLSearchParams()): URLSearchParams {
  const p = new URLSearchParams(base);
  const set = (k: string, v: string, d: string) => (v && v !== d ? p.set(k, v) : p.delete(k));
  set('q', s.q, '');
  set('sort', s.sort, 'edited');
  set('type', s.type, '');
  set('owner', s.owner, 'anyone');
  return p;
}

/** Folders that pass a list's search, type and owner filters (designs are filtered by the server). */
export function filterFolders<F extends TreeFolder>(folders: readonly F[], s: ListState, me: string | null | undefined): F[] {
  if (s.type && s.type !== 'folder') return [];
  return folders.filter((f) =>
    (!s.q.trim() || matchesQuery(f.name, s.q))
    && (s.owner === 'anyone' || (s.owner === 'me' ? f.owner_id === me : f.owner_id !== me)));
}

/**
 * Selection after clicking an item's checkbox. A plain click toggles the item; a shift-click selects everything from
 * the anchor (the last plain-clicked item) to it, in the order the items are shown, keeping what was already selected.
 */
export function toggleSelection(order: readonly string[], selected: ReadonlySet<string>, key: string, range: { shift: boolean; anchor: string | null }): Set<string> {
  const next = new Set(selected);
  const a = range.anchor ? order.indexOf(range.anchor) : -1;
  const b = order.indexOf(key);
  if (range.shift && a >= 0 && b >= 0) {
    for (let i = Math.min(a, b); i <= Math.max(a, b); i++) next.add(order[i]!);
    return next;
  }
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}

/** Keys of selected designs and folders. */
export const designKey = (id: string) => `d:${id}`;
export const folderKey = (id: string) => `f:${id}`;
export function splitSelection(keys: Iterable<string>): { designs: string[]; folders: string[] } {
  const designs: string[] = [];
  const folders: string[] = [];
  for (const k of keys) (k.startsWith('f:') ? folders : designs).push(k.slice(2));
  return { designs, folders };
}

export type ViewMode = 'grid' | 'list';
const viewKey = (userId?: string | null) => `oc.projects.view:${userId ?? 'anon'}`;

/** The grid/list choice of this user in this browser (grid when storage is blocked or empty). */
export function loadViewMode(userId?: string | null): ViewMode {
  try {
    return localStorage.getItem(viewKey(userId)) === 'list' ? 'list' : 'grid';
  } catch {
    return 'grid';
  }
}

export function saveViewMode(userId: string | null | undefined, mode: ViewMode): void {
  try {
    localStorage.setItem(viewKey(userId), mode);
  } catch {
    /* storage blocked: the choice lasts until reload */
  }
}
