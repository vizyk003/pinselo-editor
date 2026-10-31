/**
 * Sign-in when the server uses Supabase Auth (plan 12 §C). supabase-js proves who the user is (PKCE); the result is
 * bridged once at POST /api/auth/supabase/session for the normal Pinselo session. The forms mirror the built-in
 * ones in pages/Auth.tsx (same copy, embedded mode for the AuthGate dialog, the same `next` resume rule).
 */
import { BRAND } from '@oc/config';
import type { AuthError, Provider } from '@supabase/supabase-js';
import { Mail } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { api, type User } from '../api';
import { useAuth } from '../App';
import { interp, useToast } from '../components/ui';
import { t } from '../i18n';
import { ErrorLine, MfaStep, Shell, type EmbeddedAuth } from '../pages/Auth';
import type { AuthConfig } from './authConfig';
import { authErrorMessage, callbackUrl, MIN_AGE, oauthOptions, orderProviders, profileProblem, safeNext, signupClosedText, type AuthFlow } from './redirect';
import { resumeSteps } from './sessionRules';
import { bridge, BridgeError, type BridgeBody, type BridgeResult } from './supabaseApi';
import { getSupabase } from './supabaseClient';

const thisYear = new Date().getFullYear();
const YEARS = Array.from({ length: 100 }, (_, i) => thisYear - i);

/** The code of an API error, a supabase-js AuthError (duck-typed: importing supabase-js here would bundle it) or a callback error. */
export function errorCode(e: unknown): string {
  if (!e || typeof e !== 'object') return '';
  const { code, name } = e as { code?: unknown; name?: unknown };
  if (typeof code === 'string') return code;
  // supabase-js throws this one without a code.
  return name === 'AuthSessionMissingError' ? 'session_not_found' : '';
}

/** User-facing text for any sign-in error: translated text for known codes, never a raw message (review C5). */
export const authErrorText = authErrorMessage;

/** supabase-js returns `{ data, error }`; throw the error so one catch handles every call. */
export function must<R extends { data: unknown; error: AuthError | null }>(r: R): Extract<R, { error: null }>['data'] {
  if (r.error) throw r.error;
  return r.data as Extract<R, { error: null }>['data'];
}

type Step =
  | { kind: 'form' }
  | { kind: 'profile'; email: string; name: string | null }
  | { kind: 'mfa' } // Supabase TOTP step-up, then bridge again
  | { kind: 'ticket'; ticket: string }; // built-in TOTP on a linked account (existing /api/auth/mfa): always asked, even right after a Supabase step-up

/** Shared: bridge the current Supabase session and finish — signed in (resume `next`), or one more step. */
export function useSupabaseFinish(nextIn: string | null | undefined) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const next = safeNext(nextIn);
  const [step, setStep] = useState<Step>({ kind: 'form' });
  // Kept across the profile and MFA steps (e.g. revokeOtherSessions after a password reset).
  const extra = useRef<Omit<BridgeBody, 'accessToken' | 'next'>>({});
  const done = (u: User, to = next) => {
    setUser(u);
    nav(safeNext(to), { replace: true });
  };
  const finish = async (more?: Omit<BridgeBody, 'accessToken' | 'next'>) => {
    if (more) extra.current = { ...extra.current, ...more };
    const sb = await getSupabase();
    const token = (await sb.auth.getSession()).data.session?.access_token;
    if (!token) throw new BridgeError(401, 'session_ended', undefined);
    let r: BridgeResult;
    try {
      r = await bridge({ accessToken: token, next, ...extra.current });
    } catch (e) {
      // The token expired during the profile step, or was already used: refresh once and retry (plan 12 §B5).
      if (!['invalid_token', 'token_used'].includes(errorCode(e))) throw e;
      const fresh = (await sb.auth.refreshSession()).data.session?.access_token;
      if (!fresh) throw e;
      r = await bridge({ accessToken: fresh, next, ...extra.current });
    }
    if ('user' in r) return done(r.user, r.next ?? next);
    if ('needsProfile' in r) return setStep({ kind: 'profile', email: r.email, name: r.name });
    if ('ticket' in r) return setStep({ kind: 'ticket', ticket: r.ticket });
    setStep({ kind: 'mfa' });
  };
  return { step, setStep, finish, done, next };
}
export type SupabaseFinish = ReturnType<typeof useSupabaseFinish>;

