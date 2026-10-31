/**
 * The Create design dialog (g08): a category nav on the left (Suggestions, Social media, Video... Custom size),
 * Quick actions / Recently used / Popular / Try something new under Suggestions, platform tabs per category,
 * and a search that matches sizes and published templates. Custom size has an aspect lock and popular layouts (g14).
 */
import {
  findPreset, formatDims, lockedSide, POPULAR_LAYOUTS, popularPresets, PRESET_CATEGORIES, presetPlatforms, presetsInCategory, pushRecentPreset,
  pxToUnit, searchPresets, tryNewPresets, unitToPx, type PresetCategoryId, type SizePreset, type Unit,
} from '@oc/engine';
import {
  ArrowLeftRight, Clapperboard, FileText, Globe, LayoutTemplate, Lightbulb, Lock, Megaphone, PenLine, Presentation, Printer, Ruler, Smartphone, Unlock,
} from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { useNavigate } from 'react-router';
import { useAuth } from '../../App';
import { api, templateThumbUrl, type TemplateSummary } from '../../api';
import { Modal, SearchField, useScrollFade, useToast } from '../../components/ui';
import { t } from '../../i18n';

const CATEGORY_ICONS: Record<PresetCategoryId, ReactNode> = {
  suggestions: <Lightbulb size={16} />, social: <Smartphone size={16} />, video: <Clapperboard size={16} />, presentations: <Presentation size={16} />,
  print: <Printer size={16} />, documents: <FileText size={16} />, web: <Globe size={16} />, marketing: <Megaphone size={16} />,
  whiteboard: <PenLine size={16} />, custom: <Ruler size={16} />,
};

/** Quick actions under Suggestions: one-click starts of each kind of design. */
const QUICK_ACTIONS: Array<{ preset: string; icon: ReactNode }> = [
  { preset: 'doc', icon: <FileText size={18} /> },
  { preset: 'whiteboard', icon: <PenLine size={18} /> },
  { preset: 'presentation', icon: <Presentation size={18} /> },
  { preset: 'video', icon: <Clapperboard size={18} /> },
  { preset: 'ig-post', icon: <Smartphone size={18} /> },
  { preset: 'website', icon: <Globe size={18} /> },
];

// Recently used sizes, per person and per browser. Storage can be unavailable (private mode), so every access is guarded.
const recentKey = (userId: string | undefined) => `oc.create.recent.${userId ?? 'anon'}`;
function loadRecent(userId: string | undefined): string[] {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(recentKey(userId)) ?? '[]');
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string' && Boolean(findPreset(x))).slice(0, 8) : [];
  } catch {
    return [];
  }
}
function saveRecent(userId: string | undefined, ids: string[]) {
  try {
    localStorage.setItem(recentKey(userId), JSON.stringify(ids));
  } catch { /* not persisted */ }
}

