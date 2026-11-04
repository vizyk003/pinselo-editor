/**
 * Public home (signed out): browse and search templates, pick a size, see what Pinselo does —
 * no account needed until an action requires one (AuthGate opens the sign-in dialog and resumes it).
 */
import { BRAND } from '@oc/config';
import { BrandLogo } from '../components/BrandLogo';
import { Accessibility, Globe, Layers, Languages, Lock, Search, Sparkles, Users } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { api, type TemplateSummary } from '../api';
import { useAuthGate } from '../auth/AuthGate';
import { t, tk } from '../i18n';
import { TemplateCard } from './Home';
import { QUICK_TILES, QuickTiles, seeAllIntent, tileIntent } from './home/quickTiles';
import { TemplatesBrowser } from './home/Templates';

const FEATURES = [
  { icon: <Sparkles size={20} />, title: tk('Templates for everything'), body: tk('Social posts, presentations, video, print, websites and docs — all original and free.') },
  { icon: <Users size={20} />, title: tk('Real-time collaboration'), body: tk('Edit together with live cursors, comments, tasks and version history.') },
  { icon: <Layers size={20} />, title: tk('Pro tools, simple editor'), body: tk('Keyframe animation, 1,950 Google Fonts, charts, vector PDF and PPTX export.') },
  { icon: <Globe size={20} />, title: tk('Publish anywhere'), body: tk('Download, present, publish a website, schedule social posts or order prints.') },
  { icon: <Languages size={20} />, title: tk('In your language'), body: tk('Nine languages, right-to-left support and every writing system.') },
  { icon: <Accessibility size={20} />, title: tk('Accessible by design'), body: tk('Screen-reader support, keyboard editing and a built-in accessibility checker.') },
  { icon: <Lock size={20} />, title: tk('Yours to own'), body: tk('Open source and self-hostable. Your designs, your data.') },
];

export function Landing({ templatesOnly }: { templatesOnly?: boolean }) {
  const { requireAuth, openAuth } = useAuthGate();
  const nav = useNavigate();
  // Tiles resume after signing in through the Home intents: /?create=<preset>, custom / upload / photo / more or cat:<category> (g06).
  const gate = (intent: string) => { if (requireAuth(`/?create=${intent}`)) nav(`/?create=${intent}`); };
  const [q, setQ] = useState('');
  const [cat, setCat] = useState('');
  const [data, setData] = useState<{ templates: TemplateSummary[]; categories: Array<{ category: string; n: number }> } | null>(null);
  useEffect(() => {
    if (templatesOnly) return; // TemplatesBrowser loads its own
    const timer = setTimeout(() => {
      api.templates({ ...(q ? { q } : {}), ...(cat ? { category: cat } : {}) }).then(setData).catch(() => setData({ templates: [], categories: [] }));
    }, q ? 200 : 0);
    return () => clearTimeout(timer);
  }, [q, cat, templatesOnly]);
  const shown = q || cat ? data?.templates ?? [] : (data?.templates ?? []).slice(0, 24);
  return (
    <div className="landing">
      <header className="landing-bar">
        <Link to="/" className="logo"><BrandLogo /></Link>
        <nav className="row" aria-label={t('Main')}>
          <Link to="/templates">{t('Templates')}</Link>
          <Link to="/help">{t('Help')}</Link>
        </nav>
        <span className="grow" />
        <button className="btn ghost" onClick={() => openAuth('login')}>{t('Log in')}</button>
        <button className="btn primary" onClick={() => openAuth('signup')}>{t('Sign up')}</button>
      </header>

      <main>
        {!templatesOnly && (
          <section className="hero landing-hero">
            <h1>{t('What will you design today?')}</h1>
            <p className="landing-sub">{t('{brand} makes it easy to create, share and print professional designs.', { brand: BRAND.name })}</p>
            <div className="search">
              <Search size={18} color="#666" />
              <input placeholder={t('Search thousands of templates')} value={q} onChange={(e) => setQ(e.target.value)} aria-label={t('Search templates')} />
            </div>
            <QuickTiles
              onPreset={gate}
              onAction={(a) => gate(tileIntent(QUICK_TILES.find((x) => x.action === a)!))}
              onSeeAll={(c) => gate(seeAllIntent(c))}
            />
          </section>
        )}

        {/* /templates signed out: the same hub, filters and shareable ?t= preview links as signed in (g23–g25). */}
        {templatesOnly ? <TemplatesBrowser /> : (
          <section className="content">
            <div className="section-head">
              <h2>{q ? t('Results for “{q}”', { q }) : t('Start from a template')}</h2>
            </div>
            <div className="chips">
              <button className={`chip ${cat === '' ? 'active' : ''}`} onClick={() => setCat('')}>{t('All')}</button>
              {data?.categories.map((c) => (
                <button key={c.category} className={`chip ${cat === c.category ? 'active' : ''}`} onClick={() => setCat(c.category)}>
                  {t(c.category)} <span className="faint">{c.n}</span>
                </button>
              ))}
            </div>
            {!data ? <div className="spinner" /> : (
              <div className="grid" style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))' }}>
                {shown.map((tpl) => <TemplateCard key={tpl.id} t={tpl} />)}
              </div>
            )}
            {data && !shown.length && <div className="empty">{t('No templates match.')}</div>}
            {!q && !cat && (data?.templates.length ?? 0) > shown.length && (
              <div className="center" style={{ marginTop: 16 }}><Link className="btn" to="/templates">{t('See all templates')}</Link></div>
            )}
          </section>
        )}

        {!templatesOnly && (
          <section className="content landing-features">
            {FEATURES.map((f) => (
              <div key={f.title} className="landing-feature">
                <span className="ic">{f.icon}</span>
                <h3>{t(f.title)}</h3>
                <p className="muted">{t(f.body)}</p>
              </div>
            ))}
          </section>
        )}
      </main>

      <footer className="landing-foot">
        <span>© {BRAND.name}</span>
        <Link className="landing-foot-nav" to="/templates">{t('Templates')}</Link>
        <Link className="landing-foot-nav" to="/help">{t('Help')}</Link>
        <Link to="/legal/terms">{t('Terms of Use')}</Link>
        <Link to="/legal/privacy">{t('Privacy Policy')}</Link>
        <Link to="/legal/cookies">{t('Cookies')}</Link>
        <Link to="/status">{t('Status')}</Link>
      </footer>
    </div>
  );
}