/** The steps after the first bridge: profile for a new account, or a second factor. */
export function FinishStep({ f, embedded }: { f: SupabaseFinish; embedded?: boolean }) {
  const s = f.step;
  if (s.kind === 'profile') {
    return (
      <Shell embedded={embedded} title={t('Finish creating your account')} sub={s.email}>
        <ProfileStep name={s.name} onSubmit={(profile) => f.finish({ profile })} />
      </Shell>
    );
  }
  if (s.kind === 'ticket') {
    return (
      <Shell embedded={embedded} title={t('Two-step verification')} sub={t('Enter the 6-digit code from your authenticator app.')}>
        <MfaStep ticket={s.ticket} onDone={(u) => f.done(u)} errorText={authErrorText} />
      </Shell>
    );
  }
  return (
    <Shell embedded={embedded} title={t('Two-step verification')} sub={t('Enter the 6-digit code from your authenticator app.')}>
      <SupabaseMfaStep onDone={() => f.finish()} />
    </Shell>
  );
}

/** New account without a usable profile (magic link, OAuth): the same name/age/terms step as the magic-link landing. */
export function ProfileStep({ name, onSubmit }: { name: string | null; onSubmit: (p: { name: string; birthYear: number; acceptTerms: true }) => Promise<void> }) {
  const [f, setF] = useState({ name: name ?? '', birthYear: 0, terms: false });
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const tooYoung = f.birthYear > 0 && thisYear - f.birthYear < MIN_AGE;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const problem = profileProblem(f, thisYear);
    if (problem === 'birthYear') return setErr(t('Select your birth year.'));
    if (problem === 'age') return setErr(t('You need to be at least {age} to use {brand}.', { age: MIN_AGE, brand: BRAND.name }));
    if (problem === 'terms') return setErr(t('Accept the Terms to continue.'));
    if (problem) return;
    setBusy(true);
    setErr('');
    try {
      await onSubmit({ name: f.name.trim(), birthYear: f.birthYear, acceptTerms: true });
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="col" style={{ gap: 12 }} onSubmit={submit}>
      <label className="field">{t('Name')}<input className="input" required maxLength={80} value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoComplete="name" autoFocus /></label>
      <label className="field">
        {t('Birth year')}
        <select className="input" value={f.birthYear} onChange={(e) => setF({ ...f, birthYear: Number(e.target.value) })}>
          <option value={0}>{t('Select…')}</option>
          {YEARS.map((y) => <option key={y} value={y}>{y}</option>)}
        </select>
      </label>
      {tooYoung && <ErrorLine text={t('You need to be at least {age} to use {brand}.', { age: MIN_AGE, brand: BRAND.name })} />}
      <label className="check">
        <input type="checkbox" checked={f.terms} onChange={(e) => setF({ ...f, terms: e.target.checked })} />
        <span>{interp(t('I agree to the {terms} and {privacy}.'), { terms: <Link className="linkish" to="/legal/terms" target="_blank">{t('Terms of Use')}</Link>, privacy: <Link className="linkish" to="/legal/privacy" target="_blank">{t('Privacy Policy')}</Link> })}</span>
      </label>
      <ErrorLine text={tooYoung ? '' : err} />
      <button className="btn primary tall" disabled={busy || !f.name.trim() || !f.birthYear || !f.terms || tooYoung}>{busy ? t('Creating…') : t('Create account')}</button>
    </form>
  );
}