export function CreateDialog({ onClose, folderId, workspaceId, templatesOnly, initialCategory }: {
  onClose: () => void; folderId?: string; workspaceId?: string; templatesOnly?: boolean;
  /** Opens on this category, e.g. 'custom' for a "Custom size" button. */
  initialCategory?: PresetCategoryId;
}) {
  const nav = useNavigate();
  const navRow = useScrollFade();
  const toast = useToast();
  const { user } = useAuth();
  // Brand templates of the team (A8): offered first; the only option when the team requires them.
  const [teamTpl, setTeamTpl] = useState<TemplateSummary[]>([]);
  useEffect(() => {
    if (workspaceId) void api.templates({ scope: 'team' }).then((r) => setTeamTpl(r.templates.filter((x) => x.workspaceId === workspaceId))).catch(() => {});
  }, [workspaceId]);
  const [cat, setCat] = useState<PresetCategoryId>(initialCategory ?? 'suggestions');
  const [platform, setPlatform] = useState<string | null>(null);
  const [q, setQ] = useState('');
  const [recent, setRecent] = useState<string[]>(() => loadRecent(user?.id));
  const [busy, setBusy] = useState(false);
  const create = async (b: Parameters<typeof api.createDesign>[0]) => {
    if (busy) return;
    setBusy(true);
    try {
      if (b.preset) {
        const next = pushRecentPreset(recent, b.preset);
        setRecent(next);
        saveRecent(user?.id, next);
      }
      const r = await api.createDesign({ ...b, folderId: folderId ?? null, workspaceId: workspaceId ?? null });
      nav(`/design/${r.id}`);
    } catch (e) {
      toast((e as Error).message);
      setBusy(false);
    }
  };
  const pick = (p: SizePreset) => void create({ preset: p.id });

  // Search: sizes match locally; templates come from the gallery search, debounced.
  const query = q.trim();
  const sizeHits = useMemo(() => searchPresets(query, t), [query]);
  const [tplHits, setTplHits] = useState<TemplateSummary[] | null>(null);
  useEffect(() => {
    if (!query || templatesOnly) { setTplHits(null); return; }
    let live = true;
    const h = setTimeout(() => {
      api.templates({ q: query }).then((r) => live && setTplHits(r.templates.slice(0, 8))).catch(() => live && setTplHits([]));
    }, 250);
    return () => { live = false; clearTimeout(h); };
  }, [query, templatesOnly]);
  const browse = (term: string) => {
    onClose();
    nav(term ? `/templates?q=${encodeURIComponent(term)}` : '/templates');
  };

  const inCat = useMemo(() => presetsInCategory(cat), [cat]);
  const platforms = useMemo(() => presetPlatforms(inCat), [inCat]);
  const shown = platform ? inCat.filter((p) => p.platform === platform) : inCat;
  const catLabel = PRESET_CATEGORIES.find((c) => c.id === cat)?.label ?? '';
  const recentPresets = recent.map((id) => findPreset(id)).filter((p): p is SizePreset => Boolean(p));

  return (
    <Modal title={t('Create a design')} onClose={onClose} size={templatesOnly ? 'lg' : 'xl'}>
      {teamTpl.length > 0 && (
        <div className="col">
          <h4 style={{ margin: 0 }} className="muted">{t('Brand templates')}</h4>
          <TemplateGrid templates={teamTpl} onPick={(x) => void create({ templateId: x.id })} />
        </div>
      )}
      {templatesOnly && <p className="muted" style={{ margin: 0 }}>{teamTpl.length ? t('This team starts designs from its brand templates.') : t('This team starts designs from its brand templates, and none are published yet. Ask an admin or designer to publish one.')}</p>}
      {!templatesOnly && (
        <>
          <SearchField value={q} onChange={setQ} placeholder={t('What would you like to create?')} label={t('Search sizes and templates')} autoFocus onClear={() => setQ('')} />
          <div className="create-dlg">
            <nav className="create-dlg-nav" aria-label={t('Design types')} ref={navRow}>
              {PRESET_CATEGORIES.map((c) => (
                <button key={c.id} className={!query && cat === c.id ? 'active' : ''} aria-current={!query && cat === c.id ? 'true' : undefined}
                  onClick={() => { setQ(''); setCat(c.id); setPlatform(null); }}>
                  {CATEGORY_ICONS[c.id]}<span>{t(c.label)}</span>
                </button>
              ))}
            </nav>
            <div className="create-dlg-main">
              {query ? (
                <>
                  <Section title={t('Sizes')}>
                    {sizeHits.length ? <PresetGrid presets={sizeHits} onPick={pick} /> : (
                      <p className="faint" style={{ margin: 0 }}>{t('No sizes match “{q}”.', { q: query })} <button className="btn sm" onClick={() => { setQ(''); setCat('custom'); }}>{t('Use a custom size')}</button></p>
                    )}
                  </Section>
                  <Section title={t('Templates')} action={<button className="btn sm ghost" onClick={() => browse(query)}><LayoutTemplate size={14} /> {t('Browse templates')}</button>}>
                    {tplHits === null ? <p className="faint" style={{ margin: 0 }}>{t('Searching…')}</p>
                      : tplHits.length ? <TemplateGrid templates={tplHits} onPick={(x) => void create({ templateId: x.id })} />
                        : <p className="faint" style={{ margin: 0 }}>{t('No templates match “{q}”.', { q: query })}</p>}
                  </Section>
                </>
              ) : cat === 'suggestions' ? (
                <>
                  <Section title={t('Quick actions')}>
                    <div className="create-dlg-quick">
                      <button onClick={() => setCat('custom')}><Ruler size={18} /><span>{t('Custom size')}</span></button>
                      {QUICK_ACTIONS.map((a) => {
                        const p = findPreset(a.preset);
                        return p && <button key={a.preset} onClick={() => pick(p)}>{a.icon}<span>{t(p.label)}</span></button>;
                      })}
                      <button onClick={() => browse('')}><LayoutTemplate size={18} /><span>{t('Browse templates')}</span></button>
                    </div>
                  </Section>
                  {recentPresets.length > 0 && (
                    <Section title={t('Recently used')} action={<button className="btn sm ghost" onClick={() => { setRecent([]); saveRecent(user?.id, []); }}>{t('Clear')}</button>}>
                      <PresetGrid presets={recentPresets} onPick={pick} />
                    </Section>
                  )}
                  <Section title={t('Popular')}><PresetGrid presets={popularPresets()} onPick={pick} /></Section>
                  <Section title={t('Try something new')}><PresetGrid presets={tryNewPresets(recent)} onPick={pick} /></Section>
                </>
              ) : cat === 'custom' ? (
                <CustomSize onCreate={(w, h, unit) => void create({ w, h, unit })} busy={busy} />
              ) : (
                <>
                  {platforms.length > 1 && (
                    <div className="tabs" role="group" aria-label={t('Platform')}>
                      <button aria-pressed={!platform} className={!platform ? 'active' : ''} onClick={() => setPlatform(null)}>{t('All')}</button>
                      {platforms.map((pl) => <button key={pl} aria-pressed={platform === pl} className={platform === pl ? 'active' : ''} onClick={() => setPlatform(pl)}>{pl}</button>)}
                    </div>
                  )}
                  <Section title={platform ?? t(catLabel)} action={<button className="btn sm ghost" onClick={() => browse(platform ?? catLabel)}><LayoutTemplate size={14} /> {t('Browse templates')}</button>}>
                    <PresetGrid presets={shown} onPick={pick} />
                  </Section>
                </>
              )}
            </div>
          </div>
        </>
      )}
    </Modal>
  );
}

