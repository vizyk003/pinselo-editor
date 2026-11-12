/**
 * /oauth/authorize — consent screen of the AI connector's OAuth server. An assistant (Claude, ChatGPT, Claude Code…)
 * sends the user here; signed out, the normal sign-in comes back to this exact URL (?next). The API validates the
 * request (client, redirect URI, PKCE, scopes) before anything is shown, so an invalid request never redirects;
 * Allow or Cancel returns to the app's registered redirect URI with a code or `access_denied`.
 */
import { BRAND } from '@oc/config';
import { BrandLogo } from '../components/BrandLogo';
import { AlertTriangle, ArrowLeftRight, BadgeCheck, Check } from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { Link, useLocation } from 'react-router';
import { api, ApiError, oauthApi, type OAuthRequestInfo } from '../api';
import { useAuth } from '../App';
import { Avatar, interp } from '../components/ui';
import { t, tk } from '../i18n';
import { Login } from './Auth';

/** What each grantable scope lets an app do, in plain language. */
export const SCOPE_TEXT: Record<string, string> = {
  'designs:read': tk('See your designs and templates'),
  'designs:write': tk('Create and edit designs'),
  'assets:write': tk('Upload images to your account'),
  exports: tk('Export and download your designs'),
  'planner:read': tk('See posts in your Content Planner'),
  'planner:write': tk('Draft and edit posts in your Content Planner (never approve them)'),
  'planner:publish': tk('Publish approved posts and report the results'),
};

const ERROR_TEXT: Record<string, string> = {
  invalid_client: tk('This app isn’t registered with {brand}, or its registration couldn’t be loaded.'),
  invalid_redirect_uri: tk('The app asked to send you back to an address it hasn’t registered, so we stopped here to keep your account safe.'),
  invalid_scope: tk('The app asked for access that {brand} doesn’t offer.'),
  invalid_target: tk('The app asked for access to a different service.'),
  invalid_request: tk('The app sent an incomplete or invalid request. Try connecting again from the app.'),
};

/** An app's logo, or its initials when it has none (or the image can't load). */
export function AppMark({ name, logo, size = 48 }: { name: string; logo: string | null; size?: number }) {
  const [broken, setBroken] = useState(false);
  if (logo && !broken) {
    return <img src={logo} alt="" width={size} height={size} referrerPolicy="no-referrer" onError={() => setBroken(true)} style={{ borderRadius: size * 0.22, objectFit: 'cover', flex: 'none', background: 'var(--surface-2)' }} />;
  }
  return <Avatar name={name} color="#57524a" size={size} />;
}

function Card({ children }: { children: ReactNode }) {
  return (
    <div className="auth">
      {/* One shrinkable column, so a long email wraps instead of widening the card on phones. */}
      <div className="auth-card" style={{ gridTemplateColumns: 'minmax(0, 1fr)' }}>{children}</div>
    </div>
  );
}

type View =
  | { kind: 'loading' }
  | { kind: 'ready'; info: OAuthRequestInfo }
  | { kind: 'error'; code: string; detail: string }
  | { kind: 'leaving'; app: string };

const failed = (e: unknown): View =>
  e instanceof ApiError ? { kind: 'error', code: e.code, detail: e.message !== e.code ? e.message : '' } : { kind: 'error', code: 'network', detail: '' };