/** Supabase TOTP step-up to aal2 (plan 12 §B6); `onDone` then bridges (or retries) with the new token. */
export function SupabaseMfaStep({ onDone }: { onDone: () => Promise<void> | void }) {
  const [factors, setFactors] = useState<Array<{ id: string; friendly_name?: string }> | null>(null);
  const [factorId, setFactorId] = useState('');
  const [code, setCode] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    getSupabase()
      .then(async (sb) => {
        const totp = must(await sb.auth.mfa.listFactors()).totp;
        setFactors(totp);
        setFactorId(totp[0]?.id ?? '');
        if (!totp.length) setErr(t('That didn’t work. Try again.'));
      })
      .catch((e: unknown) => setErr(authErrorText(e)));
  }, []);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr('');
    try {
      const sb = await getSupabase();
      must(await sb.auth.mfa.challengeAndVerify({ factorId, code: code.replace(/\s/g, '') }));
      await onDone();
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="col" style={{ gap: 14 }} onSubmit={submit}>
      {factors && factors.length > 1 && (
        <label className="field">
          {t('Authenticator app')}
          <select className="input" value={factorId} onChange={(e) => setFactorId(e.target.value)}>
            {factors.map((f, i) => <option key={f.id} value={f.id}>{f.friendly_name || `${t('Authenticator app')} ${i + 1}`}</option>)}
          </select>
        </label>
      )}
      <label className="field">
        {t('Code from your authenticator app')}
        <input className="input" value={code} onChange={(e) => setCode(e.target.value)} autoFocus required inputMode="numeric" autoComplete="one-time-code" placeholder="123 456" />
      </label>
      <ErrorLine text={err} />
      <button className="btn primary tall" disabled={busy || !code || !factorId}>{busy ? t('Checking…') : t('Verify')}</button>
    </form>
  );
}

/** Google's "G" (their sign-in branding asks for the full-colour mark on a neutral button). */
export const GoogleMark = () => (
  <svg width="18" height="18" viewBox="0 0 48 48" aria-hidden="true" focusable="false">
    <path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z" />
    <path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z" />
    <path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z" />
    <path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z" />
  </svg>
);

/** "Continue with …" for the OAuth providers enabled in Supabase; a full-page redirect that comes back to /auth/callback. */
export function OAuthButtons({ config, next, flow, onError }: { config: AuthConfig; next: string; flow?: AuthFlow; onError: (text: string) => void }) {
  const [busy, setBusy] = useState(false);
  const go = async (provider: string) => {
    setBusy(true);
    try {
      const sb = await getSupabase();
      must(await sb.auth.signInWithOAuth({ provider: provider as Provider, options: oauthOptions(provider, next, flow) }));
      // supabase-js navigates away; keep the buttons disabled until it does.
    } catch (e) {
      onError(authErrorText(e));
      setBusy(false);
    }
  };
  const providers = orderProviders(config.supabase?.providers ?? []);
  if (!providers.length) return null;
  return (
    <div className="col" style={{ gap: 8 }}>
      {providers.map((p) => (
        <button key={p.id} type="button" className="btn tall" disabled={busy} onClick={() => void go(p.id)}>
          {p.id === 'google' && <GoogleMark />}
          {t('Continue with {provider}', { provider: p.label })}
        </button>
      ))}
    </div>
  );
}

const Or = () => <div className="or"><span>{t('or')}</span></div>;