function Section({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="col" style={{ gap: 8 }}>
      <div className="row" style={{ justifyContent: 'space-between' }}>
        <h4 style={{ margin: 0 }} className="muted">{title}</h4>
        {action}
      </div>
      {children}
    </section>
  );
}

/** Size cards with a small aspect-ratio swatch, so formats can be told apart at a glance. */
export function PresetGrid({ presets, onPick }: { presets: SizePreset[]; onPick: (p: SizePreset) => void }) {
  return (
    <div className="create-dlg-grid">
      {presets.map((p) => {
        const r = p.w / p.h;
        return (
          <button key={p.id} onClick={() => onPick(p)} title={`${t(p.label)} · ${formatDims(p.w, p.h, p.unit)}`}>
            <span className="create-dlg-swatch" aria-hidden><i style={r >= 1 ? { width: 28, height: Math.max(3, 28 / r) } : { height: 28, width: Math.max(3, 28 * r) }} /></span>
            <span className="create-dlg-name">{t(p.label)}</span>
            <span className="faint" style={{ fontSize: 12 }}>{formatDims(p.w, p.h, p.unit)}</span>
          </button>
        );
      })}
    </div>
  );
}

function TemplateGrid({ templates, onPick }: { templates: TemplateSummary[]; onPick: (x: TemplateSummary) => void }) {
  return (
    <div className="preset-list">
      {templates.map((x) => (
        <button key={x.id} onClick={() => onPick(x)} style={{ flexDirection: 'column' }}>
          <img src={templateThumbUrl(x.id)} alt="" style={{ width: '100%', aspectRatio: `${x.width} / ${x.height}`, maxHeight: 180, objectFit: 'cover', borderRadius: 6 }} />
          <span style={{ fontWeight: 600 }}>{x.title}</span>
        </button>
      ))}
    </div>
  );
}

