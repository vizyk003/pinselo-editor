/** Home sections below the hero (g07): template category tiles and popular size cards. */
import { formatDims, SIZE_PRESETS } from '@oc/engine';
import { NavLink } from 'react-router';
import { templateThumbUrl, type TemplateSummary } from '../../api';
import { ScrollRow } from '../../components/ui';
import { t, tn } from '../../i18n';

/** Popular sizes, in Canva's Home order. Ids missing from SIZE_PRESETS are skipped. */
export const POPULAR_SIZES = ['ig-post', 'presentation', 'ig-story', 'yt-thumb', 'fb-post', 'a4', 'poster', 'flyer', 'logo', 'business-card'];

/** 'Browse template categories': one tile per category, illustrated by its first template. */
export function CategoryTiles({ categories, templates }: { categories: Array<{ category: string; n: number }>; templates: TemplateSummary[] }) {
  if (!categories.length) return null;
  return (
    <section>
      <div className="section-head">
        <h2>{t('Browse template categories')}</h2>
        <NavLink to="/templates" className="btn ghost sm">{t('See all')}</NavLink>
      </div>
      <ScrollRow className="cat-tiles">
        {categories.map((c) => {
          const sample = templates.find((x) => x.category === c.category);
          return (
            <NavLink key={c.category} className="cat-tile" to={`/templates?category=${encodeURIComponent(c.category)}`}>
              <span className="img">{sample && <img src={templateThumbUrl(sample.id)} alt="" loading="lazy" />}</span>
              <span className="t">{t(c.category)}</span>
              <span className="faint">{tn(c.n, '{n} template', '{n} templates')}</span>
            </NavLink>
          );
        })}
      </ScrollRow>
    </section>
  );
}

/** 'Popular sizes': a card per size showing its proportions. */
export function PopularSizes({ onPreset }: { onPreset: (preset: string) => void }) {
  const list = POPULAR_SIZES.map((id) => SIZE_PRESETS.find((p) => p.id === id)).filter((p) => p !== undefined);
  return (
    <section>
      <div className="section-head"><h2>{t('Popular sizes')}</h2></div>
      <div className="size-cards">
        {list.map((p) => {
          const k = 56 / Math.max(p.w, p.h);
          return (
            <button key={p.id} className="size-card" onClick={() => onPreset(p.id)} title={t('Create a {size}', { size: t(p.label) })}>
              <span className="shape"><span style={{ width: Math.max(8, p.w * k), height: Math.max(8, p.h * k) }} /></span>
              <span className="t">{t(p.label)}</span>
              <span className="faint">{formatDims(p.w, p.h, p.unit)}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
