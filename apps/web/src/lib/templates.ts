/**
 * Template browsing helpers (docs/plan/11-canva-gap-analysis.md g23–g26): the preview and faceted-list API calls,
 * "Recently used" templates (per signed-in user on this browser), recommendations for the open design and the URL form of the browse filters.
 * The logic below the fetch calls is pure (storage is injected) so it runs under vitest in Node.
 */
import { SIZE_PRESETS, type DesignDoc, type Surface } from '@oc/engine';
import type { TemplateSummary } from '../api';

// ------------------------------------------------------------------ API

export interface TemplateFacets { templates: TemplateSummary[]; categories: Array<{ category: string; n: number }>; tags: Array<{ tag: string; n: number }> }

export interface TemplateDetails extends TemplateSummary {
  description: string | null;
  uses: number;
  creator: string | null;
  teamName: string | null;
  /** Visible pages in the template. */
  pages: number;
  surface: Surface | null;
  unit: string | null;
}

export interface TemplatePreviewData { template: TemplateDetails; doc: DesignDoc | null; locked: boolean; similar: TemplateSummary[] }

async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(url, { credentials: 'same-origin' });
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  return r.json() as Promise<T>;
}

/** The catalogue with category counts and the tag facet of the current result set. */
export function fetchTemplates(f: { q?: string; category?: string; tags?: string[]; scope?: 'all' | 'team' } = {}): Promise<TemplateFacets> {
  const p = new URLSearchParams();
  if (f.q) p.set('q', f.q);
  if (f.category) p.set('category', f.category);
  if (f.tags?.length) p.set('tag', f.tags.join(','));
  if (f.scope === 'team') p.set('scope', 'team');
  return getJson<Partial<TemplateFacets>>(`/api/templates?${p}`).then((r) => ({ templates: r.templates ?? [], categories: r.categories ?? [], tags: r.tags ?? [] }));
}

export const fetchTemplatePreview = (id: string) => getJson<TemplatePreviewData>(`/api/templates/${encodeURIComponent(id)}/preview`);

/** Shareable link that opens a template's preview on the Templates page. */
export const templateLink = (id: string, origin: string) => `${origin}/templates?t=${encodeURIComponent(id)}`;

// ------------------------------------------------------------------ presets

/** The size preset a template was made for (label, unit, surface), if it is a known one. */
export const presetOf = (preset: string) => SIZE_PRESETS.find((p) => p.id === preset);

export function templateSurface(tpl: Pick<TemplateSummary, 'preset'>): Surface | null {
  return presetOf(tpl.preset)?.surface ?? null;
}

// ------------------------------------------------------------------ recently used

export interface KV { getItem(k: string): string | null; setItem(k: string, v: string): void }

export const RECENT_KEY = 'oc:recent-templates';
export const RECENT_MAX = 12;

/** Storage key of one user's list; signed out (no uid) there is none, so nothing leaks between accounts. */
export const recentTemplatesKey = (uid: string | null | undefined) => (uid ? `${RECENT_KEY}:${uid}` : null);

/** localStorage, or null where it is blocked (private windows, sandboxed previews). */
export function browserStorage(): KV | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

export function recentTemplates(uid: string | null | undefined, store: KV | null = browserStorage()): TemplateSummary[] {
  const key = recentTemplatesKey(uid);
  if (!key) return [];
  try {
    const raw = store?.getItem(key);
    const list = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(list) ? list.filter((x): x is TemplateSummary => !!x && typeof x === 'object' && typeof (x as TemplateSummary).id === 'string') : [];
  } catch {
    return [];
  }
}

