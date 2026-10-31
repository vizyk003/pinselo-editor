/**
 * Uploads library and trash (g19, g22, g87): pure helpers shared by the dashboard Uploads and Trash pages and the
 * editor's Uploads panel. No React or API imports, so they stay unit-testable.
 */

export const UPLOAD_TYPES = ['images', 'videos', 'audio', 'files'] as const;
export const UPLOAD_SORTS = ['newest', 'oldest', 'name', 'size'] as const;
export type UploadType = (typeof UPLOAD_TYPES)[number];
export type UploadSort = (typeof UPLOAD_SORTS)[number];
export type UploadCounts = Record<'all' | UploadType, number>;

export interface UploadFilters { q: string; type: 'all' | UploadType; sort: UploadSort }
export const DEFAULT_UPLOAD_FILTERS: UploadFilters = { q: '', type: 'all', sort: 'newest' };
export const EMPTY_COUNTS: UploadCounts = { all: 0, images: 0, videos: 0, audio: 0, files: 0 };

/** Days a trashed design or upload stays restorable (the worker's 30-day purge). */
export const TRASH_DAYS = 30;
/** Most ids POST /api/assets/batch takes at once. */
export const BATCH_MAX = 500;
/** Rows per page on the Uploads and Trash pages. */
export const PAGE_SIZE = 60;

/** Query string for GET /api/assets(/trash); defaults are left out so URLs stay short. */
export function uploadParams(f: Partial<UploadFilters> & { limit?: number; offset?: number; id?: string }): URLSearchParams {
  const p = new URLSearchParams();
  if (f.id) p.set('id', f.id);
  if (f.q?.trim()) p.set('q', f.q.trim());
  if (f.type && f.type !== 'all') p.set('type', f.type);
  if (f.sort && f.sort !== 'newest') p.set('sort', f.sort);
  if (f.limit) p.set('limit', String(f.limit));
  if (f.offset) p.set('offset', String(f.offset));
  return p;
}

/** The type an upload falls under. Animated GIFs are stored as looping video but are images to people; SVGs are images. */
export function uploadTypeOf(a: { kind: string; animated?: boolean | null }): UploadType {
  if (a.kind === 'image' || a.kind === 'svg' || (a.kind === 'video' && a.animated)) return 'images';
  if (a.kind === 'video') return 'videos';
  if (a.kind === 'audio') return 'audio';
  return 'files';
}

/** Whole days until a trashed item is purged (0 on its last day). */
export function daysLeft(trashedAt: string | Date, now: number = Date.now(), days = TRASH_DAYS): number {
  const purge = new Date(trashedAt).getTime() + days * 86_400_000;
  if (!Number.isFinite(purge)) return days;
  return Math.max(0, Math.ceil((purge - now) / 86_400_000));
}

/** "1.4 MB": binary units, one decimal from KB up, the number in the locale's digits and separators ("1,4 MB" in German). */
export function fmtBytes(n: number, locale = 'en'): string {
  const num = (v: number, digits: number) => new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(v);
  if (!(n > 0)) return `${num(0, 0)} B`;
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return `${num(i === 0 || v >= 100 ? Math.round(v) : Math.round(v * 10) / 10, i === 0 || v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** "1:05" or "1:02:03" for media durations. */
export function fmtDuration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

const UPLOADABLE_MIME = /^(image\/(png|jpeg|webp|gif|svg\+xml)|video\/|audio\/)/;
const UPLOADABLE_EXT = /\.(png|jpe?g|webp|gif|svg|mp4|m4v|mov|webm|mp3|m4a|wav|ogg|oga|flac)$/i;

/** Files the upload pipeline accepts (the server sniffs the bytes; this only skips junk such as .DS_Store in a folder). */
export function isUploadable(f: { name: string; type: string }): boolean {
  if (f.name.startsWith('.')) return false;
  return UPLOADABLE_MIME.test(f.type) || UPLOADABLE_EXT.test(f.name);
}

/** Split a list into runs of at most `n`. */
export function chunk<T>(list: T[], n: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < list.length; i += n) out.push(list.slice(i, i + n));
  return out;
}

/** Add or remove one id. */
export function toggleId(sel: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(sel);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/** Selection state of the "select all" checkbox for the visible ids. */
export function selectionState(sel: ReadonlySet<string>, ids: string[]): 'none' | 'some' | 'all' {
  const n = ids.filter((id) => sel.has(id)).length;
  return n === 0 ? 'none' : n === ids.length ? 'all' : 'some';
}

/** Keep only selected ids that are still listed (after a reload or filter change). */
export function pruneSelection(sel: ReadonlySet<string>, ids: string[]): Set<string> {
  const live = new Set(ids);
  return new Set([...sel].filter((id) => live.has(id)));
}

/** Append a further page, dropping rows already shown (rows can shift between pages while things change). */
export function appendPage<T extends { id: string }>(cur: T[], next: T[]): T[] {
  const seen = new Set(cur.map((r) => r.id));
  return [...cur, ...next.filter((r) => !seen.has(r.id))];
}
