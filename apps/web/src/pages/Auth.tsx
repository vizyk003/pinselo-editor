/**
 * Sign-in and account recovery (PL-001/004): password, magic link, passkeys, Google/Apple/SSO, TOTP step,
 * sign-up with age gate and terms, email verification, password reset and team-invite landing.
 * When the server uses Supabase Auth (plan 12), Login, Signup, ForgotPassword and ResetPassword render the
 * variants in auth/SupabaseAuth.tsx instead; a spinner shows until the mode is known.
 */
import { BRAND } from '@oc/config';
import { startAuthentication } from '@simplewebauthn/browser';
import { KeyRound, Mail } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { api, ApiError, authApi, teamApi, type AuthProviders, type LoginResult, type User } from '../api';
import { useAuth } from '../App';
import { retryAuthConfig, useAuthConfigState, type AuthConfig, type AuthConfigState } from '../auth/authConfig';
import { safeNext, signupClosedText } from '../auth/redirect';
import { SupabaseForgot, SupabaseLogin, SupabaseReset, SupabaseSignup, GoogleMark } from '../auth/SupabaseAuth';
import { interp } from '../components/ui';
import { t } from '../i18n';

const MIN_AGE = 13;
const thisYear = new Date().getFullYear();
const YEARS = Array.from({ length: 100 }, (_, i) => thisYear - i);

const msg = (e: unknown) => (e instanceof ApiError ? (e.code === 'signup_closed' ? signupClosedText() : e.message) : t('Something went wrong. Try again.'));

export function Shell({ title, sub, children, embedded }: { title: ReactNode; sub?: ReactNode; children: ReactNode; embedded?: boolean }) {
  // Embedded: inside the sign-in dialog (AuthGate) instead of a full page.
  if (embedded) {
    return (
      <div className="col auth-embedded" style={{ gap: 16 }}>
        <div>
          <h2 className="auth-title" style={{ fontSize: 22 }}>{title}</h2>
          {sub && <p className="muted" style={{ margin: 0 }}>{sub}</p>}
        </div>
        {children}
      </div>
    );
  }
  return (
    <div className="auth">
      <div className="auth-card">
        <Link to="/" className="logo">
          <img src="/favicon.svg" alt="" />
          {BRAND.name}
        </Link>
        <div>
          <h1 className="auth-title">{title}</h1>
          {sub && <p className="muted" style={{ margin: 0 }}>{sub}</p>}
        </div>
        {children}
      </div>
    </div>
  );
}

export function ErrorLine({ text }: { text: string }) {
  return text ? <div className="form-error" role="alert">{text}</div> : null;
}

/** Shared: finish a login result — either a session or a TOTP challenge. */
function useFinish(nextOverride?: string) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const [params] = useSearchParams();
  return (r: LoginResult, onMfa: (ticket: string) => void) => {
    if ('mfaRequired' in r) return onMfa(r.ticket);
    setUser(r.user);
    nav(safeNext(nextOverride ?? params.get('next')), { replace: true });
  };
}

/** Props for using the forms inside the sign-in dialog: where to go afterwards, and how to switch forms. */
export interface EmbeddedAuth { embedded?: boolean; next?: string; onSwitch?: () => void }

/** The built-in code step. `errorText`: how errors read (Supabase mode passes its translated text instead of the server's). */
export function MfaStep({ ticket, onDone, errorText = msg }: { ticket: string; onDone: (u: User) => void; errorText?: (e: unknown) => string }) {
  const [code, setCode] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [recovery, setRecovery] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr('');
    try {
      onDone((await authApi.mfa(ticket, code)).user);
    } catch (x) {
      setErr(errorText(x));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="col" style={{ gap: 14 }} onSubmit={submit}>
      <label className="field">
        {recovery ? t('Recovery code') : t('Code from your authenticator app')}
        <input
          className="input" value={code} onChange={(e) => setCode(e.target.value)} autoFocus required
          inputMode={recovery ? 'text' : 'numeric'} autoComplete="one-time-code" placeholder={recovery ? 'XXXXX-XXXXX' : '123 456'}
        />
      </label>
      <ErrorLine text={err} />
      <button className="btn primary tall" disabled={busy || !code}>{busy ? t('Checking…') : t('Verify')}</button>
      <button type="button" className="linkish" onClick={() => setRecovery((r) => !r)}>
        {recovery ? t('Use an authenticator code') : t('Use a recovery code instead')}
      </button>
    </form>
  );
}

