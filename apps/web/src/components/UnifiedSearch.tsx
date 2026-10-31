/**
 * Unified search box (g16): one query across designs, folders, uploads and templates, with
 * autocomplete (recent searches kept in this browser + matching template titles, categories and tags),
 * grouped live results and Type / Owner / Date-modified filter chips. ARIA 1.2 combobox: arrows move
 * through the options, Enter opens one (or "See all results"), Escape closes the list, then clears.
 */
import { Clock, FileImage, Folder, Hash, LayoutTemplate, Search, Tag, Users, X } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { assetUrl, templateThumbUrl, thumbUrl } from '../api';
import { useAuth } from '../App';
import { fmtRelative, t, tk } from '../i18n';
import {
  DEFAULT_FILTERS, filtersFromParams, forgetSearch, recentSearches, rememberSearch, scopedType, searchApi, searchParams,
  type SearchFilters, type SearchGroup, type SearchModified, type SearchOwner, type SearchResults, type SearchType, type Suggestion,
} from '../searchApi';
import './unifiedSearch.css';

export type SearchScope = 'all' | 'designs' | 'templates';

export const TYPE_OPTIONS: Array<{ value: SearchType; label: string }> = [
  { value: 'all', label: tk('All types') },
  { value: 'designs', label: tk('Designs') },
  { value: 'folders', label: tk('Folders') },
  { value: 'uploads', label: tk('Uploads') },
  { value: 'templates', label: tk('Templates') },
];
export const OWNER_OPTIONS: Array<{ value: SearchOwner; label: string }> = [
  { value: 'anyone', label: tk('Owned by anyone') },
  { value: 'me', label: tk('Owned by me') },
  { value: 'shared', label: tk('Shared with me') },
];
export const MODIFIED_OPTIONS: Array<{ value: SearchModified; label: string }> = [
  { value: 'any', label: tk('Any time') },
  { value: '1d', label: tk('Today') },
  { value: '7d', label: tk('Last 7 days') },
  { value: '30d', label: tk('Last 30 days') },
  { value: '365d', label: tk('Last year') },
];
export const GROUP_LABELS: Record<SearchGroup, string> = { designs: tk('Designs'), folders: tk('Folders'), uploads: tk('Uploads'), templates: tk('Templates') };
const GROUPS_FOR_SCOPE: Record<SearchScope, SearchGroup[]> = {
  all: ['designs', 'folders', 'uploads', 'templates'],
  designs: ['designs'],
  templates: ['templates'],
};
const PLACEHOLDER: Record<SearchScope, string> = {
  all: tk('Search designs, folders, uploads and templates'),
  designs: tk('Search your designs'),
  templates: tk('Search templates'),
};
const PREVIEW_LIMIT = 4;
const DEBOUNCE_MS = 180;

/** Where a template result opens: the template preview on the Templates page (`?t=`, see lib/templates). */
export const templateHref = (id: string) => `/templates?${new URLSearchParams({ t: id })}`;
/** An upload hit opens the dashboard Uploads page (g19) searched for it, where it can be previewed, downloaded or used. */
export const uploadHref = (u: { name: string | null }) => (u.name ? `/uploads?q=${encodeURIComponent(u.name)}` : '/uploads');
export const uploadThumb = (u: { id: string; kind: string }) => (u.kind === 'video' ? `${assetUrl(u.id)}/poster` : u.kind === 'audio' ? null : assetUrl(u.id));

/** "See all" target for a query: the unified results page, or the templates page for a templates-only box. */
export function seeAllHref(scope: SearchScope, q: string, f: SearchFilters): string {
  if (scope === 'templates') return `/templates?${new URLSearchParams(q.trim() ? { q: q.trim() } : {})}`;
  return `/search?${searchParams(q, { ...f, type: scopedType(scope, f.type) })}`;
}

interface Option {
  key: string;
  group: 'recent' | 'suggest' | SearchGroup | 'all';
  label: string;
  sub?: string;
  icon?: ReactNode;
  thumb?: string | null;
  run: () => void;
  remove?: () => void;
}