export function SupabaseLogin({ embedded, next: nextProp, onSwitch, config }: EmbeddedAuth & { config: AuthConfig }) {
  const sb = config.supabase!;
  const [params] = useSearchParams();
  const next = safeNext(nextProp ?? params.get('next'));
  const f = useSupabaseFinish(next);
  const [mode, setMode] = useState<'password' | 'magic' | 'sent' | 'dev'>('password');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [name, setName] = useState('');
  const [err, setErr] = useState('');
  const [unconfirmed, setUnconfirmed] = useState(false);
  const [busy, setBusy] = useState(false);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setErr('');
    try {
      await fn();
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  const login = () =>
    run(async () => {
      setUnconfirmed(false);
      const c = await getSupabase();
      const r = await c.auth.signInWithPassword({ email: email.trim(), password });
      if (r.error?.code === 'email_not_confirmed') setUnconfirmed(true);
      must(r);
      await f.finish();
    });
  const magic = () =>
    run(async () => {
      const c = await getSupabase();
      const r = await c.auth.signInWithOtp({ email: email.trim(), options: { emailRedirectTo: callbackUrl(next), shouldCreateUser: sb.signup } });
      // With sign-ups closed, an unknown address must look the same as a known one (no enumeration).
      if (r.error && !['otp_disabled', 'signup_disabled', 'user_not_found'].includes(r.error.code ?? '')) throw r.error;
      setMode('sent');
    });
  const resend = () =>
    run(async () => {
      const c = await getSupabase();
      must(await c.auth.resend({ type: 'signup', email: email.trim(), options: { emailRedirectTo: callbackUrl(next) } }));
      setUnconfirmed(false);
      setMode('sent');
    });
  // The development shortcut (no password, never in production) stays on the built-in endpoint.
  const dev = () =>
    run(async () => {
      const r = await api.login(email.trim(), name.trim() || email.split('@')[0]!);
      if ('mfaRequired' in r) f.setStep({ kind: 'ticket', ticket: r.ticket });
      else f.done(r.user);
    });

  if (f.step.kind !== 'form') return <FinishStep f={f} embedded={embedded} />;
  if (mode === 'sent') {
    return (
      <Shell embedded={embedded} title={t('Check your email')} sub={interp(t('We sent a sign-in link to {email}. Open it in this browser to continue.'), { email: <b>{email}</b> })}>
        <button className="linkish" onClick={() => setMode(sb.email ? 'magic' : 'password')}>{t('Use a different email')}</button>
      </Shell>
    );
  }

  return (
    <Shell embedded={embedded} title={embedded ? t('Log in to continue') : t('Log in to {brand}', { brand: BRAND.name })} sub={t('Design anything. Publish anywhere.')}>
      <OAuthButtons config={config} next={next} onError={setErr} />
      {sb.providers.length > 0 && sb.email && <Or />}

      {sb.email && mode === 'password' && (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void login(); }}>
          <label className="field">
            {t('Email')}
            <input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username" autoFocus />
          </label>
          <label className="field">
            <span className="row">{t('Password')} <span className="grow" /><Link to={`/auth/forgot?email=${encodeURIComponent(email)}`} className="linkish">{t('Forgot?')}</Link></span>
            <input className="input" type="password" required value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
          </label>
          <ErrorLine text={err} />
          {unconfirmed && <button type="button" className="btn ghost" disabled={busy} onClick={() => void resend()}>{t('Resend email')}</button>}
          <button className="btn primary tall" disabled={busy}>{busy ? t('Signing in…') : t('Log in')}</button>
          <button type="button" className="btn ghost" onClick={() => setMode('magic')}><Mail size={16} /> {t('Email me a sign-in link')}</button>
        </form>
      )}

      {sb.email && mode === 'magic' && (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void magic(); }}>
          <label className="field">
            {t('Email')}
            <input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" autoFocus />
          </label>
          <ErrorLine text={err} />
          <button className="btn primary tall" disabled={busy}>{busy ? t('Sending…') : t('Send sign-in link')}</button>
          <button type="button" className="btn ghost" onClick={() => setMode('password')}>{t('Use a password instead')}</button>
        </form>
      )}

      {mode === 'dev' && (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void dev(); }}>
          <label className="field">{t('Name')}<input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={t('Ada Lovelace')} /></label>
          <label className="field">{t('Email')}<input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoFocus /></label>
          <ErrorLine text={err} />
          <button className="btn primary tall" disabled={busy}>{t('Continue (development)')}</button>
          <button type="button" className="btn ghost" onClick={() => setMode('password')}>{t('Back')}</button>
        </form>
      )}
      {!sb.email && mode !== 'dev' && <ErrorLine text={err} />}

      {sb.signup && (
        <p className="muted center" style={{ margin: 0 }}>
          {t('New here?')} {onSwitch
            ? <button type="button" className="linkish" onClick={onSwitch}>{t('Create an account')}</button>
            : <Link className="linkish" to={`/signup${next !== '/' ? `?next=${encodeURIComponent(next)}` : ''}`}>{t('Create an account')}</Link>}
        </p>
      )}
      {config.devLogin && mode !== 'dev' && (
        <button className="linkish faint" style={{ fontSize: 12 }} onClick={() => setMode('dev')}>{t('Development sign-in without a password')}</button>
      )}
    </Shell>
  );
}