/** Until the sign-in mode is known: a spinner, never the built-in form (no flash before the Supabase variant). */
function AuthLoading({ embedded }: { embedded?: boolean }) {
  return embedded ? <div className="spinner" /> : <div className="auth"><div className="auth-card"><div className="spinner" /></div></div>;
}

/**
 * The sign-in options couldn't load (offline, server unreachable): say so and offer to try again (it also retries
 * when the browser is back online), never the built-in forms, which a Supabase server would refuse (review C4).
 */
export function SignInUnavailable({ embedded, compact }: { embedded?: boolean; compact?: boolean }) {
  const retry = <button type="button" className={compact ? 'btn' : 'btn primary tall'} onClick={retryAuthConfig}>{t('Try again')}</button>;
  if (compact) return <div className="col" style={{ gap: 12 }}><ErrorLine text={t('Sign-in is unavailable right now. Try again in a minute.')} /><div>{retry}</div></div>;
  return <Shell embedded={embedded} title={t('Can’t load sign-in')} sub={t('Check your connection and try again.')}>{retry}</Shell>;
}

/** Loading or failed config: the spinner or the "try again" panel. */
function AuthPending({ state, embedded }: { state: Exclude<AuthConfigState, { kind: 'ready' }>; embedded?: boolean }) {
  return state.kind === 'failed' ? <SignInUnavailable embedded={embedded} /> : <AuthLoading embedded={embedded} />;
}

export function Login(p: EmbeddedAuth = {}) {
  const s = useAuthConfigState();
  if (s.kind !== 'ready') return <AuthPending state={s} embedded={p.embedded} />;
  const config = s.config;
  return config.supabase ? <SupabaseLogin {...p} config={config} /> : <BuiltinLogin {...p} config={config} />;
}