/** Custom size (g14): width × height in a unit, an aspect-ratio lock, orientation swap and popular layouts. */
function CustomSize({ onCreate, busy }: { onCreate: (w: number, h: number, unit: Unit) => void; busy: boolean }) {
  const [w, setW] = useState(1080);
  const [h, setH] = useState(1080);
  const [unit, setUnit] = useState<Unit>('px');
  const [lock, setLock] = useState(false);
  const [ratio, setRatio] = useState(1);
  const setSide = (side: 'w' | 'h', v: number) => {
    if (side === 'w') setW(v); else setH(v);
    // Pixels are whole numbers; physical units keep two decimals.
    if (lock && v > 0) (side === 'w' ? setH : setW)(lockedSide(side, v, ratio, unit === 'px' ? 0 : 2));
  };
  const changeUnit = (u: Unit) => {
    // Keep the physical size when switching units.
    const step = u === 'px' ? 1 : 100;
    const conv = (v: number) => Math.round(pxToUnit(unitToPx(v, unit), u) * step) / step;
    setW(conv(w));
    setH(conv(h));
    setUnit(u);
  };
  const ok = w > 0 && h > 0 && unitToPx(w, unit) <= 20000 && unitToPx(h, unit) <= 20000;
  return (
    <div className="col" style={{ gap: 14 }}>
      <Section title={t('Custom size')}>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <input className="input" style={{ width: 110 }} type="number" min={1} value={w} onChange={(e) => setSide('w', +e.target.value)} aria-label={t('Width')} />
          <button className={`btn icon sm${lock ? ' primary' : ''}`} aria-pressed={lock} title={lock ? t('Unlock aspect ratio') : t('Lock aspect ratio')}
            onClick={() => { if (!lock && w > 0 && h > 0) setRatio(w / h); setLock(!lock); }}>
            {lock ? <Lock size={14} /> : <Unlock size={14} />}
          </button>
          <input className="input" style={{ width: 110 }} type="number" min={1} value={h} onChange={(e) => setSide('h', +e.target.value)} aria-label={t('Height')} />
          <select className="input" value={unit} onChange={(e) => changeUnit(e.target.value as Unit)} style={{ width: 80 }} aria-label={t('Unit')}>
            {(['px', 'in', 'mm', 'cm'] as const).map((u) => <option key={u}>{u}</option>)}
          </select>
          <button className="btn icon sm" title={t('Swap width and height')} onClick={() => { setW(h); setH(w); if (h > 0 && w > 0) setRatio(h / w); }}><ArrowLeftRight size={14} /></button>
          <button className="btn primary" onClick={() => onCreate(Math.round(unitToPx(w, unit)), Math.round(unitToPx(h, unit)), unit)} disabled={!ok || busy}>{t('Create')}</button>
        </div>
        {!ok && w > 0 && h > 0 && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Designs can be up to 20,000 px on each side.')}</p>}
      </Section>
      <Section title={t('Popular layouts')}>
        <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
          {POPULAR_LAYOUTS.map((l) => (
            <button key={l.id} className={`chip${w === l.w && h === l.h && unit === l.unit ? ' on' : ''}`} onClick={() => { setW(l.w); setH(l.h); setUnit(l.unit); setRatio(l.w / l.h); }}>
              {t(l.label)} <span className="faint" style={{ fontWeight: 400 }}>{l.w} × {l.h} {l.unit}</span>
            </button>
          ))}
        </div>
      </Section>
    </div>
  );
}