export function SupabaseSignup({ embedded, next: nextProp, onSwitch, config }: EmbeddedAuth & { config: AuthConfig }) {
  const sb = config.supabase!;
  const [params] = useSearchParams();
  const next = safeNext(nextProp ?? params.get('next'));
  const fin = useSupabaseFinish(next);
  const [f, setF] = useState({ name: '', email: '', password: '', birthYear: 0, terms: false });
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState(false);
  const tooYoung = f.birthYear > 0 && thisYear - f.birthYear < MIN_AGE;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!f.birthYear) return setErr(t('Select your birth year.'));
    if (tooYoung) return setErr(t('You need to be at least {age} to use {brand}.', { age: MIN_AGE, brand: BRAND.name }));
    if (!f.terms) return setErr(t('Accept the Terms to continue.'));
    setBusy(true);
    setErr('');
    try {
      const c = await getSupabase();
      const name = f.name.trim();
      // The profile also travels in user_metadata, so the confirmation link can create the account in any tab (plan 12 §B4).
      const data = must(await c.auth.signUp({
        email: f.email.trim(), password: f.password,
        options: { emailRedirectTo: callbackUrl(next), data: { name, oc_birth_year: f.birthYear, oc_terms: config.termsVersion } },
      }));
      // Auto-confirm returns a session at once. Otherwise (and for an address that already has an account) the same "check your email".
      if (data.session) await fin.finish({ profile: { name, birthYear: f.birthYear, acceptTerms: true } });
      else setSent(true);
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  if (fin.step.kind !== 'form') return <FinishStep f={fin} embedded={embedded} />;
  if (sent) {
    return (
      <Shell embedded={embedded} title={t('Check your email')} sub={interp(t('We sent a link to {email}. Open it in this browser to confirm your address and finish signing up.'), { email: <b>{f.email.trim()}</b> })}>
        <button className="linkish" onClick={() => setSent(false)}>{t('Use a different email')}</button>
      </Shell>
    );
  }
  const loginHref = `/login${params.get('next') ? `?next=${encodeURIComponent(params.get('next')!)}` : ''}`;
  return (
    <Shell embedded={embedded} title={t('Create your account')} sub={t('Free forever for personal use.')}>
      {config.signup === 'invite' && <p className="muted" style={{ margin: 0 }}>{signupClosedText()}</p>}
      <OAuthButtons config={config} next={next} onError={setErr} />
      {sb.providers.length > 0 && sb.email && <Or />}
      {sb.email ? (
        <form className="col" style={{ gap: 12 }} onSubmit={submit}>
          <label className="field">{t('Name')}<input className="input" required maxLength={80} value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoComplete="name" autoFocus /></label>
          <label className="field">{t('Email')}<input className="input" type="email" required value={f.email} onChange={(e) => setF({ ...f, email: e.target.value })} autoComplete="email" /></label>
          <label className="field">
            {t('Password')}
            <input className="input" type="password" required minLength={10} value={f.password} onChange={(e) => setF({ ...f, password: e.target.value })} autoComplete="new-password" />
            <span className="faint" style={{ fontWeight: 400 }}>{t('At least 10 characters.')}</span>
          </label>
          <label className="field">
            {t('Birth year')}
            <select className="input" value={f.birthYear} onChange={(e) => setF({ ...f, birthYear: Number(e.target.value) })}>
              <option value={0}>{t('Select…')}</option>
              {YEARS.map((y) => <option key={y} value={y}>{y}</option>)}
            </select>
          </label>
          <label className="check">
            <input type="checkbox" checked={f.terms} onChange={(e) => setF({ ...f, terms: e.target.checked })} />
            <span>{interp(t('I agree to the {terms} and have read the {privacy}.'), { terms: <Link className="linkish" to="/legal/terms" target="_blank">{t('Terms of Use')}</Link>, privacy: <Link className="linkish" to="/legal/privacy" target="_blank">{t('Privacy Policy')}</Link> })}</span>
          </label>
          <ErrorLine text={err} />
          <button className="btn primary tall" disabled={busy || tooYoung}>{busy ? t('Creating…') : t('Create account')}</button>
        </form>
      ) : <ErrorLine text={err} />}
      <p className="muted center" style={{ margin: 0 }}>{t('Already have an account?')} {onSwitch
        ? <button type="button" className="linkish" onClick={onSwitch}>{t('Log in')}</button>
        : <Link className="linkish" to={loginHref}>{t('Log in')}</Link>}</p>
    </Shell>
  );
}