/** A filter chip: a native select styled as a chip (keyboard and screen-reader friendly for free). */
export function FilterChip<T extends string>({ label, value, options, onChange, defaultValue }: {
  label: string; value: T; defaultValue: T; options: Array<{ value: T; label: string }>; onChange: (v: T) => void;
}) {
  return (
    <label className={`chip us-chip ${value !== defaultValue ? 'on' : ''}`}>
      <span className="sr-only">{label}</span>
      <select value={value} onChange={(e) => onChange(e.target.value as T)} aria-label={label}>
        {options.map((o) => <option key={o.value} value={o.value}>{t(o.label)}</option>)}
      </select>
    </label>
  );
}

export function UnifiedSearch({ scope, autoFocus }: { scope: 'all' | 'designs' | 'templates'; autoFocus?: boolean }) {
  const nav = useNavigate();
  const location = useLocation();
  // Recent searches are kept per account (none signed out).
  const me = useAuth().user?.id ?? null;
  const uid = useId();
  const listId = `${uid}-list`;
  const wrap = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  // On the results page the box starts with the page's query and filters.
  const onResultsPage = location.pathname === '/search';
  const urlParams = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const [q, setQ] = useState(() => (onResultsPage ? urlParams.get('q') ?? '' : ''));
  const [filters, setFilters] = useState<SearchFilters>(() => (onResultsPage ? filtersFromParams(urlParams) : DEFAULT_FILTERS));
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [recent, setRecent] = useState<string[]>(() => recentSearches(me));
  const [results, setResults] = useState<SearchResults | null>(null);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const effective: SearchFilters = { ...filters, type: scopedType(scope, filters.type) };
  const query = q.trim();

  useEffect(() => {
    if (onResultsPage) {
      setQ(urlParams.get('q') ?? '');
      setFilters(filtersFromParams(urlParams));
    }
  }, [onResultsPage, urlParams]);

  // Live results and suggestions, debounced; a newer keystroke aborts the older request.
  useEffect(() => {
    if (!open) return;
    const filtered = effective.owner !== 'anyone' || effective.modified !== 'any' || (scope === 'all' && effective.type !== 'all');
    if (!query && !filtered) {
      setResults(null);
      setSuggestions([]);
      setLoading(false);
      return;
    }
    const ac = new AbortController();
    setLoading(true);
    const timer = window.setTimeout(() => {
      const wantSuggest = query && scope !== 'designs';
      Promise.all([
        searchApi.search(query, effective, PREVIEW_LIMIT, ac.signal),
        wantSuggest ? searchApi.suggest(query, ac.signal).then((r) => r.suggestions) : Promise.resolve([] as Suggestion[]),
      ]).then(([r, s]) => {
        setResults(r);
        setSuggestions(s);
        setError(false);
        setLoading(false);
      }).catch((e: unknown) => {
        if ((e as Error).name === 'AbortError') return;
        setError(true);
        setLoading(false);
      });
    }, DEBOUNCE_MS);
    return () => {
      ac.abort();
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, query, scope, effective.type, effective.owner, effective.modified, effective.surface]);

  const close = useCallback(() => {
    setOpen(false);
    setActive(-1);
  }, []);

  const go = useCallback((href: string) => {
    if (query) {
      rememberSearch(query, me);
      setRecent(recentSearches(me));
    }
    close();
    input.current?.blur();
    nav(href);
  }, [query, close, nav, me]);

  const pickText = useCallback((text: string) => {
    setQ(text);
    setActive(-1);
    setOpen(true);
    input.current?.focus();
  }, []);

  const options: Option[] = useMemo(() => {
    const out: Option[] = [];
    if (!query) {
      for (const r of recent.slice(0, 6)) {
        out.push({
          key: `recent:${r}`, group: 'recent', label: r, icon: <Clock size={16} />, run: () => pickText(r),
          remove: () => { forgetSearch(r, me); setRecent(recentSearches(me)); },
        });
      }
      // A real option (not a header button) so keyboard and screen-reader users can clear the list too.
      if (recent.length) out.push({ key: 'recent:clear', group: 'recent', label: t('Clear recent searches'), icon: <X size={16} />, run: () => { forgetSearch(null, me); setRecent([]); } });
    }
    for (const s of suggestions.slice(0, 5)) {
      out.push({
        key: `suggest:${s.kind}:${s.text}`, group: 'suggest', label: s.text,
        sub: s.kind === 'category' ? t('Template category') : s.kind === 'tag' ? t('Tag') : t('Template'),
        icon: s.kind === 'category' ? <Hash size={16} /> : s.kind === 'tag' ? <Tag size={16} /> : <LayoutTemplate size={16} />,
        run: () => (s.kind === 'template' && s.id ? go(templateHref(s.id)) : pickText(s.text)),
      });
    }
    if (results) {
      for (const g of GROUPS_FOR_SCOPE[scope]) {
        if (g === 'designs') for (const d of results.designs) {
          out.push({ key: `d:${d.id}`, group: g, label: d.title, sub: t('Edited {when}', { when: fmtRelative(d.updated_at) }), thumb: thumbUrl(d.id, d.updated_at), run: () => go(`/design/${d.id}`) });
        }
        if (g === 'folders') for (const f of results.folders) {
          out.push({ key: `f:${f.id}`, group: g, label: f.name, sub: f.team ?? undefined, icon: f.team ? <Users size={16} /> : <Folder size={16} />, run: () => go(`/projects/${f.id}`) });
        }
        if (g === 'uploads') for (const u of results.uploads) {
          out.push({
            key: `u:${u.id}`, group: g, label: u.name || t('Untitled upload'), sub: fmtRelative(u.created_at), thumb: uploadThumb(u), icon: <FileImage size={16} />,
            run: () => go(uploadHref(u)),
          });
        }
        if (g === 'templates') for (const tpl of results.templates) {
          out.push({ key: `t:${tpl.id}`, group: g, label: tpl.title, sub: tpl.category, thumb: templateThumbUrl(tpl.id), run: () => go(templateHref(tpl.id)) });
        }
      }
    }
    if (query || results) out.push({ key: 'all', group: 'all', label: query ? t('See all results for “{q}”', { q: query }) : t('See all results'), icon: <Search size={16} />, run: () => go(seeAllHref(scope, query, effective)) });
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, recent, suggestions, results, scope, go, pickText, close, effective.type, effective.owner, effective.modified]);

  useEffect(() => {
    if (active >= options.length) setActive(options.length - 1);
  }, [active, options.length]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (!open) setOpen(true);
      setActive((i) => (options.length ? (i + 1) % options.length : -1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (!open) setOpen(true);
      setActive((i) => (options.length ? (i <= 0 ? options.length - 1 : i - 1) : -1));
    } else if (e.key === 'Home' && open && active >= 0) {
      e.preventDefault();
      setActive(0);
    } else if (e.key === 'End' && open && active >= 0) {
      e.preventDefault();
      setActive(options.length - 1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const opt = open && active >= 0 ? options[active] : undefined;
      if (opt) opt.run();
      else if (query) go(seeAllHref(scope, query, effective));
    } else if (e.key === 'Escape') {
      if (open) {
        e.preventDefault();
        close();
      } else if (q) {
        e.preventDefault();
        setQ('');
      }
    } else if (e.key === 'Delete' && e.shiftKey && open && options[active]?.remove) {
      // Shift+Delete removes the highlighted recent search.
      e.preventDefault();
      options[active]!.remove!();
    }
  };

  // Keep the highlighted option in view while arrowing.
  useEffect(() => {
    if (active < 0) return;
    document.getElementById(`${uid}-opt-${active}`)?.scrollIntoView({ block: 'nearest' });
  }, [active, uid]);

  const setFilter = (patch: Partial<SearchFilters>) => {
    setFilters((f) => ({ ...f, ...patch }));
    setActive(-1);
  };

  const groupTitle = (g: Option['group']): string | null =>
    g === 'recent' ? t('Recent searches') : g === 'suggest' ? t('Suggestions') : g === 'all' ? null : t(GROUP_LABELS[g]);

  // Render options in their groups, keeping one flat index for aria-activedescendant.
  const sections: Array<{ group: Option['group']; items: Array<{ opt: Option; index: number }> }> = [];
  options.forEach((opt, index) => {
    const last = sections[sections.length - 1];
    if (last && last.group === opt.group) last.items.push({ opt, index });
    else sections.push({ group: opt.group, items: [{ opt, index }] });
  });
  const empty = !loading && results && !results.designs.length && !results.folders.length && !results.uploads.length && !results.templates.length && !suggestions.length;
  const showPanel = open && (options.length > 0 || loading || error || empty || scope === 'all');

  return (
    <div
      ref={wrap}
      className={`us ${showPanel ? 'open' : ''}`}
      onBlur={(e) => {
        if (!wrap.current?.contains(e.relatedTarget as Node | null)) close();
      }}
    >
      <div className="search us-box">
        <Search size={18} aria-hidden />
        <input
          ref={input}
          role="combobox"
          aria-expanded={showPanel}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={open && active >= 0 ? `${uid}-opt-${active}` : undefined}
          aria-describedby={recent.length ? `${uid}-hint` : undefined}
          aria-label={t(PLACEHOLDER[scope])}
          placeholder={t(PLACEHOLDER[scope])}
          autoFocus={autoFocus}
          autoComplete="off"
          spellCheck={false}
          value={q}
          onChange={(e) => {
            setQ(e.target.value);
            setActive(-1);
            setOpen(true);
          }}
          onFocus={() => {
            setRecent(recentSearches(me));
            setOpen(true);
          }}
          onKeyDown={onKeyDown}
        />
        {q && (
          <button type="button" className="us-x us-clear" aria-label={t('Clear search')} onClick={() => { setQ(''); input.current?.focus(); }}>
            <X size={16} />
          </button>
        )}
      </div>
      {recent.length > 0 && <span id={`${uid}-hint`} className="sr-only">{t('Press Shift+Delete to remove the highlighted recent search.')}</span>}
      {showPanel && (
        <div className="us-panel">
          <div className="chips us-filters" role="group" aria-label={t('Filters')}>
            {scope === 'all' && <FilterChip label={t('Type')} value={filters.type} defaultValue="all" options={TYPE_OPTIONS} onChange={(v) => setFilter({ type: v })} />}
            {scope !== 'templates' && <FilterChip label={t('Owner')} value={filters.owner} defaultValue="anyone" options={OWNER_OPTIONS} onChange={(v) => setFilter({ owner: v })} />}
            <FilterChip label={t('Date modified')} value={filters.modified} defaultValue="any" options={MODIFIED_OPTIONS} onChange={(v) => setFilter({ modified: v })} />
          </div>
          <ul id={listId} role="listbox" className="us-list" aria-label={t('Search results')}>
            {sections.map((s) => {
              const title = groupTitle(s.group);
              const headId = `${uid}-g-${s.group}`;
              const items = s.items.map(({ opt, index }) => (
                <li
                  key={opt.key}
                  id={`${uid}-opt-${index}`}
                  role="option"
                  aria-selected={index === active}
                  className={`us-opt ${index === active ? 'active' : ''} ${opt.group === 'all' ? 'see-all' : ''}`}
                  onMouseDown={(e) => e.preventDefault()}
                  onMouseMove={() => setActive(index)}
                  onClick={() => opt.run()}
                >
                  <span className="us-ic">{opt.thumb ? <img src={opt.thumb} alt="" loading="lazy" /> : opt.icon}</span>
                  <span className="us-txt">
                    <span className="us-label">{opt.label}</span>
                    {opt.sub && <span className="us-sub">{opt.sub}</span>}
                  </span>
                  {opt.remove && (
                    <button type="button" tabIndex={-1} className="us-x us-remove" aria-label={t('Remove “{q}” from recent searches', { q: opt.label })} onClick={(e) => { e.stopPropagation(); opt.remove!(); }}>
                      <X size={14} />
                    </button>
                  )}
                </li>
              ));
              // The group itself is the listbox child (named by its heading); the heading is visual only.
              return title ? (
                <li key={s.group} role="group" aria-label={title}>
                  <div id={headId} className="us-head" aria-hidden>{title}</div>
                  <ul role="presentation" className="us-group">{items}</ul>
                </li>
              ) : items;
            })}
          </ul>
          <div className="us-status" role="status" aria-live="polite">
            {loading ? t('Searching…') : error ? t('Search is unavailable right now.') : empty ? (query ? t('No results for “{q}”', { q: query }) : t('Nothing matches these filters.')) : !options.length ? t('Type to search, or pick a filter.') : ''}
          </div>
        </div>
      )}
    </div>
  );
}