function BuiltinLogin({ embedded, next: nextProp, onSwitch, config }: EmbeddedAuth & { config: AuthConfig }) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const finish = useFinish(nextProp);
  const providers: AuthProviders | null = config;
  const [mode, setMode] = useState<'password' | 'magic' | 'sent' | 'mfa' | 'dev'>(params.get('mfa') ? 'mfa' : 'password');
  const [ticket, setTicket] = useState(params.get('mfa') ?? '');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [name, setName] = useState('');
  const [err, setErr] = useState(params.get('error') === 'oauth' ? t('Sign-in with that provider failed. Try again.') : params.get('error') === 'unverified_email' ? t('That account’s email isn’t verified with the provider.') : params.get('error') === 'signup_closed' ? signupClosedText() : '');
  const [busy, setBusy] = useState(false);
  const next = safeNext(nextProp ?? params.get('next'));

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setErr('');
    try {
      await fn();
    } catch (x) {
      setErr(msg(x));
    } finally {
      setBusy(false);
    }
  };
  const onMfa = (tkt: string) => {
    setTicket(tkt);
    setMode('mfa');
  };

  const passkey = () =>
    run(async () => {
      const options = await authApi.passkeyLoginOptions();
      let response;
      try {
        response = await startAuthentication({ optionsJSON: options as never });
      } catch {
        throw new ApiError(400, 'cancelled', t('Passkey sign-in was cancelled.'));
      }
      finish(await authApi.passkeyLoginVerify(response), onMfa);
    });

  if (mode === 'mfa') {
    return (
      <Shell embedded={embedded} title={t('Two-step verification')} sub={t('Enter the 6-digit code from your authenticator app.')}>
        <MfaStep ticket={ticket} onDone={(u) => { setUser(u); nav(next, { replace: true }); }} />
      </Shell>
    );
  }
  if (mode === 'sent') {
    return (
      <Shell embedded={embedded} title={t('Check your email')} sub={interp(t('We sent a sign-in link to {email}. It works once and expires in 15 minutes.'), { email: <b>{email}</b> })}>
        {providers?.devLogin && <Link className="btn tall" to={`/dev/mail?to=${encodeURIComponent(email)}`}>{t('Open the dev mailbox')}</Link>}
        <button className="linkish" onClick={() => setMode('magic')}>{t('Use a different email')}</button>
      </Shell>
    );
  }

  return (
    <Shell embedded={embedded} title={embedded ? t('Log in to continue') : t('Log in to {brand}', { brand: BRAND.name })} sub={t('Design anything. Publish anywhere.')}>
      <div className="col" style={{ gap: 8 }}>
        {providers?.providers.map((p) => (
          <a key={p.id} className="btn tall" href={`/api/auth/oauth/${p.id}/start?next=${encodeURIComponent(next)}`}>
            {p.id === 'google' && <GoogleMark />}
            {t('Continue with {provider}', { provider: p.label })}
          </a>
        ))}
        {providers?.passkeys && window.PublicKeyCredential && (
          <button className="btn tall" onClick={passkey} disabled={busy}>
            <KeyRound size={16} /> {t('Sign in with a passkey')}
          </button>
        )}
      </div>
      <div className="or"><span>{t('or')}</span></div>

      {mode === 'password' && (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void run(async () => finish(await authApi.login(email.trim(), password), onMfa)); }}>
          <label className="field">
            {t('Email')}
            <input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username" autoFocus />
          </label>
          <label className="field">
            <span className="row">{t('Password')} <span className="grow" /><Link to={`/auth/forgot?email=${encodeURIComponent(email)}`} className="linkish">{t('Forgot?')}</Link></span>
            <input className="input" type="password" required value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
          </label>
          <ErrorLine text={err} />
          <button className="btn primary tall" disabled={busy}>{busy ? t('Signing in…') : t('Log in')}</button>
          <button type="button" className="btn tall" onClick={() => setMode('magic')}><Mail size={16} /> {t('Email me a sign-in link')}</button>
        </form>
      )}

      {mode === 'magic' && (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void run(async () => { await authApi.magic(email.trim(), next); setMode('sent'); }); }}>
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
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void run(async () => finish(await api.login(email.trim(), name.trim() || email.split('@')[0]!), onMfa)); }}>
          <label className="field">{t('Name')}<input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder={t('Ada Lovelace')} /></label>
          <label className="field">{t('Email')}<input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoFocus /></label>
          <ErrorLine text={err} />
          <button className="btn primary tall" disabled={busy}>{t('Continue (development)')}</button>
          <button type="button" className="btn ghost" onClick={() => setMode('password')}>{t('Back')}</button>
        </form>
      )}

      <p className="muted center" style={{ margin: 0 }}>
        {t('New here?')} {onSwitch
          ? <button type="button" className="linkish" onClick={onSwitch}>{t('Create an account')}</button>
          : <Link className="linkish" to={`/signup${next !== '/' ? `?next=${encodeURIComponent(next)}` : ''}`}>{t('Create an account')}</Link>}
      </p>
      {providers?.devLogin && mode !== 'dev' && (
        <button className="linkish faint" style={{ fontSize: 12 }} onClick={() => setMode('dev')}>{t('Development sign-in without a password')}</button>
      )}
    </Shell>
  );
}

export function Signup(p: EmbeddedAuth = {}) {
  const s = useAuthConfigState();
  if (s.kind !== 'ready') return <AuthPending state={s} embedded={p.embedded} />;
  const config = s.config;
  if (!config.supabase) return <BuiltinSignup {...p} config={config} />;
  // Closed registration (Supabase's disable_signup): existing accounts can still log in.
  return config.supabase.signup ? <SupabaseSignup {...p} config={config} /> : <SupabaseLogin {...p} config={config} />;
}

