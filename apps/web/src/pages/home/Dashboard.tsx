import type { PresetCategoryId } from '@oc/engine';
import { Plus, Sparkles } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { NavLink, useNavigate, useSearchParams } from 'react-router';
import { api, type DesignSummary, type Folder as FolderT, type TemplateSummary } from '../../api';
import { DesignWithAiCard } from '../../components/ConnectAi';
import { UnifiedSearch } from '../../components/UnifiedSearch';
import { EmptyState, ScrollRow, useToast } from '../../components/ui';
import { t } from '../../i18n';
import { DesignGrid } from './Designs';
import { CategoryTiles, PopularSizes } from './HomeSections';
import { QuickTiles, type QuickAction } from './quickTiles';
import { FileDropArea, IMPORT_ACCEPT, MEDIA_ACCEPT, useFilePicker, useStartFromFile } from './StartFromUpload';
import { TemplateCard } from './Templates';
import { keepIfSame, useLiveRefresh } from '../../lib/liveRefresh';

export { QUICK } from './quickTiles';

/** Recent designs shown per 'Show more' step. */
const PAGE = 12;

export function Dashboard({ onCreate, folders }: { onCreate: (category?: PresetCategoryId) => void; folders: FolderT[] }) {
  const nav = useNavigate();
  const toast = useToast();
  // ?q= filters recent designs (the search box's fallback when it has no results dropdown of its own).
  const [params] = useSearchParams();
  const q = params.get('q') ?? '';
  const [scope, setScope] = useState<'designs' | 'templates'>('designs');
  const [designs, setDesigns] = useState<DesignSummary[] | null>(null);
  const [shown, setShown] = useState(PAGE);
  const [tpl, setTpl] = useState<{ templates: TemplateSummary[]; categories: Array<{ category: string; n: number }> }>({ templates: [], categories: [] });
  const load = useCallback(() => api.designs({ filter: 'recent', ...(q ? { q } : {}) }).then((r) => setDesigns((prev) => keepIfSame(prev, r.designs))), [q]);
  // Designs an AI assistant makes through the connector appear without a reload.
  useLiveRefresh(load);
  useEffect(() => {
    setShown(PAGE);
    void load();
  }, [load]);
  useEffect(() => {
    api.templates().then(setTpl).catch(() => {});
  }, []);
  const create = async (preset: string) => {
    try {
      nav(`/design/${(await api.createDesign({ preset })).id}`);
    } catch (e) {
      toast((e as Error).message);
    }
  };
  // Upload (media or importable files) and Photo editor (media only) start a design from the picked file (g15).
  const { start, status } = useStartFromFile();
  const [busyTile, setBusyTile] = useState<string | null>(null);
  const upload = useFilePicker(`${MEDIA_ACCEPT},${IMPORT_ACCEPT}`, (f) => { setBusyTile('upload'); void start(f); });
  const photo = useFilePicker(MEDIA_ACCEPT, (f) => { setBusyTile('photo'); void start(f, 'media'); });
  const onAction = (a: QuickAction) => {
    if (a === 'custom') onCreate('custom');
    else if (a === 'more') onCreate();
    else if (a === 'upload') upload.open();
    else photo.open();
  };
  return (
    <FileDropArea onFile={(f) => { setBusyTile('upload'); void start(f); }} disabled={Boolean(status)}>
      <div className="content" style={{ paddingTop: 24 }}>
        <section className="hero">
          <h1>{scope === 'templates' ? t('Find a template') : t('What will you design today?')}</h1>
          <div className="segmented home-scope" role="group" aria-label={t('Search in')}>
            <button aria-pressed={scope === 'designs'} className={scope === 'designs' ? 'on' : ''} onClick={() => setScope('designs')}>{t('Your designs')}</button>
            <button aria-pressed={scope === 'templates'} className={scope === 'templates' ? 'on' : ''} onClick={() => setScope('templates')}>{t('Templates')}</button>
          </div>
          <div id="home-search-slot">
            <UnifiedSearch scope={scope} />
          </div>
          <p className="quick-create-label">{t('Start a design')}</p>
          <QuickTiles onPreset={create} onAction={onAction} onSeeAll={onCreate} busy={status && busyTile ? { [busyTile]: status } : undefined} />
          <p className="faint home-drop-tip">{t('Tip: drop a photo, video, PDF or PowerPoint file anywhere here to start a design from it.')}</p>
          {upload.input}
          {photo.input}
        </section>

        {!q && <DesignWithAiCard />}

        {!q && tpl.templates.length > 0 && (
          <section>
            <div className="section-head">
              <h2>{t('Start from a template')}</h2>
              <NavLink to="/templates" className="btn ghost sm">{t('See all')}</NavLink>
            </div>
            <ScrollRow className="tpl-row">
              {tpl.templates.slice(0, 14).map((x) => <TemplateCard key={x.id} t={x} />)}
            </ScrollRow>
          </section>
        )}

        <section>
          <div className="section-head">
            <h2>{q ? t('Results for “{q}”', { q }) : t('Recent designs')}</h2>
            {q && <NavLink to="/" className="btn ghost sm">{t('Clear search')}</NavLink>}
          </div>
          <DesignGrid designs={designs && designs.slice(0, shown)} reload={load} folders={folders} empty={q ? t('No designs match “{q}”.', { q }) : (
            <EmptyState icon={<Sparkles size={22} />} action={<button className="btn primary" onClick={() => onCreate()}><Plus size={16} /> {t('Create a design')}</button>}>
              {t('No designs yet. Create one above or start from a template.')}
            </EmptyState>
          )} />
          {designs && designs.length > shown && (
            <div className="center" style={{ marginTop: 16 }}>
              <button className="btn" onClick={() => setShown((n) => n + PAGE)}>{t('Show more')}</button>
            </div>
          )}
        </section>

        {!q && <CategoryTiles categories={tpl.categories} templates={tpl.templates} />}
        {!q && <PopularSizes onPreset={create} />}
      </div>
    </FileDropArea>
  );
}