export function OAuthConsent() {
  const { user, setUser } = useAuth();
  const loc = useLocation();
  const params = useMemo(() => Object.fromEntries(new URLSearchParams(loc.search)), [loc.search]);
  const [view, setView] = useState<View>({ kind: 'loading' });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!user) return;
    setView({ kind: 'loading' });
    oauthApi.info(params).then((info) => setView({ kind: 'ready', info })).catch((e) => setView(failed(e)));
  }, [user?.id, params]);

  // Signed out: the regular sign-in, which returns here with the full request.
  if (!user) return <Login next={loc.pathname + loc.search} />;

  if (view.kind === 'loading') return <Card><div className="spinner" /></Card>;
  if (view.kind === 'leaving') {
    return (
      <Card>
        <h1 className="auth-title center">{t('Returning you to {app}…', { app: view.app })}</h1>
        <div className="spinner" />
      </Card>
    );
  }
  if (view.kind === 'error') {
    const text = view.code === 'network' ? t('Something went wrong. Try again.') : t(ERROR_TEXT[view.code] ?? ERROR_TEXT.invalid_request!, { brand: BRAND.name });
    return (
      <Card>
        <Link to="/" className="logo"><BrandLogo /></Link>
        <div>
          <h1 className="auth-title">{t('This connection request isn’t valid')}</h1>
          <p className="muted" style={{ margin: 0 }}>{text}</p>
        </div>
        {view.detail && <p className="faint" style={{ margin: 0, fontSize: 12, overflowWrap: 'anywhere' }}>{view.detail}</p>}
        <Link className="btn primary tall" to="/">{t('Go to {brand}', { brand: BRAND.name })}</Link>
      </Card>
    );
  }

  const { client, scopes, user: who } = view.info;
  const decide = async (approve: boolean) => {
    setBusy(true);
    try {
      const r = await oauthApi.decide(params, approve);
      setView({ kind: 'leaving', app: client.name });
      window.location.assign(r.redirect);
    } catch (e) {
      setView(failed(e));
      setBusy(false);
    }
  };
  const switchAccount = async () => {
    await api.logout().catch(() => {});
    setUser(null);
  };

  return (
    <Card>
      <div className="row" style={{ justifyContent: 'center', gap: 14 }}>
        <AppMark name={client.name} logo={client.logo} />
        <ArrowLeftRight size={18} className="faint" aria-hidden />
        <span className="logo"><img src="/favicon.svg" alt="" style={{ width: 48, height: 48 }} /></span>
      </div>
      <h1 className="auth-title center" style={{ fontSize: 20, overflowWrap: 'anywhere' }}>
        {interp(t('{app} wants to access your {brand} account'), { app: <b>{client.name}</b>, brand: BRAND.name })}
      </h1>
      {client.verified && client.domain ? (
        <p className="row" style={{ margin: 0, gap: 6, justifyContent: 'center', fontSize: 13 }}>
          <BadgeCheck size={16} style={{ color: 'var(--ok)', flex: 'none' }} aria-hidden />
          <span>{interp(t('Verified app from {domain}'), { domain: <b>{client.domain}</b> })}</span>
        </p>
      ) : (
        <div role="note" className="row" style={{ gap: 10, alignItems: 'flex-start', padding: '10px 12px', borderRadius: 'var(--radius-sm)', background: 'color-mix(in srgb, #d97706 12%, var(--surface))', fontSize: 13, lineHeight: 1.45 }}>
          <AlertTriangle size={16} style={{ color: '#b45309', flex: 'none', marginTop: 2 }} aria-hidden />
          <span>{interp(t('{app} named itself; {brand} can’t verify who made it. Only allow it if you just started connecting from an app you trust; afterwards you’ll go to {host}.'), {
            app: <b>{client.name}</b>, brand: BRAND.name, host: <b>{client.redirectHost}</b>,
          })}</span>
        </div>
      )}
      <div className="col" style={{ gap: 10 }}>
        <div className="muted" style={{ fontSize: 13 }}>{t('This will allow {app} to:', { app: client.name })}</div>
        <ul className="plain col" style={{ gap: 8 }}>
          {scopes.map((s) => (
            <li key={s} className="row" style={{ gap: 10, alignItems: 'flex-start' }}>
              <Check size={16} style={{ color: 'var(--ok)', flex: 'none', marginTop: 2 }} aria-hidden />
              <span>{SCOPE_TEXT[s] ? t(SCOPE_TEXT[s]) : s}</span>
            </li>
          ))}
        </ul>
      </div>
      <div className="row" style={{ gap: 10, padding: '10px 12px', border: '1px solid var(--border)', borderRadius: 'var(--radius-sm)', background: 'var(--surface-2)' }}>
        <Avatar name={who.name} color={user.color} size={28} />
        <div className="grow" style={{ minWidth: 0, fontSize: 13, overflowWrap: 'anywhere' }}>
          {interp(t('Signed in as {email}'), { email: <b>{who.email}</b> })}
        </div>
        <button type="button" className="linkish" style={{ fontSize: 13, flex: 'none' }} onClick={switchAccount} disabled={busy}>{t('Switch account')}</button>
      </div>
      <div className="row" style={{ gap: 10 }}>
        <button className="btn tall grow" onClick={() => decide(false)} disabled={busy}>{t('Cancel')}</button>
        <button className="btn primary tall grow" onClick={() => decide(true)} disabled={busy} autoFocus>{t('Allow')}</button>
      </div>
      <p className="faint" style={{ margin: 0, fontSize: 12, lineHeight: 1.5 }}>
        {interp(t('You’ll go back to {host} afterwards. Only allow apps you trust; you can disconnect them anytime in {settings}.'), {
          host: <b>{client.redirectHost}</b>,
          settings: <Link className="linkish" style={{ fontSize: 12 }} to="/settings/developers" target="_blank">{t('Settings → Developers')}</Link>,
        })}
      </p>
    </Card>
  );
}
