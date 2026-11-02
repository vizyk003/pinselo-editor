/** Template studio (TP-040): publish the current design as a team (brand) template or, for staff, to the public catalogue. */
import { useState } from 'react';
import { ApiError, brandHub, contentApi } from '../api';
import { brandStore, useBrandHub } from './brandStore';
import { useAuth } from '../App';
import { interp, Modal, useToast } from '../components/ui';
import { useDoc, useEd } from './context';
import { t, tk } from '../i18n';

const CATEGORIES = [tk('Social'), tk('Presentation'), tk('Video'), tk('Print'), tk('Marketing'), tk('Education'), tk('Business'), tk('Events'), tk('Personal'), tk('Docs'), tk('Website')];

export function PublishTemplateDialog({ onClose }: { onClose: () => void }) {
  const { design } = useEd();
  const doc = useDoc();
  const { user } = useAuth();
  const toast = useToast();
  const hub = useBrandHub();
  const [f, setF] = useState({ title: doc.meta.title, category: 'Social', tags: '', description: '', scope: 'team' as 'team' | 'public', brand: false });
  const published = hub.design?.id === design.id ? hub.design.published ?? [] : [];
  const [busyTpl, setBusyTpl] = useState('');
  const republish = async (id: string) => {
    setBusyTpl(id);
    try {
      const r = await brandHub.republish(id);
      toast(t('Template updated to version {v}', { v: r.version }));
      void brandStore.loadDesign(design.id);
    } catch (e) {
      toast(e instanceof ApiError ? e.message ?? t('Publishing failed.') : t('Publishing failed.'));
    } finally {
      setBusyTpl('');
    }
  };
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('Publish as template')} onClose={onClose} size="sm">
      <p className="muted" style={{ margin: 0 }}>{t('A snapshot of this design becomes a template. Later edits to the design don’t change it.')}</p>
      <label className="field">{t('Name')}<input className="input" value={f.title} onChange={(e) => setF({ ...f, title: e.target.value })} maxLength={120} /></label>
      <div className="row">
        <label className="field grow">{t('Category')}
          <select className="input" value={f.category} onChange={(e) => setF({ ...f, category: e.target.value })}>{CATEGORIES.map((c) => <option key={c} value={c}>{t(c)}</option>)}</select>
        </label>
        <label className="field grow">{t('Tags (comma-separated)')}<input className="input" value={f.tags} onChange={(e) => setF({ ...f, tags: e.target.value })} placeholder={t('sale, summer')} /></label>
      </div>
      <label className="field">{t('Description')}<textarea className="input" rows={2} value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} maxLength={500} /></label>
      <div className="col" role="radiogroup" aria-label={t('Who can use it')} style={{ gap: 6 }}>
        <label className="check"><input type="radio" checked={f.scope === 'team'} onChange={() => setF({ ...f, scope: 'team' })} /> <span>{interp(t('{name} — everyone in this design’s team can start from it'), { name: <b>{t('Team template')}</b> })}</span></label>
        {user?.staff && <label className="check"><input type="radio" checked={f.scope === 'public'} onChange={() => setF({ ...f, scope: 'public' })} /> <span>{interp(t('{name} — every user (staff only)'), { name: <b>{t('Public catalogue')}</b> })}</span></label>}
      </div>
      {f.scope === 'team' && (
        <label className="check"><input type="checkbox" checked={f.brand} onChange={(e) => setF({ ...f, brand: e.target.checked })} />
          <span>{interp(t('{name} — designs started from it follow this team’s brand controls'), { name: <b>{t('Brand template')}</b> })}</span>
        </label>
      )}
      {published.length > 0 && (
        <div className="col" style={{ gap: 6 }}>
          <b>{t('Published from this design')}</b>
          {published.map((p) => (
            <div key={p.id} className="row">
              <span className="grow">{p.title} <span className="faint">v{p.version}{p.brand ? ` · ${t('Brand template')}` : ''}</span></span>
              <button className="btn sm" disabled={!!busyTpl} onClick={() => republish(p.id)}>{t('Update template')}</button>
            </div>
          ))}
        </div>
      )}
      {err && <div className="form-error">{err}</div>}
      <div className="row"><span className="grow" /><button className="btn" onClick={onClose}>{t('Cancel')}</button>
        <button className="btn primary" disabled={busy || !f.title.trim()} onClick={async () => {
          setBusy(true);
          setErr('');
          try {
            await contentApi.publishTemplate({ designId: design.id, title: f.title.trim(), category: f.category, tags: f.tags.split(',').map((s) => s.trim()).filter(Boolean), description: f.description || undefined, scope: f.scope, brand: f.scope === 'team' && f.brand });
            toast(f.scope === 'team' ? t('Published to your team’s templates') : t('Published to the catalogue'));
            onClose();
          } catch (e) {
            setErr(e instanceof ApiError ? e.message ?? t('Publishing failed.') : t('Publishing failed.'));
          } finally {
            setBusy(false);
          }
        }}>{t('Publish')}</button>
      </div>
    </Modal>
  );
}
