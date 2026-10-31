/** Cookie / analytics consent (PL-008): asked once, equal-weight choices, changeable at any time. */
import { useEffect, useState } from 'react';
import { Link, useLocation } from 'react-router';
import { consent, setConsent, track } from '../lib/analytics';
import { t } from '../i18n';

export function ConsentBanner() {
  const [open, setOpen] = useState(() => !consent());
  const loc = useLocation();
  useEffect(() => {
    const on = () => setOpen(true);
    window.addEventListener('oc:consent-open', on);
    return () => window.removeEventListener('oc:consent-open', on);
  }, []);
  // Page views (only recorded once analytics is allowed).
  useEffect(() => {
    track('page_view', { route: loc.pathname.replace(/\/[A-Za-z0-9]{16}(?=\/|$)/g, '/:id') });
  }, [loc.pathname]);
  if (!open || loc.pathname.startsWith('/view/')) return null;
  const choose = (analytics: boolean) => {
    void setConsent(analytics);
    setOpen(false);
  };
  return (
    <div className="consent" role="dialog" aria-label={t('Cookie choices')} aria-live="polite">
      <p style={{ margin: 0 }}>
        {t('We use essential cookies to keep you signed in. With your OK we also measure how features are used, on our own servers only — no ads, no third parties.')}{' '}
        <Link className="linkish" to="/legal/cookies">{t('Cookie policy')}</Link>
      </p>
      <div className="row">
        <button className="btn" onClick={() => choose(false)}>{t('Essential only')}</button>
        <button className="btn" onClick={() => choose(true)}>{t('Allow analytics')}</button>
      </div>
    </div>
  );
}

export function ConsentSettingsLink() {
  return <button className="linkish faint" style={{ fontWeight: 400 }} onClick={() => window.dispatchEvent(new Event('oc:consent-open'))}>{t('Cookie settings')}</button>;
}
