/** Publish as a website (WS-001..006): address, SEO, password, custom domain; phone preview; insights (WS-010). */
import { sortedPages } from '@oc/engine';
import { Copy, ExternalLink, Globe, Monitor, RefreshCw, Smartphone } from 'lucide-react';
import { useEffect, useState } from 'react';
import { ApiError, siteApi, type SiteInfo } from '../api';
import { interp, Modal, useToast } from '../components/ui';
import { useDoc, useEd } from './context';
import { SiteInsightsPanel } from './SiteInsights';
import { t, tn } from '../i18n';

const slugify = (s: string) => s.toLowerCase().normalize('NFKD').replace(/[^\w\s-]/g, '').trim().replace(/[\s_]+/g, '-').replace(/-+/g, '-').slice(0, 48) || 'my-site';

export function PublishSiteDialog({ onClose }: { onClose: () => void }) {
  const { design, editor } = useEd();
  const [tab, setTab] = useState<'publish' | 'preview' | 'insights'>('publish');
  const [preview, setPreview] = useState<{ html: string; device: 'mobile' | 'desktop' } | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const doc = useDoc();
  const toast = useToast();
  const [info, setInfo] = useState<SiteInfo | null>(null);
  const [f, setF] = useState({ slug: '', title: '', description: '', noindex: false, analytics: true, password: '', usePassword: false });
  const [domain, setDomain] = useState('');
  const [dns, setDns] = useState<{ txt: { name: string; value: string }; cname: { name: string; value: string } } | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');
  const load = () => siteApi.get(design.id).then((r) => {
    setInfo(r);
    setF((cur) => ({
      ...cur,
      slug: r.site?.slug ?? slugify(doc.meta.title),
      title: r.site?.title ?? doc.meta.title,
      description: r.site?.description ?? '',
      noindex: r.site?.noindex ?? false,
      analytics: r.site?.analytics ?? true,
      usePassword: r.site?.hasPassword ?? false,
    }));
  });
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [design.id]);
  const msg = (e: unknown) => (e instanceof ApiError ? e.message ?? t('That didn’t work.') : t('That didn’t work.'));

  const publish = async () => {
    setBusy(true);
    setErr('');
    try {
      await siteApi.save(design.id, {
        slug: f.slug, title: f.title, description: f.description || undefined, noindex: f.noindex, analytics: f.analytics,
        password: f.usePassword ? (f.password ? f.password : undefined) : null,
      });
      const r = await siteApi.publish(design.id);
      toast(t('Your site is live'));
      await load();
      window.open(r.url, '_blank', 'noopener');
    } catch (e) {
      setErr(msg(e));
    } finally {
      setBusy(false);
    }
  };

  const loadPreview = async (device: 'mobile' | 'desktop') => {
    setPreviewBusy(true);
    try {
      const r = await siteApi.preview(design.id);
      setPreview({ html: r.html, device });
    } catch (e) {
      setErr(msg(e));
    } finally {
      setPreviewBusy(false);
    }
  };
  const setAnalytics = async (on: boolean) => {
    setF((c) => ({ ...c, analytics: on }));
    if (!info?.site) return;
    try {
      await siteApi.save(design.id, { slug: info.site.slug, title: info.site.title, description: info.site.description ?? undefined, noindex: info.site.noindex, analytics: on });
      await load();
    } catch (e) {
      setErr(msg(e));
    }
  };
  const pages = sortedPages(doc);

  if (!info) return <Modal title={t('Publish website')} onClose={onClose}><div className="spinner" /></Modal>;
  const live = info.site?.published;
  const totalViews = info.views.reduce((a, v) => a + v.views, 0);
  return (
    <Modal title={t('Publish website')} onClose={onClose} size="lg">
      <nav className="tabs" aria-label={t('Website')}>
        <button className={tab === 'publish' ? 'active' : ''} onClick={() => setTab('publish')}>{t('Publish')}</button>
        <button className={tab === 'preview' ? 'active' : ''} onClick={() => { setTab('preview'); if (!preview) void loadPreview('mobile'); }}>{t('Preview')}</button>
        {info.site && <button className={tab === 'insights' ? 'active' : ''} onClick={() => setTab('insights')}>{t('Insights')}</button>}
      </nav>
      {tab === 'insights' && info.site && <SiteInsightsPanel designId={design.id} analytics={info.site.analytics} onAnalytics={(on) => void setAnalytics(on)} />}
      {tab === 'preview' && (
        <div style={{ display: 'grid', gap: 12 }}>
          <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
            <button className={`btn sm${preview?.device !== 'desktop' ? ' primary' : ''}`} aria-pressed={preview?.device !== 'desktop'} onClick={() => setPreview((p) => (p ? { ...p, device: 'mobile' } : p))}><Smartphone size={14} /> {t('Mobile')}</button>
            <button className={`btn sm${preview?.device === 'desktop' ? ' primary' : ''}`} aria-pressed={preview?.device === 'desktop'} onClick={() => setPreview((p) => (p ? { ...p, device: 'desktop' } : p))}><Monitor size={14} /> {t('Desktop')}</button>
            <span className="grow" />
            <button className="btn sm" disabled={previewBusy} onClick={() => void loadPreview(preview?.device ?? 'mobile')}><RefreshCw size={14} /> {t('Refresh')}</button>
          </div>
          {err && <div className="form-error">{err}</div>}
          {!preview && <div className="spinner" />}
          {preview && (
            <div style={{ display: 'flex', justifyContent: 'center', background: 'var(--surface-2, #f1f1f4)', borderRadius: 12, padding: 12, overflow: 'hidden' }}>
              {preview.device === 'mobile' ? (
                <iframe title={t('Mobile preview')} srcDoc={preview.html} sandbox="allow-same-origin" style={{ width: 390, height: 640, border: '1px solid var(--border)', borderRadius: 24, background: '#fff' }} />
              ) : (
                <div style={{ width: 640, height: 400, overflow: 'hidden', borderRadius: 8, border: '1px solid var(--border)', background: '#fff' }}>
                  <iframe title={t('Desktop preview')} srcDoc={preview.html} sandbox="allow-same-origin" style={{ width: 1280, height: 800, border: 0, transform: 'scale(0.5)', transformOrigin: '0 0' }} />
                </div>
              )}
            </div>
          )}
          <h3 style={{ margin: 0, fontSize: 14 }}>{t('Mobile layout')}</h3>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Automatic stacks each wide page into one readable column on phones. Same as desktop shrinks the page instead.')}</p>
          {pages.map((p, i) => (
            <label key={p.id} className="row" style={{ fontSize: 13 }}>
              <span className="grow">{p.title || t('Page {n}', { n: i + 1 })}</span>
              <select className="input" style={{ width: 'auto' }} value={p.mobileLayout ?? 'auto'} onChange={(e) => editor.updatePage(p.id, { mobileLayout: e.target.value === 'desktop' ? 'desktop' : undefined })}>
                <option value="auto">{t('Automatic')}</option>
                <option value="desktop">{t('Same as desktop')}</option>
              </select>
            </label>
          ))}
        </div>
      )}
      {tab === 'publish' && <>
      <p className="muted" style={{ margin: 0 }}>{t('Each page becomes a section; page names become the menu. Links, videos and forms work on the site.')}</p>
      {live && info.url && (
        <div className="banner">
          <Globe size={16} /><a className="linkish grow" href={info.url} target="_blank" rel="noreferrer">{info.url}</a>
          <button className="btn sm icon" aria-label={t('Copy link')} onClick={() => void navigator.clipboard.writeText(info.url!).then(() => toast(t('Link copied')))}><Copy size={14} /></button>
          <a className="btn sm icon" href={info.url} target="_blank" rel="noreferrer" aria-label={t('Open site')}><ExternalLink size={14} /></a>
        </div>
      )}
      <label className="field">{t('Address')}
        <div className="row"><span className="faint" style={{ fontSize: 13 }}>{location.origin}/site/</span><input className="input grow" value={f.slug} onChange={(e) => setF({ ...f, slug: slugify(e.target.value) })} /></div>
      </label>
      <label className="field">{t('Site title (browser tab and search results)')}<input className="input" value={f.title} onChange={(e) => setF({ ...f, title: e.target.value })} maxLength={120} /></label>
      <label className="field">{t('Description')}<textarea className="input" rows={2} value={f.description} onChange={(e) => setF({ ...f, description: e.target.value })} maxLength={300} /></label>
      <label className="check"><input type="checkbox" checked={f.usePassword} onChange={(e) => setF({ ...f, usePassword: e.target.checked })} /> {t('Password protect')}</label>
      {f.usePassword && <input className="input" type="password" placeholder={info.site?.hasPassword ? t('Leave empty to keep the current password') : t('Password')} value={f.password} onChange={(e) => setF({ ...f, password: e.target.value })} />}
      <label className="check"><input type="checkbox" checked={f.noindex} onChange={(e) => setF({ ...f, noindex: e.target.checked })} /> {t('Hide from search engines')}</label>
      <label className="check"><input type="checkbox" checked={f.analytics} onChange={(e) => setF({ ...f, analytics: e.target.checked })} /> {t('Collect insights (cookie-free, honours Do Not Track)')}</label>
      {err && <div className="form-error">{err}</div>}
      <div className="row">
        {live && <button className="btn" onClick={() => siteApi.unpublish(design.id).then(load).then(() => toast(t('Site unpublished')))}>{t('Unpublish')}</button>}
        <span className="grow" />
        <button className="btn primary" disabled={busy || f.slug.length < 3 || (f.usePassword && !info.site?.hasPassword && !f.password)} onClick={publish}>{busy ? t('Publishing…') : live ? t('Publish changes') : t('Publish')}</button>
      </div>
      {info.site && (
        <section className="box" style={{ padding: 14 }}>
          <h2 style={{ fontSize: 15 }}>{t('Custom domain')}</h2>
          {info.domains.map((d) => (
            <div key={d.domain} className="row" style={{ flexWrap: 'wrap' }}>
              <b className="grow">{d.domain}</b>
              {d.verified_at ? <span className="pill ok">{t('Connected')}</span> : <button className="btn sm" onClick={() => siteApi.verifyDomain(design.id, d.domain).then(load).then(() => toast(t('Domain connected'))).catch((e) => setErr(msg(e)))}>{t('Verify')}</button>}
              <button className="btn ghost sm" onClick={() => siteApi.removeDomain(design.id, d.domain).then(load)}>{t('Remove')}</button>
              {!d.verified_at && <div className="faint" style={{ fontSize: 12, width: '100%' }}>{interp(t('Add DNS records: TXT {txtName} = {txtValue}, and CNAME (or A/ALIAS) {domain} → {host}.'), { txtName: <code>_pinselo.{d.domain}</code>, txtValue: <code>oc-verify={d.token}</code>, domain: <code>{d.domain}</code>, host: <code>{location.hostname}</code> })}</div>}
            </div>
          ))}
          <div className="row">
            <input className="input grow" placeholder="www.example.com" value={domain} onChange={(e) => setDomain(e.target.value.trim().toLowerCase())} />
            <button className="btn" disabled={!domain} onClick={() => siteApi.addDomain(design.id, domain).then((r) => { setDns(r); setDomain(''); return load(); }).catch((e) => setErr(msg(e)))}>{t('Connect')}</button>
          </div>
          {dns && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Now add the DNS records above, then press Verify.')}</p>}
        </section>
      )}
      {live && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{tn(totalViews, '{n} view in the last 30 days.', '{n} views in the last 30 days.')}</p>}
      </>}
    </Modal>
  );
}