/** Move a template to the front of the "Recently used" list (deduplicated, capped). */
export function recordRecentTemplate(uid: string | null | undefined, tpl: TemplateSummary, store: KV | null = browserStorage()): TemplateSummary[] {
  const key = recentTemplatesKey(uid);
  if (!key) return [];
  const { id, title, category, preset, tags, width, height, premium, team } = tpl;
  const entry: TemplateSummary = { id, title, category, preset, tags: tags ?? [], width, height, ...(premium ? { premium } : {}), ...(team ? { team } : {}) };
  const next = [entry, ...recentTemplates(uid, store).filter((x) => x.id !== id)].slice(0, RECENT_MAX);
  try {
    store?.setItem(key, JSON.stringify(next));
  } catch {
    /* quota or blocked storage: the list is a convenience */
  }
  return next;
}

// ------------------------------------------------------------------ recommendations

/** Aspect-ratio distance in log space (0 = same shape). */
const ratioDistance = (a: number, b: number) => Math.abs(Math.log(a / b));

/**
 * "More templates for you" in the editor: templates for the same surface and category as the open design
 * (its source template's category, else the categories of recently used templates), closest page shape first.
 */
export function recommendTemplates(
  list: readonly TemplateSummary[],
  design: { surface: Surface; w: number; h: number; category?: string | null },
  opts: { exclude?: Iterable<string>; recentCategories?: readonly string[]; limit?: number } = {},
): TemplateSummary[] {
  const exclude = new Set(opts.exclude ?? []);
  const ratio = design.w / design.h;
  const recentCats = opts.recentCategories ?? [];
  const score = (tpl: TemplateSummary) => {
    let s = 0;
    if (templateSurface(tpl) === design.surface) s += 4;
    if (design.category && tpl.category === design.category) s += 3;
    else if (recentCats.includes(tpl.category)) s += 1;
    s -= Math.min(3, ratioDistance(tpl.width / tpl.height, ratio) * 3);
    return s;
  };
  return list
    .filter((tpl) => !exclude.has(tpl.id))
    .map((tpl) => ({ tpl, s: score(tpl) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s || a.tpl.title.localeCompare(b.tpl.title))
    .slice(0, opts.limit ?? 6)
    .map((x) => x.tpl);
}

// ------------------------------------------------------------------ browse

/** Category rows for the "Discover" carousels: largest categories first, a few templates each. */
export function groupByCategory(list: readonly TemplateSummary[], perGroup = 12): Array<{ category: string; n: number; templates: TemplateSummary[] }> {
  const groups = new Map<string, TemplateSummary[]>();
  for (const tpl of list) {
    const g = groups.get(tpl.category);
    if (g) g.push(tpl);
    else groups.set(tpl.category, [tpl]);
  }
  return [...groups].map(([category, all]) => ({ category, n: all.length, templates: all.slice(0, perGroup) }))
    .sort((a, b) => b.n - a.n || a.category.localeCompare(b.category));
}

export type TemplateView = 'all' | 'team' | 'recent';

export interface TemplateFilters { q: string; category: string; tags: string[]; view: TemplateView; preview: string | null }

/** Read the Templates page filters from its query string (?q=&category=&tag=a,b&view=&t=). */
export function parseTemplateFilters(search: string | URLSearchParams): TemplateFilters {
  const p = typeof search === 'string' ? new URLSearchParams(search) : search;
  const view = p.get('view');
  return {
    q: p.get('q') ?? '',
    category: p.get('category') ?? '',
    tags: [...new Set((p.get('tag') ?? '').split(',').map((s) => s.trim()).filter(Boolean))],
    view: view === 'team' || view === 'recent' ? view : 'all',
    preview: p.get('t') || null,
  };
}

export function templateFiltersToParams(f: Partial<TemplateFilters>): URLSearchParams {
  const p = new URLSearchParams();
  if (f.q) p.set('q', f.q);
  if (f.category) p.set('category', f.category);
  if (f.tags?.length) p.set('tag', f.tags.join(','));
  if (f.view && f.view !== 'all') p.set('view', f.view);
  if (f.preview) p.set('t', f.preview);
  return p;
}

export const toggleTag = (tags: readonly string[], tag: string) => (tags.includes(tag) ? tags.filter((x) => x !== tag) : [...tags, tag]);