function BuiltinSignup({ embedded, next: nextProp, onSwitch, config }: EmbeddedAuth & { config: AuthConfig }) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const [f, setF] = useState({ name: '', email: '', password: '', birthYear: 0, terms: false });
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const tooYoung = f.birthYear > 0 && thisYear - f.birthYear < MIN_AGE;
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!f.birthYear) return setErr(t('Select your birth year.'));
    if (tooYoung) return setErr(t('You need to be at least {age} to use {brand}.', { age: MIN_AGE, brand: BRAND.name }));
    if (!f.terms) return setErr(t('Accept the Terms to continue.'));
    setBusy(true);
    setErr('');
    try {
      const r = await authApi.signup({ name: f.name.trim(), email: f.email.trim(), password: f.password, birthYear: f.birthYear, acceptTerms: true });
      setUser(r.user);
      nav(safeNext(nextProp ?? params.get('next')), { replace: true });
    } catch (x) {
      setErr(msg(x));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Shell embedded={embedded} title={t('Create your account')} sub={t('Free forever for personal use.')}>
      {config.signup === 'invite' && <p className="muted" style={{ margin: 0 }}>{signupClosedText()}</p>}
      {config.providers.length > 0 && (
        <>
          <div className="col" style={{ gap: 8 }}>
            {config.providers.map((p) => (
              <a key={p.id} className="btn tall" href={`/api/auth/oauth/${p.id}/start?next=${encodeURIComponent(safeNext(nextProp ?? params.get('next')))}`}>
                {p.id === 'google' && <GoogleMark />}
            {t('Continue with {provider}', { provider: p.label })}
              </a>
            ))}
          </div>
          <div className="or"><span>{t('or')}</span></div>
        </>
      )}
      <form className="col" style={{ gap: 12 }} onSubmit={submit}>
        <label className="field">{t('Name')}<input className="input" required value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoComplete="name" autoFocus /></label>
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
      <p className="muted center" style={{ margin: 0 }}>{t('Already have an account?')} {onSwitch
        ? <button type="button" className="linkish" onClick={onSwitch}>{t('Log in')}</button>
        : <Link className="linkish" to={`/login${params.get('next') ? `?next=${encodeURIComponent(params.get('next')!)}` : ''}`}>{t('Log in')}</Link>}</p>
    </Shell>
  );
}

/** /auth/magic?token= — signs in, or collects name/age/terms for a brand-new account. */
export function MagicLanding() {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const token = params.get('token') ?? '';
  const [state, setState] = useState<{ kind: 'checking' } | { kind: 'profile'; email: string } | { kind: 'mfa'; ticket: string; next: string } | { kind: 'error'; text: string }>({ kind: 'checking' });
  const [f, setF] = useState({ name: '', birthYear: 0, terms: false });
  const started = useRef(false);

  const handle = (r: Awaited<ReturnType<typeof authApi.magicVerify>>) => {
    if ('needsProfile' in r) return setState({ kind: 'profile', email: r.email });
    if ('mfaRequired' in r) return setState({ kind: 'mfa', ticket: r.ticket, next: safeNext((r as { next?: string }).next ?? null) });
    setUser(r.user);
    nav(safeNext((r as { next?: string }).next ?? null), { replace: true });
  };
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    authApi.magicVerify({ token }).then(handle).catch((e) => setState({ kind: 'error', text: msg(e) }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token]);

  if (state.kind === 'checking') return <Shell title={t('Signing you in…')}><div className="spinner" /></Shell>;
  if (state.kind === 'error') return <Shell title={t('That link didn’t work')} sub={state.text}><Link className="btn primary tall" to="/login">{t('Back to log in')}</Link></Shell>;
  if (state.kind === 'mfa') return <Shell title={t('Two-step verification')}><MfaStep ticket={state.ticket} onDone={(u) => { setUser(u); nav(state.next, { replace: true }); }} /></Shell>;
  const tooYoung = f.birthYear > 0 && thisYear - f.birthYear < MIN_AGE;
  return (
    <Shell title={t('Finish creating your account')} sub={state.email}>
      <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void authApi.magicVerify({ token, name: f.name, birthYear: f.birthYear, acceptTerms: f.terms }).then(handle).catch((x) => setState({ kind: 'error', text: msg(x) })); }}>
        <label className="field">{t('Name')}<input className="input" required value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} autoFocus /></label>
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
        <button className="btn primary tall" disabled={!f.name || !f.birthYear || !f.terms || tooYoung}>{t('Create account')}</button>
      </form>
    </Shell>
  );
}

