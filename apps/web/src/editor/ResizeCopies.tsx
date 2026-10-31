/**
 * Copy and resize (g101): make copies of the design in several sizes at once; the open design stays as it is. Each
 * copy is made by the API from the latest saved state and resized there (content scaled to fit, like Resize).
 * Smart relayout is a separate feature (plan 10, DS-103).
 */
import { formatDims, PRESET_CATEGORIES, presetsInCategory, searchPresets, SIZE_PRESETS, unitToPx, type PresetCategoryId, type Unit } from '@oc/engine';
import { ExternalLink, Plus, X } from 'lucide-react';
import { useState } from 'react';
import { api, ApiError } from '../api';
import { SearchField, useToast } from '../components/ui';
import { t, tn } from '../i18n';
import { useDoc, useEd } from './context';

interface Target { key: string; label: string; w: number; h: number; unit: Unit }

export function ResizeCopies({ onClose }: { onClose: () => void }) {
  const { design, provider } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const [q, setQ] = useState('');
  const [cat, setCat] = useState<PresetCategoryId | 'all'>('all');
  const [picked, setPicked] = useState<Target[]>([]);
  const [cw, setCw] = useState(1080);
  const [ch, setCh] = useState(1080);
  const [cu, setCu] = useState<Unit>('px');
  const [scaleContent, setScaleContent] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [made, setMade] = useState<Array<{ id: string; label: string }>>([]);
  const formats = q.trim() ? searchPresets(q, t) : cat === 'all' ? SIZE_PRESETS : presetsInCategory(cat);
  const has = (key: string) => picked.some((p) => p.key === key);
  const toggle = (x: Target) => setPicked((ps) => (ps.some((p) => p.key === x.key) ? ps.filter((p) => p.key !== x.key) : [...ps, x]));
  const addCustom = () => {
    if (!(cw > 0 && ch > 0)) return;
    const w = unitToPx(cw, cu), h = unitToPx(ch, cu);
    const key = `custom:${w}x${h}:${cu}`;
    if (!has(key)) setPicked((ps) => [...ps, { key, label: formatDims(w, h, cu), w, h, unit: cu }]);
  };

  const create = async () => {
    setBusy(t('Saving your latest changes…'));
    // The copies start from the saved design: wait (briefly) for edits still on their way to the server.
    for (let i = 0; i < 40 && provider.hasUnsyncedChanges; i++) await new Promise((r) => setTimeout(r, 100));
    const out: Array<{ id: string; label: string }> = [];
    try {
      for (const [i, p] of picked.entries()) {
        setBusy(t('Creating copy {done} of {total}…', { done: i + 1, total: picked.length }));
        const title = `${doc.meta.title || t('Untitled design')} – ${p.label}`.slice(0, 200);
        const r = await api.copyDesign(design.id, { w: p.w, h: p.h, unit: p.unit, scaleContent, title });
        out.push({ id: r.id, label: p.label });
        // A server without "Copy and resize" makes a plain copy: say so instead of pretending it was resized.
        if (!r.resized) throw new Error(t('This server made a copy but could not resize it.'));
      }
      toast(tn(out.length, 'Created {n} resized copy', 'Created {n} resized copies'));
    } catch (e) {
      toast(e instanceof ApiError ? e.message ?? t('Could not create the copies.') : (e as Error).message);
    } finally {
      setMade(out);
      setBusy(null);
    }
  };

  if (made.length)
    return (
      <div className="col">
        <p className="muted" style={{ margin: 0 }}>{t('Your copies are in Projects. This design is unchanged.')}</p>
        <ul className="list resize-made">
          {made.map((m) => (
            <li key={m.id} className="row">
              <span className="grow">{m.label}</span>
              <a className="btn sm" href={`/design/${m.id}`} target="_blank" rel="noreferrer">{t('Open')} <ExternalLink size={14} /></a>
            </li>
          ))}
        </ul>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button className="btn" onClick={() => { setMade([]); setPicked([]); }}>{t('Make more copies')}</button>
          <button className="btn primary" onClick={onClose}>{t('Done')}</button>
        </div>
      </div>
    );

  return (
    <div className="col">
      <p className="muted" style={{ margin: 0 }}>{t('Pick one or more sizes. Each becomes a new design; this one stays as it is.')}</p>
      <div className="row">
        <SearchField className="grow" value={q} onChange={setQ} placeholder={t('Search sizes')} onClear={() => setQ('')} />
        <select className="input" style={{ width: 170 }} value={cat} onChange={(e) => { setQ(''); setCat(e.target.value as typeof cat); }} aria-label={t('Category')}>
          <option value="all">{t('All sizes')}</option>
          {PRESET_CATEGORIES.filter((c) => c.group).map((c) => <option key={c.id} value={c.id}>{t(c.label)}</option>)}
        </select>
      </div>
      <div className="preset-list resize-pick" style={{ maxHeight: 220, overflow: 'auto' }} role="group" aria-label={t('Sizes')}>
        {formats.map((p) => (
          <label key={p.id} className="row">
            <input type="checkbox" checked={has(p.id)} onChange={() => toggle({ key: p.id, label: t(p.label), w: p.w, h: p.h, unit: p.unit })} />
            <span className="grow">{t(p.label)}</span><span className="faint" style={{ fontSize: 12 }}>{formatDims(p.w, p.h, p.unit)}</span>
          </label>
        ))}
        {!formats.length && <span className="faint" style={{ fontSize: 13 }}>{t('No sizes match “{q}”.', { q: q.trim() })}</span>}
      </div>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <span className="muted" style={{ fontSize: 13 }}>{t('Custom size')}</span>
        <input className="input" type="number" style={{ width: 90 }} value={cw} min={1} onChange={(e) => setCw(+e.target.value)} aria-label={t('Width')} />
        <span aria-hidden>×</span>
        <input className="input" type="number" style={{ width: 90 }} value={ch} min={1} onChange={(e) => setCh(+e.target.value)} aria-label={t('Height')} />
        <select className="input" style={{ width: 72 }} value={cu} onChange={(e) => setCu(e.target.value as Unit)} aria-label={t('Unit')}>{(['px', 'in', 'mm', 'cm'] as const).map((u) => <option key={u}>{u}</option>)}</select>
        <button className="btn sm" onClick={addCustom} disabled={!(cw > 0 && ch > 0)}><Plus size={14} /> {t('Add size')}</button>
      </div>
      {picked.length > 0 && (
        <div className="chips" aria-live="polite">
          {picked.map((p) => (
            <span key={p.key} className="chip">
              {p.label}
              <button className="btn ghost sm icon" aria-label={t('Remove {name}', { name: p.label })} onClick={() => toggle(p)}><X size={12} /></button>
            </span>
          ))}
        </div>
      )}
      <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={scaleContent} onChange={(e) => setScaleContent(e.target.checked)} /> {t('Scale content to fit the new size')}</label>
      <div className="row" style={{ justifyContent: 'flex-end' }}>
        {busy && <span className="muted grow" role="status" style={{ fontSize: 13 }}>{busy}</span>}
        <button className="btn primary" disabled={!picked.length || Boolean(busy)} onClick={() => void create()}>
          {picked.length ? tn(picked.length, 'Create {n} copy', 'Create {n} copies') : t('Pick a size')}
        </button>
      </div>
    </div>
  );
}
