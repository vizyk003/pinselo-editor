import { Crown } from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { useSearchParams } from 'react-router';
import { plansEnabled, templateThumbUrl, type TemplateSummary } from '../../api';
import { t, tk, tn } from '../../i18n';
import { useAuth } from '../../App';
import { TemplatePreviewModal } from '../../components/TemplatePreview';
import { PageHeader, ScrollRow, SearchField } from '../../components/ui';
import {
  fetchTemplates, groupByCategory, parseTemplateFilters, recentTemplates, templateFiltersToParams, toggleTag,
  type TemplateFacets, type TemplateFilters, type TemplateView,
} from '../../lib/templates';

/**
 * A template thumbnail. Clicking opens the preview (g25) — its own dialog, or `onPreview` when the page keeps the
 * open template in the URL. `masonry` keeps the template's shape instead of a fixed-height box.
 */
export function TemplateCard({ t: tpl, onPreview, masonry }: { t: TemplateSummary; onPreview?: (tpl: TemplateSummary) => void; masonry?: boolean }) {
  const [open, setOpen] = useState<TemplateSummary | null>(null);
  const ratio = Math.max(0.5, Math.min(2, tpl.width / tpl.height));
  return (
    <>
      <button
        className={`tpl-card${masonry ? ' masonry' : ''}`}
        onClick={() => (onPreview ? onPreview(tpl) : setOpen(tpl))}
        style={{ background: 'none', border: 0, padding: 0, textAlign: 'left' }}
        title={t('Preview “{title}”', { title: tpl.title })}
      >
        <div className="img" style={{ position: 'relative', ...(masonry ? { aspectRatio: String(ratio), height: 'auto' } : {}) }}>
          <img src={templateThumbUrl(tpl.id)} alt="" loading="lazy" />
          {tpl.premium && plansEnabled() && <span className="premium-badge" title={t('Premium template (Pro)')}><Crown size={11} /> {t('PRO')}</span>}
          {tpl.team && <span className="premium-badge team" title={t('Your team’s template')}>{t('TEAM')}</span>}
        </div>
        <div className="t">{tpl.title}</div>
      </button>
      {open && <TemplatePreviewModal id={open.id} initial={open} onClose={() => setOpen(null)} onOpen={setOpen} />}
    </>
  );
}

const VIEWS: Array<{ id: TemplateView; label: string; signedIn?: boolean }> = [
  { id: 'all', label: tk('Templates') },
  { id: 'team', label: tk('Your team'), signedIn: true },
  { id: 'recent', label: tk('Recently used'), signedIn: true },
];

/**
 * Templates hub (g23/g24): sub-nav, category tiles and "Discover" rows per category when nothing is filtered;
 * category and tag facets with masonry results otherwise. Every filter lives in the URL (?q=&category=&tag=&view=&t=).
 */