export function VerifyLanding() {
  const [params] = useSearchParams();
  const { user, setUser } = useAuth();
  const [state, setState] = useState<'checking' | 'ok' | string>('checking');
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    authApi.verify(params.get('token') ?? '')
      .then(() => {
        setState('ok');
        if (user) setUser({ ...user, verified: true });
      })
      .catch((e) => setState(msg(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (state === 'checking') return <Shell title={t('Confirming your email…')}><div className="spinner" /></Shell>;
  if (state === 'ok') return <Shell title={t('Email confirmed')} sub={t('Thanks! Your account is all set.')}><Link className="btn primary tall" to="/">{t('Start designing')}</Link></Shell>;
  return <Shell title={t('That link didn’t work')} sub={state}><Link className="btn primary tall" to="/settings/account">{t('Send a new link')}</Link></Shell>;
}

export function ForgotPassword() {
  const s = useAuthConfigState();
  if (s.kind !== 'ready') return <AuthPending state={s} />;
  return s.config.supabase ? <SupabaseForgot /> : <BuiltinForgotPassword />;
}

function BuiltinForgotPassword() {
  const [params] = useSearchParams();
  const [email, setEmail] = useState(params.get('email') ?? '');
  const [sent, setSent] = useState(false);
  const [busy, setBusy] = useState(false);
  if (sent) {
    return (
      <Shell title={t('Check your email')} sub={interp(t('If an account exists for {email}, we sent a link to reset the password. It expires in 30 minutes.'), { email: <b>{email}</b> })}>
        <Link className="btn tall" to="/login">{t('Back to log in')}</Link>
      </Shell>
    );
  }
  return (
    <Shell title={t('Reset your password')} sub={t('We’ll email you a link to choose a new one.')}>
      <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); setBusy(true); void authApi.resetRequest(email.trim()).finally(() => { setBusy(false); setSent(true); }); }}>
        <label className="field">{t('Email')}<input className="input" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} autoFocus /></label>
        <button className="btn primary tall" disabled={busy}>{t('Send reset link')}</button>
      </form>
      <p className="muted center" style={{ margin: 0 }}><Link className="linkish" to="/login">{t('Back to log in')}</Link></p>
    </Shell>
  );
}

export function ResetPassword() {
  const s = useAuthConfigState();
  if (s.kind !== 'ready') return <AuthPending state={s} />;
  return s.config.supabase ? <SupabaseReset /> : <BuiltinResetPassword />;
}

function BuiltinResetPassword() {
  const [params] = useSearchParams();
  const [pw, setPw] = useState('');
  const [pw2, setPw2] = useState('');
  const [err, setErr] = useState('');
  const [done, setDone] = useState(false);
  if (done) return <Shell title={t('Password changed')} sub={t('You’ve been signed out everywhere. Log in with your new password.')}><Link className="btn primary tall" to="/login">{t('Log in')}</Link></Shell>;
  return (
    <Shell title={t('Choose a new password')}>
      <form className="col" style={{ gap: 12 }} onSubmit={(e) => {
        e.preventDefault();
        if (pw !== pw2) return setErr(t('The passwords don’t match.'));
        setErr('');
        void authApi.reset(params.get('token') ?? '', pw).then(() => setDone(true)).catch((x) => setErr(msg(x)));
      }}>
        <label className="field">{t('New password')}<input className="input" type="password" minLength={10} required value={pw} onChange={(e) => setPw(e.target.value)} autoComplete="new-password" autoFocus /></label>
        <label className="field">{t('Repeat it')}<input className="input" type="password" required value={pw2} onChange={(e) => setPw2(e.target.value)} autoComplete="new-password" /></label>
        <ErrorLine text={err} />
        <button className="btn primary tall">{t('Change password')}</button>
      </form>
    </Shell>
  );
}

/** /invite?token= — accept a team invite (signing in first if needed). */
export function InviteLanding() {
  const { user } = useAuth();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const token = params.get('token') ?? '';
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  if (!user) {
    const next = encodeURIComponent(`/invite?token=${token}`);
    return (
      <Shell title={t('You’re invited')} sub={t('Log in or create an account with the email address the invite was sent to.')}>
        <Link className="btn primary tall" to={`/login?next=${next}`}>{t('Log in')}</Link>
        <Link className="btn tall" to={`/signup?next=${next}`}>{t('Create an account')}</Link>
      </Shell>
    );
  }
  return (
    <Shell title={t('Join the team')} sub={interp(t('You’re signed in as {email}.'), { email: <b>{user.email}</b> })}>
      <ErrorLine text={err} />
      <button className="btn primary tall" disabled={busy} onClick={() => {
        setBusy(true);
        teamApi.accept(token).then((r) => nav(`/team/${r.workspaceId}`, { replace: true })).catch((x) => { setErr(msg(x)); setBusy(false); });
      }}>{t('Accept invite')}</button>
      <Link className="btn ghost" to="/">{t('Not now')}</Link>
    </Shell>
  );
}
