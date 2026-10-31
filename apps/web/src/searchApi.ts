/** Unified search client (g16): GET /api/search and /api/search/suggest, plus recent searches kept in this browser. */
import { ApiError, type DesignSummary, type Folder, type TemplateSummary, type UploadedAsset } from './api';

export type SearchType = 'all' | 'designs' | 'folders' | 'uploads' | 'templates';
export type SearchOwner = 'anyone' | 'me' | 'shared';
export type SearchModified = 'any' | '1d' | '7d' | '30d' | '365d';
export type SearchGroup = Exclude<SearchType, 'all'>;

export interface SearchFilters { type: SearchType; owner: SearchOwner; modified: SearchModified; surface?: string }
export const DEFAULT_FILTERS: SearchFilters = { type: 'all', owner: 'anyone', modified: 'any' };

/** Same shape as GET /api/designs rows, so results can use the regular design cards. */
export type SearchDesign = DesignSummary;
export type SearchFolder = Pick<Folder, 'id' | 'name' | 'parent_id' | 'workspace_id' | 'team' | 'designs'> & { created_at: string };
export type SearchUpload = Pick<UploadedAsset, 'id' | 'mime' | 'kind' | 'bytes' | 'w' | 'h' | 'name' | 'sha256' | 'durationMs' | 'animated'> & { created_at: string };
export interface SearchResults { q: string; designs: SearchDesign[]; folders: SearchFolder[]; uploads: SearchUpload[]; templates: TemplateSummary[] }
export interface Suggestion { kind: 'template' | 'category' | 'tag'; text: string; id?: string }

async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, { credentials: 'same-origin', signal });
  if (!res.ok) {
    let j: { error?: string; message?: string } = {};
    try {
      j = (await res.json()) as typeof j;
    } catch {
      /* not json */
    }
    throw new ApiError(res.status, j.error ?? 'error', j.message);
  }
  return res.json() as Promise<T>;
}

/** Query string for a search; defaults are left out so URLs stay short. */
export function searchParams(q: string, f: Partial<SearchFilters> = {}, limit?: number): URLSearchParams {
  const p = new URLSearchParams();
  if (q.trim()) p.set('q', q.trim());
  if (f.type && f.type !== 'all') p.set('type', f.type);
  if (f.owner && f.owner !== 'anyone') p.set('owner', f.owner);
  if (f.modified && f.modified !== 'any') p.set('modified', f.modified);
  if (f.surface) p.set('surface', f.surface);
  if (limit) p.set('limit', String(limit));
  return p;
}

const pick = <T extends string>(v: string | null, allowed: readonly T[], d: T): T => (v && (allowed as readonly string[]).includes(v) ? (v as T) : d);

/** Filters from a /search?… URL (unknown values fall back to the defaults). */
export function filtersFromParams(p: URLSearchParams): SearchFilters {
  return {
    type: pick(p.get('type'), ['all', 'designs', 'folders', 'uploads', 'templates'] as const, 'all'),
    owner: pick(p.get('owner'), ['anyone', 'me', 'shared'] as const, 'anyone'),
    modified: pick(p.get('modified'), ['any', '1d', '7d', '30d', '365d'] as const, 'any'),
    ...(p.get('surface') ? { surface: p.get('surface')! } : {}),
  };
}

/** The type a scope allows: a scoped box never searches outside its scope. */
export function scopedType(scope: 'all' | 'designs' | 'templates', t: SearchType): SearchType {
  return scope === 'all' ? t : scope;
}

export const searchApi = {
  search: (q: string, f: Partial<SearchFilters> = {}, limit = 8, signal?: AbortSignal) => get<SearchResults>(`/api/search?${searchParams(q, f, limit)}`, signal),
  suggest: (q: string, signal?: AbortSignal) => get<{ suggestions: Suggestion[] }>(`/api/search/suggest?${new URLSearchParams({ q })}`, signal),
};

// ---------------------------------------------------------------- recent searches (per signed-in user on this browser)

const RECENT_KEY = 'oc.search.recent';
const RECENT_MAX = 8;
/** One user's list; signed out (no uid) nothing is kept, so queries never show to the next account. */
const recentKey = (uid: string | null | undefined) => (uid ? `${RECENT_KEY}.${uid}` : null);

export function recentSearches(uid?: string | null): string[] {
  const key = recentKey(uid);
  if (!key) return [];
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? '[]') as unknown;
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}

/** Most recent first, case-insensitive de-duplication, capped. Pure so it can be tested. */
export function pushRecent(list: string[], q: string, max = RECENT_MAX): string[] {
  const v = q.replace(/\s+/g, ' ').trim();
  if (!v) return list;
  return [v, ...list.filter((x) => x.toLowerCase() !== v.toLowerCase())].slice(0, max);
}

export function rememberSearch(q: string, uid?: string | null): void {
  const key = recentKey(uid);
  if (!key) return;
  try {
    localStorage.setItem(key, JSON.stringify(pushRecent(recentSearches(uid), q)));
  } catch {
    /* storage blocked */
  }
}

export function forgetSearch(q: string | null, uid?: string | null): void {
  const key = recentKey(uid);
  if (!key) return;
  try {
    if (q === null) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(recentSearches(uid).filter((x) => x !== q)));
  } catch {
    /* storage blocked */
  }
}

/** Sign-out: drop the browser-wide lists older builds kept (not tied to any account). */
export function clearLegacyRecents(): void {
  try {
    localStorage.removeItem(RECENT_KEY);
    localStorage.removeItem('oc:recent-templates');
  } catch {
    /* storage blocked */
  }
}
