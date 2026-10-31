/** Unified search results page (g16): /search?q=&type=&owner=&modified=&surface= with filter chips and grouped sections. */
import { FileAudio, Folder, Search, Users } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router';
import type { Folder as FolderT } from '../../api';
import { useAuth } from '../../App';
import { FilterChip, GROUP_LABELS, MODIFIED_OPTIONS, OWNER_OPTIONS, TYPE_OPTIONS, UnifiedSearch, uploadHref, uploadThumb } from '../../components/UnifiedSearch';
import { t, tk, tn } from '../../i18n';
import { filtersFromParams, rememberSearch, searchApi, searchParams, type SearchFilters, type SearchResults } from '../../searchApi';
import { DesignGrid } from './Designs';
import { EmptyState } from '../../components/ui';
import { TemplateCard } from './Templates';

const SURFACE_OPTIONS: Array<{ value: string; label: string }> = [
  { value: '', label: tk('All categories') },
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
const PAGE_LIMIT = 60;

export function SearchResultsPage({ folders }: { folders: FolderT[] }) {
  const nav = useNavigate();
  const me = useAuth().user?.id ?? null;
  const [params, setParams] = useSearchParams();
  const q = (params.get('q') ?? '').trim();
  const filters = useMemo(() => filtersFromParams(params), [params]);
  const [res, setRes] = useState<SearchResults | null>(null);
  const [error, setError] = useState(false);

  const load = useCallback(() => {
    setError(false);
    return searchApi.search(q, filters, PAGE_LIMIT).then(setRes, () => setError(true));
  }, [q, filters]);

  useEffect(() => {
    setRes(null);
    void load();
    if (q) rememberSearch(q, me);
  }, [load, q, me]);

  const setFilter = (patch: Partial<SearchFilters>) => {
    const next = searchParams(q, { ...filters, ...patch });
    // A category (design type) only applies to designs.
    if (patch.surface === '') next.delete('surface');
    setParams(next, { replace: true });
  };

  const total = res ? res.designs.length + res.folders.length + res.uploads.length + res.templates.length : 0;
  const show = (g: 'designs' | 'folders' | 'uploads' | 'templates') => filters.type === 'all' || filters.type === g;

  return (
    <>
      <div className="topbar">
        <UnifiedSearch scope="all" />
      </div>
      <div className="content">
        <section style={{ display: 'grid', gap: 12 }}>
          <h1 className="page-title">{q ? t('Results for “{q}”', { q }) : t('Search')}</h1>
          <div className="chips us-page-filters" role="group" aria-label={t('Filters')}>
            <FilterChip label={t('Type')} value={filters.type} defaultValue="all" options={TYPE_OPTIONS} onChange={(v) => setFilter({ type: v })} />
            <FilterChip label={t('Category')} value={filters.surface ?? ''} defaultValue="" options={SURFACE_OPTIONS} onChange={(v) => setFilter({ surface: v })} />
            <FilterChip label={t('Owner')} value={filters.owner} defaultValue="anyone" options={OWNER_OPTIONS} onChange={(v) => setFilter({ owner: v })} />
            <FilterChip label={t('Date modified')} value={filters.modified} defaultValue="any" options={MODIFIED_OPTIONS} onChange={(v) => setFilter({ modified: v })} />
            {/* Always mounted so screen readers announce the new count when a filter changes. */}
            <span className="faint" role="status" aria-live="polite">{res ? tn(total, '{n} result', '{n} results') : error ? '' : t('Searching…')}</span>
          </div>
        </section>

        {error && <div className="empty">{t('Search is unavailable right now.')}</div>}
        {!error && !res && <div className="empty">{t('Searching…')}</div>}
        {res && total === 0 && <EmptyState icon={<Search size={22} />}>{q ? t('No results for “{q}”. Try another word or fewer filters.', { q }) : t('Nothing matches these filters.')}</EmptyState>}

        {res && show('designs') && res.designs.length > 0 && (
          <section>
            <div className="section-head"><h2>{t(GROUP_LABELS.designs)}</h2></div>
            <DesignGrid designs={res.designs} reload={() => void load()} folders={folders} empty="" />
          </section>
        )}

        {res && show('folders') && res.folders.length > 0 && (
          <section>
            <div className="section-head"><h2>{t(GROUP_LABELS.folders)}</h2></div>
            <div className="grid" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))' }}>
              {res.folders.map((f) => (
                <button key={f.id} className="btn" style={{ height: 56, justifyContent: 'flex-start' }} onClick={() => nav(`/projects/${f.id}`)}>
                  {f.team ? <Users size={20} /> : <Folder size={20} />} <span className="grow" style={{ textAlign: 'left' }}>{f.name}{f.team && <span className="faint" style={{ display: 'block', fontSize: 11, fontWeight: 400 }}>{f.team}</span>}</span> <span className="faint">{f.designs}</span>
                </button>
              ))}
            </div>
          </section>
        )}

        {res && show('uploads') && res.uploads.length > 0 && (
          <section>
            <div className="section-head"><h2>{t(GROUP_LABELS.uploads)}</h2></div>
            <div className="us-uploads">
              {res.uploads.map((u) => {
                const src = uploadThumb(u);
                return (
                  <button key={u.id} className="us-upload" onClick={() => nav(uploadHref(u))} title={t('Open “{name}”', { name: u.name || t('Untitled upload') })}>
                    <div className="img">{src ? <img src={src} alt="" loading="lazy" /> : <FileAudio size={28} />}</div>
                    <div className="t">{u.name || t('Untitled upload')}</div>
                  </button>
                );
              })}
            </div>
          </section>
        )}

        {res && show('templates') && res.templates.length > 0 && (
          <section>
            <div className="section-head"><h2>{t(GROUP_LABELS.templates)}</h2></div>
            <div className="grid" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))' }}>
              {res.templates.map((tpl) => <TemplateCard key={tpl.id} t={tpl} />)}
            </div>
          </section>
        )}
      </div>
    </>
  );
}