/** /auth/forgot: Supabase emails a recovery link that comes back through /auth/callback?flow=recovery. */
export function SupabaseForgot() {
  const [params] = useSearchParams();
  const [email, setEmail] = useState(params.get('email') ?? '');
  const [sent, setSent] = useState(false);
  const [busy, setBusy] = useState(false);
  const send = async () => {
    setBusy(true);
    try {
      const c = await getSupabase();
      await c.auth.resetPasswordForEmail(email.trim(), { redirectTo: callbackUrl(safeNext(params.get('next')), 'recovery') });
    } catch {
      /* same answer either way (no enumeration) */
    } finally {
      setBusy(false);
      setSent(true);
    }
  };
  if (sent) {
    return (
      <Shell title={t('Check your email')} sub={interp(t('If an account exists for {email}, we sent a link to reset the password. Open it in this browser.'), { email: <b>{email}</b> })}>
        <Link className="btn tall" to="/login">{t('Back to log in')}</Link>
      </Shell>
    );
  }
  return (
    <Shell title={t('Reset your password')} sub={t('We’ll email you a link to choose a new one.')}>
      <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void send(); }}>
        <label className="field">{t('Email')}<input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoFocus /></label>
        <button className="btn primary tall" disabled={busy}>{t('Send reset link')}</button>
      </form>
    </Shell>
  );
}

/**
 * /auth/reset after a recovery link (the callback exchanged its code). Sets the password, signs out the other Supabase
 * sessions, then bridges with revokeOtherSessions: like the built-in reset, only this device stays signed in. Each
 * step runs once: when the bridge fails, "Try again" only bridges again (review C6), since the password is already saved.
 */
export function SupabaseReset() {
  const [params] = useSearchParams();
  const toast = useToast();
  const f = useSupabaseFinish(params.get('next'));
  // Whose password this sets (shown, so a link for someone else's account is obvious); null: no session.
  const [account, setAccount] = useState<{ email: string } | null | undefined>(undefined);
  const [pw, setPw] = useState('');
  const [pw2, setPw2] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [stepUp, setStepUp] = useState(false);
  const [saved, setSaved] = useState(false);
  const done = useRef(new Set<string>());
  useEffect(() => {
    getSupabase()
      .then(async (c) => {
        const s = (await c.auth.getSession()).data.session;
        setAccount(s ? { email: s.user.email ?? '' } : null);
      })
      .catch(() => setAccount(null));
  }, []);
  const save = async () => {
    setBusy(true);
    setErr('');
    try {
      const c = await getSupabase();
      await resumeSteps(done.current, [
        ['password', async () => {
          const r = await c.auth.updateUser({ password: pw });
          if (r.error?.code === 'insufficient_aal') {
            setStepUp(true);
            return false;
          }
          must(r);
          setSaved(true);
          toast(t('Password saved'));
        }],
        // Best effort, as before: the bridge's revokeOtherSessions still ends the other Pinselo sessions.
        ['others', async () => void (await c.auth.signOut({ scope: 'others' }))],
        ['bridge', () => f.finish({ revokeOtherSessions: true })],
      ]);
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  if (f.step.kind !== 'form') return <FinishStep f={f} />;
  if (stepUp) {
    return (
      <Shell title={t('Two-step verification')} sub={t('Enter the 6-digit code from your authenticator app.')}>
        <SupabaseMfaStep onDone={() => { setStepUp(false); return save(); }} />
      </Shell>
    );
  }
  if (saved) {
    return (
      <Shell title={t('Password saved')} sub={account?.email}>
        {busy ? <div className="spinner" /> : (
          <>
            <ErrorLine text={err} />
            <button className="btn primary tall" onClick={() => void save()}>{t('Try again')}</button>
            <Link className="btn ghost" to="/login">{t('Back to log in')}</Link>
          </>
        )}
      </Shell>
    );
  }
  if (account === undefined) return <Shell title={t('Choose a new password')}><div className="spinner" /></Shell>;
  if (!account) return <Shell title={t('That link didn’t work')} sub={t('This link has expired or was already used.')}><Link className="btn primary tall" to="/auth/forgot">{t('Send a new link')}</Link></Shell>;
  return (
    <Shell title={t('Choose a new password')} sub={account.email}>
      <form className="col" style={{ gap: 12 }} onSubmit={(e) => {
        e.preventDefault();
        if (pw !== pw2) return setErr(t('The passwords don’t match.'));
        void save();
      }}>
        <label className="field">{t('New password')}<input className="input" type="password" minLength={10} required value={pw} onChange={(e) => setPw(e.target.value)} autoComplete="new-password" autoFocus /></label>
        <label className="field">{t('Repeat it')}<input className="input" type="password" required value={pw2} onChange={(e) => setPw2(e.target.value)} autoComplete="new-password" /></label>
        <ErrorLine text={err} />
        <button className="btn primary tall" disabled={busy}>{t('Change password')}</button>
      </form>
    </Shell>
  );
}