export function TemplatesBrowser({ searchInTopbar }: { searchInTopbar?: (input: ReactNode) => ReactNode }) {
  const { user } = useAuth();
  const [params, setParams] = useSearchParams();
  const f = parseTemplateFilters(params);
  const [qInput, setQInput] = useState(f.q);
  const [data, setData] = useState<TemplateFacets | null>(null);
  const [recent] = useState(() => recentTemplates(user?.id));

  const update = (patch: Partial<TemplateFilters>, replace = false) => setParams(templateFiltersToParams({ ...f, ...patch }), { replace });
  // Typing in the search box updates ?q= after a pause (replacing the history entry).
  useEffect(() => setQInput(f.q), [f.q]);
  useEffect(() => {
    if (qInput === f.q) return;
    const timer = setTimeout(() => update({ q: qInput }, true), 250);
    return () => clearTimeout(timer);
  });

  const tagKey = f.tags.join(',');
  useEffect(() => {
    if (f.view === 'recent') return;
    let live = true;
    fetchTemplates({ q: f.q, category: f.category, tags: f.tags, scope: f.view === 'team' ? 'team' : 'all' })
      .then((d) => live && setData(d))
      .catch(() => live && setData({ templates: [], categories: [], tags: [] }));
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [f.q, f.category, tagKey, f.view]);

  const list = useMemo(() => {
    if (f.view !== 'recent') return data?.templates ?? null;
    const q = f.q.toLowerCase();
    return recent.filter((x) => (!q || `${x.title} ${x.category} ${(x.tags ?? []).join(' ')}`.toLowerCase().includes(q))
      && (!f.category || x.category === f.category) && f.tags.every((tag) => x.tags?.includes(tag)));
  }, [data, f.view, f.q, f.category, tagKey, recent]); // eslint-disable-line react-hooks/exhaustive-deps

  const browsing = f.view === 'all' && !f.q && !f.category && !f.tags.length;
  const groups = useMemo(() => (browsing && data ? groupByCategory(data.templates, 12) : []), [browsing, data]);
  const covers = useMemo(() => new Map(groups.map((g) => [g.category, g.templates[0]!])), [groups]);
  const preview = f.preview ? (list?.find((x) => x.id === f.preview) ?? data?.templates.find((x) => x.id === f.preview)) : undefined;
  const open = (tpl: TemplateSummary) => update({ preview: tpl.id });

  // Selected tags stay visible even when the narrowed result set no longer counts them.
  const facetTags = [...f.tags.map((tag) => ({ tag, n: 0 })), ...(data?.tags ?? []).filter((x) => !f.tags.includes(x.tag)).slice(0, 20)];
  const categories = f.view === 'recent' ? [...new Set(recent.map((x) => x.category))].map((category) => ({ category, n: recent.filter((x) => x.category === category).length })) : data?.categories ?? [];

  const search = (
    <SearchField value={qInput} onChange={setQInput} placeholder={t('Search templates')} onClear={() => setQInput('')} />
  );
  return (
    <>
      {searchInTopbar?.(search)}
      <div className="content tpl-hub">
        <nav className="tpl-subnav" aria-label={t('Templates')}>
          {VIEWS.filter((v) => !v.signedIn || user).map((v) => (
            <button key={v.id} className={f.view === v.id ? 'on' : ''} aria-current={f.view === v.id ? 'page' : undefined} onClick={() => update({ view: v.id, category: '', tags: [] })}>
              {t(v.label)}
            </button>
          ))}
          {!searchInTopbar && <><span className="grow" />{search}</>}
        </nav>

        {browsing ? (
          <>
            {!data && <div className="spinner" />}
            {data && categories.length > 0 && (
              <section>
                <div className="section-head"><h2>{t('Browse by category')}</h2></div>
                <div className="tpl-cat-grid">
                  {[...categories].sort((a, b) => b.n - a.n).map((c) => {
                    const cover = covers.get(c.category);
                    return (
                      <button key={c.category} className="tpl-cat-tile" onClick={() => update({ category: c.category })}>
                        <span className="cover">{cover && <img src={templateThumbUrl(cover.id)} alt="" loading="lazy" />}</span>
                        <span className="name">{t(c.category)}</span>
                        <span className="faint">{c.n}</span>
                      </button>
                    );
                  })}
                </div>
              </section>
            )}
            {groups.map((g) => (
              <section key={g.category}>
                <div className="section-head">
                  <h2>{t('Discover {category}', { category: t(g.category) })}</h2>
                  {g.n > g.templates.length && <button className="btn ghost sm" onClick={() => update({ category: g.category })}>{t('See all')}</button>}
                </div>
                <ScrollRow className="tpl-row">
                  {g.templates.map((tpl) => <TemplateCard key={tpl.id} t={tpl} onPreview={open} />)}
                </ScrollRow>
              </section>
            ))}
          </>
        ) : (
          <>
            <div className="chips">
              <button className={`chip ${f.category === '' ? 'active' : ''}`} aria-pressed={f.category === ''} onClick={() => update({ category: '' })}>{t('All')}</button>
              {categories.map((c) => (
                <button key={c.category} className={`chip ${f.category === c.category ? 'active' : ''}`} aria-pressed={f.category === c.category} onClick={() => update({ category: f.category === c.category ? '' : c.category })}>
                  {t(c.category)} <span className="faint">{c.n}</span>
                </button>
              ))}
            </div>
            {facetTags.length > 0 && f.view !== 'recent' && (
              <div className="chips tpl-tags" aria-label={t('Tags')}>
                {facetTags.map((x) => (
                  <button key={x.tag} className={`chip ${f.tags.includes(x.tag) ? 'on' : ''}`} aria-pressed={f.tags.includes(x.tag)} onClick={() => update({ tags: toggleTag(f.tags, x.tag) })}>
                    #{x.tag}{x.n > 0 && <span className="faint"> {x.n}</span>}
                  </button>
                ))}
                {f.tags.length > 0 && <button className="btn ghost sm" onClick={() => update({ tags: [] })}>{t('Clear tags')}</button>}
              </div>
            )}
            {list && list.length > 0 && <div className="faint">{tn(list.length, '{n} template', '{n} templates')}</div>}
            {!list && <div className="spinner" />}
            <div className="tpl-masonry">
              {list?.map((tpl) => <TemplateCard key={tpl.id} t={tpl} onPreview={open} masonry />)}
            </div>
            {list && !list.length && (
              <div className="empty">{f.view === 'recent' && !recent.length ? t('Templates you use will appear here.') : t('No templates match.')}</div>
            )}
          </>
        )}
      </div>
      {f.preview && (
        <TemplatePreviewModal id={f.preview} initial={preview} onClose={() => update({ preview: null })} onOpen={open} />
      )}
    </>
  );
}

export function TemplatesPage() {
  return (
    <TemplatesBrowser
      searchInTopbar={(search) => (
        <PageHeader title={t('Templates')}>
          <div className="grow" />
          {search}
        </PageHeader>
      )}
    />
  );
}
