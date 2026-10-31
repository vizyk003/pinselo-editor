/**
 * Settings when the server uses Supabase Auth (plan 12 §C1, §C2). Password, email and two-step verification live in
 * Supabase and change through supabase-js; the Pinselo endpoints that need fresh proof (log out everywhere else,
 * delete account) get the current Supabase token. Without a usable Supabase session for this account, the re-auth
 * panel asks the user to sign in again, then the action is retried. "This account's session" means its linked Supabase
 * user (review C1): the Supabase email can differ from the one here.
 */
import type { Factor, Session, SupabaseClient } from '@supabase/supabase-js';
import { KeyRound, ShieldCheck } from 'lucide-react';
import QRCode from 'qrcode';
import { useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { accountApi, type AccountInfo } from '../api';
import { useAuth } from '../App';
import { interp, Modal, timeAgo, useToast } from '../components/ui';
import { t, tn } from '../i18n';
import { ErrorLine } from '../pages/Auth';
import { Panel, SessionsPanel, TotpDisable } from '../pages/Settings';
import { retryAuthConfig, useAuthConfigState } from './authConfig';
import { callbackUrl } from './redirect';
import { isOwnSession, reauthEmail, sessionKey, type SessionAccount } from './sessionRules';
import { authErrorText, errorCode, must, OAuthButtons, SupabaseMfaStep } from './SupabaseAuth';
import { deleteAccountSupabase, revokeOthersWithToken, syncSupabase, type AccountAuth } from './supabaseApi';
import { getSupabase } from './supabaseClient';
import { signOutSupabaseLocal } from './session';

/** `GET /api/account` → `auth` (absent on servers without Supabase support). */
export const accountAuth = (info: AccountInfo): AccountAuth | undefined => (info as AccountInfo & { auth?: AccountAuth }).auth;

/** The signed-in account as the session rules see it: its email here and its linked Supabase user. */
function useSessionAccount(auth: AccountAuth | undefined): SessionAccount {
  const { user } = useAuth();
  return { email: user?.email ?? '', supabaseId: auth?.supabaseId, supabaseEmail: auth?.supabaseEmail };
}

/** After `sync`: when Supabase's address for this account is taken here by another account, say so (review C1). */
function emailConflictText(r: { emailConflict?: true }, supabaseEmail: string | undefined): string {
  return r.emailConflict && supabaseEmail
    ? t('You now sign in with {email}. Another account here already uses that address, so the email on this account stays the same.', { email: supabaseEmail })
    : '';
}

/** This browser's Supabase session, kept current; undefined while supabase-js loads. */
export function useSupabaseSession(): Session | null | undefined {
  const [session, setSession] = useState<Session | null | undefined>(undefined);
  useEffect(() => {
    let live = true;
    let unsubscribe = () => {};
    getSupabase()
      .then(async (sb) => {
        const current = (await sb.auth.getSession()).data.session;
        if (!live) return;
        setSession(current);
        const { subscription } = sb.auth.onAuthStateChange((_event, s) => live && setSession(s)).data;
        unsubscribe = () => subscription.unsubscribe();
      })
      .catch(() => live && setSession(null));
    return () => {
      live = false;
      unsubscribe();
    };
  }, []);
  return session;
}

// Error codes (Pinselo and Supabase) that mean "sign in again" or "step up to aal2".
const REAUTH = new Set(['reauth_required', 'wrong_account', 'session_ended', 'invalid_token', 'session_not_found', 'session_expired', 'refresh_token_not_found', 'refresh_token_already_used', 'bad_jwt']);
const STEP_UP = new Set(['mfa_required', 'insufficient_aal']);

/** Throws `insufficient_aal` when this session can still step up (a verified factor and aal1), e.g. before removing a factor. */
async function requireAal2(sb: SupabaseClient) {
  const { data } = await sb.auth.mfa.getAuthenticatorAssuranceLevel();
  if (data?.nextLevel === 'aal2' && data.currentLevel !== 'aal2') throw Object.assign(new Error('insufficient_aal'), { code: 'insufficient_aal' });
}

/**
 * Runs an action that needs this account's live Supabase session. Without one (or when the server asks for a recent
 * sign-in or a second factor) it shows the re-auth panel or the MFA step instead, and retries when that is done.
 */
export function useFreshSupabase(auth: AccountAuth | undefined) {
  const account = useSessionAccount(auth);
  const toast = useToast();
  const [need, setNeed] = useState<{ kind: 'reauth' | 'mfa'; retry: () => void } | null>(null);
  const run = async (action: (sb: SupabaseClient, token: string) => Promise<void>, onError: (text: string) => void = toast, checked = false): Promise<void> => {
    const again = () => {
      setNeed(null);
      void run(action, onError, true);
    };
    try {
      const sb = await getSupabase();
      const session = (await sb.auth.getSession()).data.session;
      // A Supabase session someone else left in this browser must not act for this account.
      if (session && !checked && !isOwnSession(session.user, account)) await signOutSupabaseLocal();
      else if (session) return await action(sb, session.access_token);
      setNeed({ kind: 'reauth', retry: again });
    } catch (e) {
      const code = errorCode(e);
      if (code === 'wrong_account') await signOutSupabaseLocal();
      if (REAUTH.has(code)) return setNeed({ kind: 'reauth', retry: again });
      if (STEP_UP.has(code)) return setNeed({ kind: 'mfa', retry: again });
      onError(authErrorText(e));
    }
  };
  const cancel = () => setNeed(null);
  const title = need?.kind === 'mfa' ? t('Two-step verification') : t('Confirm it’s you');
  const panel: ReactNode = need && (need.kind === 'reauth'
    ? <ReauthPanel auth={auth} onDone={need.retry} />
    : <><p className="muted" style={{ margin: 0 }}>{t('Enter the 6-digit code from your authenticator app.')}</p><SupabaseMfaStep onDone={need.retry} /></>);
  const modal: ReactNode = need && <Modal title={title} onClose={cancel} size="sm">{panel}</Modal>;
  return { run, need, title, panel, modal, cancel };
}

/**
 * Sign in again as this account (plan 12 §C2): password in place, or an email link / OAuth that returns through
 * /auth/callback?flow=reauth. Either way `sync` confirms the Supabase user is this account (and links it if it isn't yet).
 * The email starts as the one this account signs in with at Supabase and can be changed (review C1): it may differ
 * from the one here, and `sync` refuses any other account.
 */
export function ReauthPanel({ auth, onDone }: { auth: AccountAuth | undefined; onDone: () => void }) {
  const configState = useAuthConfigState();
  const config = configState.kind === 'ready' ? configState.config : null;
  const sb = config?.supabase;
  const linked = Boolean(auth?.supabaseLinked);
  const { user, setUser } = useAuth();
  const toast = useToast();
  const account = useSessionAccount(auth);
  const session = useSupabaseSession();
  const { pathname } = useLocation();
  const [email, setEmail] = useState(account.email);
  const edited = useRef(false);
  const key = sessionKey(session);
  useEffect(() => {
    if (!edited.current) setEmail(reauthEmail(session, account));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, account.email, account.supabaseId, account.supabaseEmail]);
  const [password, setPassword] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const [sent, setSent] = useState(false);
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
  const withPassword = () =>
    run(async () => {
      const c = await getSupabase();
      must(await c.auth.signInWithPassword({ email: email.trim(), password }));
      const current = (await c.auth.getSession()).data.session;
      try {
        const r = await syncSupabase(current?.access_token ?? '');
        setUser(r.user);
        const conflict = emailConflictText(r, current?.user.email);
        if (conflict) toast(conflict);
      } catch (e) {
        if (errorCode(e) === 'wrong_account') await signOutSupabaseLocal();
        throw e;
      }
      onDone();
    });
  const withLink = () =>
    run(async () => {
      const c = await getSupabase();
      // An account not yet linked may have no Supabase user: the link creates it (sync then links it to this account),
      // but only for this account's own address.
      const own = email.trim().toLowerCase() === (user?.email ?? '').toLowerCase();
      must(await c.auth.signInWithOtp({ email: email.trim(), options: { shouldCreateUser: !linked && own && Boolean(sb?.signup), emailRedirectTo: callbackUrl(pathname, 'reauth') } }));
      setSent(true);
    });
  if (sent) return <p className="muted" style={{ margin: 0 }}>{interp(t('We sent a sign-in link to {email}. Open it in this browser to continue.'), { email: <b>{email.trim()}</b> })}</p>;
  if (configState.kind === 'loading') return <div className="spinner" />;
  // The sign-in options didn't load: say so and retry (review C4).
  if (configState.kind === 'failed') {
    return (
      <div className="col" style={{ gap: 12 }}>
        <ErrorLine text={t('Sign-in is unavailable right now. Try again in a minute.')} />
        <div><button type="button" className="btn" onClick={retryAuthConfig}>{t('Try again')}</button></div>
      </div>
    );
  }
  return (
    <div className="col" style={{ gap: 12 }}>
      <p className="muted" style={{ margin: 0 }}>{t('For your security, sign in again to continue.')}</p>
      {config && <OAuthButtons config={config} next={pathname} flow="reauth" onError={setErr} />}
      {sb?.email ? (
        <form className="col" style={{ gap: 12 }} onSubmit={(e) => { e.preventDefault(); void withPassword(); }}>
          <label className="field">{t('Email')}<input className="input" type="email" required value={email} onChange={(e) => { edited.current = true; setEmail(e.target.value); }} autoComplete="username" /></label>
          <label className="field">{t('Password')}<input className="input" type="password" required value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" autoFocus /></label>
          <ErrorLine text={err} />
          <button className="btn primary" disabled={busy}>{busy ? t('Checking…') : t('Continue')}</button>
          <button type="button" className="btn ghost" disabled={busy} onClick={() => void withLink()}>{t('Email me a sign-in link')}</button>
        </form>
      ) : <ErrorLine text={err} />}
    </div>
  );
}

/** Settings → Account in Supabase mode: Supabase's resend for the unconfirmed-email banner. */
export async function resendConfirmation(email: string): Promise<void> {
  const c = await getSupabase();
  must(await c.auth.resend({ type: 'signup', email, options: { emailRedirectTo: callbackUrl('/settings/account') } }));
}

/** Settings → Account: change the email through Supabase; the confirmation link comes back as flow=email_change and syncs. */
export function SupabaseEmailField({ info }: { info: AccountInfo }) {
  const { user } = useAuth();
  const toast = useToast();
  const fresh = useFreshSupabase(accountAuth(info));
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState('');
  const [err, setErr] = useState('');
  const save = (e: FormEvent) => {
    e.preventDefault();
    setErr('');
    void fresh.run(async (sb) => {
      must(await sb.auth.updateUser({ email: value.trim() }, { emailRedirectTo: callbackUrl('/settings/account', 'email_change') }));
      toast(t('Confirm the change from the email we sent.'));
      setEditing(false);
      setValue('');
    }, setErr);
  };
  return (
    <>
      <label className="field">
        {t('Email')}
        <div className="row">
          <input className="input grow" value={user?.email ?? info.user.email} disabled />
          {!editing && <button type="button" className="btn" onClick={() => setEditing(true)}>{t('Change')}</button>}
        </div>
      </label>
      {editing && (
        <form className="row" onSubmit={save}>
          <input className="input grow" type="email" required autoComplete="email" placeholder={t('New email address')} value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
          <button className="btn primary">{t('Save')}</button>
          <button type="button" className="btn ghost" onClick={() => { setEditing(false); setErr(''); }}>{t('Cancel')}</button>
        </form>
      )}
      {err && <div className="form-error">{err}</div>}
      {fresh.modal}
    </>
  );
}

/** Settings → Login & security in Supabase mode. */
export function SupabaseSecurityTab({ info, reload }: { info: AccountInfo; reload: () => void }) {
  const toast = useToast();
  const fresh = useFreshSupabase(accountAuth(info));
  const [pw, setPw] = useState({ current: '', next: '', nonce: '' });
  const [nonceSent, setNonceSent] = useState(false);
  const [pwErr, setPwErr] = useState('');
  const [sessionsVersion, setSessionsVersion] = useState(0);
  const [disable, setDisable] = useState(false);

  const savePassword = (e: FormEvent) => {
    e.preventDefault();
    setPwErr('');
    void fresh.run(async (sb) => {
      const r = await sb.auth.updateUser({ password: pw.next, ...(pw.current ? { current_password: pw.current } : {}), ...(nonceSent ? { nonce: pw.nonce.trim() } : {}) });
      if (r.error?.code === 'reauthentication_needed') {
        // Supabase's secure password change: confirm with the code it emails.
        must(await sb.auth.reauthenticate());
        return setNonceSent(true);
      }
      must(r);
      setPw({ current: '', next: '', nonce: '' });
      setNonceSent(false);
      toast(t('Password saved'));
      // Like the built-in change, it signs out the other sessions: Pinselo and Supabase.
      await revokeOthersWithToken((await sb.auth.getSession()).data.session?.access_token);
      setSessionsVersion((v) => v + 1);
    }, setPwErr);
  };

  return (
    <>
      <Panel title={t('Change password')} sub={t('Changing it signs out your other sessions.')}>
        <form className="col" onSubmit={savePassword}>
          <label className="field">
            {t('Current password')}
            <input className="input" type="password" autoComplete="current-password" value={pw.current} onChange={(e) => setPw({ ...pw, current: e.target.value })} />
            <span className="faint" style={{ fontWeight: 400 }}>{t('Leave empty if you don’t have one yet.')}</span>
          </label>
          <label className="field">{t('New password')}<input className="input" type="password" autoComplete="new-password" minLength={10} value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} required /></label>
          {nonceSent && (
            <label className="field">
              {t('Code from your email')}
              <input className="input" inputMode="numeric" autoComplete="one-time-code" value={pw.nonce} onChange={(e) => setPw({ ...pw, nonce: e.target.value })} required autoFocus />
              <span className="faint" style={{ fontWeight: 400 }}>{t('We emailed you a code to confirm this change.')}</span>
            </label>
          )}
          {pwErr && <div className="form-error">{pwErr}</div>}
          <div><button className="btn">{t('Save password')}</button></div>
        </form>
      </Panel>

      <SupabaseMfaPanel fresh={fresh} auth={accountAuth(info)} legacy={info.user.mfa ? (
        <div className="item">
          <ShieldCheck size={18} color="#16a34a" />
          <span className="grow">{t('On')} · {tn(info.recoveryCodesLeft, '{n} recovery code left', '{n} recovery codes left')}</span>
          <button className="btn danger" onClick={() => setDisable(true)}>{t('Turn off')}</button>
        </div>
      ) : null} />

      {info.passkeys.length > 0 && (
        // Passkeys can't sign in with Supabase Auth; leftovers can still be removed.
        <Panel title={t('Passkeys')}>
          <div className="list">
            {info.passkeys.map((p) => (
              <div className="item" key={p.id}>
                <KeyRound size={18} />
                <div className="main"><div>{p.name ?? t('Passkey')}</div><div className="sub">{t('Added {when}', { when: timeAgo(p.created_at) })}</div></div>
                <button className="btn ghost sm danger" onClick={async () => { await accountApi.removePasskey(p.id); reload(); }}>{t('Remove')}</button>
              </div>
            ))}
          </div>
        </Panel>
      )}

      <SessionsPanel version={sessionsVersion} alwaysOffer revokeOthers={(done) => fresh.run(async (_sb, token) => {
        await revokeOthersWithToken(token);
        toast(t('Logged out of other sessions'));
        done();
      })} />
      {disable && <TotpDisable onClose={() => { setDisable(false); reload(); }} />}
      {fresh.modal}
    </>
  );
}

const verifiedTotp = (all: Factor[]) => all.filter((f) => f.factor_type === 'totp' && f.status === 'verified');

/** Supabase TOTP factors: add (QR), remove (needs aal2). Built-in TOTP (`legacy`) can only be turned off in this mode. */
function SupabaseMfaPanel({ fresh, auth, legacy }: { fresh: ReturnType<typeof useFreshSupabase>; auth: AccountAuth | undefined; legacy: ReactNode }) {
  const toast = useToast();
  const account = useSessionAccount(auth);
  const session = useSupabaseSession();
  const [factors, setFactors] = useState<Factor[] | 'signin' | null>(null);
  const [version, setVersion] = useState(0);
  const [adding, setAdding] = useState(false);
  // Whose session, never its token: listing factors may refresh the token, which must not run this again (review C2).
  const key = sessionKey(session);
  useEffect(() => {
    if (session === undefined) return;
    if (!session || !isOwnSession(session.user, account)) return setFactors('signin');
    let live = true;
    getSupabase()
      .then(async (sb) => {
        const r = await sb.auth.mfa.listFactors();
        if (live) setFactors(r.error ? 'signin' : verifiedTotp(r.data.all));
      })
      .catch(() => live && setFactors('signin'));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, account.email, account.supabaseId, version]);
  const reloadFactors = () => setVersion((v) => v + 1);
  const remove = (factorId: string) =>
    fresh.run(async (sb) => {
      await requireAal2(sb);
      must(await sb.auth.mfa.unenroll({ factorId }));
      await sb.auth.refreshSession();
      reloadFactors();
    });
  const add = () =>
    fresh.run(async (sb) => {
      // With a factor already on, Supabase only lets an aal2 session add another.
      await requireAal2(sb);
      setAdding(true);
    });
  const list = Array.isArray(factors) ? factors : [];
  return (
    <Panel title={t('Two-step verification')} sub={t('Ask for a code from an authenticator app when you log in with a password, link or provider.')}>
      {(list.length > 0 || legacy) && (
        <div className="list">
          {list.map((f) => (
            <div className="item" key={f.id}>
              <ShieldCheck size={18} color="#16a34a" />
              <div className="main"><div>{f.friendly_name || t('Authenticator app')}</div>{f.created_at && <div className="sub">{t('Added {when}', { when: timeAgo(f.created_at) })}</div>}</div>
              <button className="btn ghost sm danger" onClick={() => void remove(f.id)}>{t('Remove')}</button>
            </div>
          ))}
          {legacy}
        </div>
      )}
      {factors === null && <div className="spinner" />}
      {factors === 'signin' && <div><button className="btn" onClick={() => void fresh.run(async () => reloadFactors())}>{t('Confirm it’s you')}</button></div>}
      {Array.isArray(factors) && (
        <>
          {/* Supabase TOTP has no recovery codes (plan 12 §H4). */}
          {factors.length === 1 && <p className="muted" style={{ margin: 0 }}>{t('There are no recovery codes. Add a second authenticator app in case you lose your phone.')}</p>}
          <div><button className={factors.length ? 'btn' : 'btn primary'} onClick={() => void add()}>{factors.length ? t('Add an authenticator app') : t('Turn on')}</button></div>
        </>
      )}
      {adding && <SupabaseTotpSetup taken={list.map((f) => f.friendly_name ?? '')} onClose={(added) => { setAdding(false); reloadFactors(); if (added) toast(t('Saved')); }} />}
    </Panel>
  );
}

function SupabaseTotpSetup({ taken, onClose }: { taken: string[]; onClose: (added: boolean) => void }) {
  const [setup, setSetup] = useState<{ id: string; secret: string; qr: string } | null>(null);
  const [code, setCode] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const pending = useRef<string | null>(null);
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    (async () => {
      const sb = await getSupabase();
      // Factors left unverified by an earlier attempt would count toward Supabase's limit.
      for (const f of must(await sb.auth.mfa.listFactors()).all) {
        if (f.factor_type === 'totp' && f.status === 'unverified') await sb.auth.mfa.unenroll({ factorId: f.id });
      }
      let n = taken.length + 1;
      while (taken.includes(`${t('Authenticator app')} ${n}`)) n++;
      const e = must(await sb.auth.mfa.enroll({ factorType: 'totp', friendlyName: `${t('Authenticator app')} ${n}` }));
      pending.current = e.id;
      setSetup({ id: e.id, secret: e.totp.secret, qr: await QRCode.toDataURL(e.totp.uri, { margin: 1, width: 200 }) });
    })().catch((x: unknown) => setErr(authErrorText(x)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const close = (added: boolean) => {
    // An enrolment that was never verified is removed again.
    const id = pending.current;
    if (!added && id) void getSupabase().then((sb) => sb.auth.mfa.unenroll({ factorId: id })).catch(() => {});
    onClose(added);
  };
  const verify = async (e: FormEvent) => {
    e.preventDefault();
    if (!setup) return;
    setBusy(true);
    setErr('');
    try {
      const sb = await getSupabase();
      must(await sb.auth.mfa.challengeAndVerify({ factorId: setup.id, code: code.replace(/\s/g, '') }));
      pending.current = null;
      close(true);
    } catch (x) {
      setErr(authErrorText(x));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title={t('Turn on two-step verification')} onClose={() => close(false)} size="sm">
      <ol className="muted" style={{ margin: 0, paddingLeft: 18, lineHeight: 1.6 }}>
        <li>{t('Open an authenticator app (1Password, Google Authenticator, Authy…).')}</li>
        <li>{t('Scan this code, or enter the key by hand.')}</li>
        <li>{t('Type the 6-digit code it shows.')}</li>
      </ol>
      {setup ? (
        <div className="row" style={{ gap: 16, alignItems: 'center' }}>
          <img src={setup.qr} width={160} height={160} alt={t('QR code for your authenticator app')} style={{ borderRadius: 8, background: '#fff' }} />
          <code style={{ wordBreak: 'break-all', fontSize: 13 }}>{setup.secret.replace(/(.{4})/g, '$1 ')}</code>
        </div>
      ) : !err && <div className="spinner" />}
      <form className="row" onSubmit={verify}>
        <input className="input grow" inputMode="numeric" autoComplete="one-time-code" placeholder="123456" value={code} onChange={(e) => setCode(e.target.value)} autoFocus />
        <button className="btn primary" disabled={busy || !setup || code.replace(/\s/g, '').length !== 6}>{t('Turn on')}</button>
      </form>
      {err && <div className="form-error">{err}</div>}
    </Modal>
  );
}

/**
 * Settings → Privacy → Delete account for a linked account: a Supabase sign-in from the last 10 minutes (aal2 with
 * MFA) replaces the password; the re-auth panel or MFA step appear in place when the server asks for them.
 */
export function SupabaseDeleteAccount({ info, onClose }: { info: AccountInfo; onClose: () => void }) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const fresh = useFreshSupabase(accountAuth(info));
  const [f, setF] = useState({ code: '', confirm: '' });
  const [err, setErr] = useState('');
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setErr('');
    void fresh.run(async (_sb, token) => {
      await deleteAccountSupabase({ confirm: 'DELETE', code: f.code || undefined, accessToken: token });
      // The account is gone; dropping its Supabase session here must not hold up the UI (review C3).
      void signOutSupabaseLocal();
      setUser(null);
      nav('/', { replace: true });
    }, setErr);
  };
  return (
    <Modal title={fresh.need ? fresh.title : t('Delete your account?')} onClose={onClose} size="sm">
      {fresh.need ? (
        <>
          {fresh.panel}
          <button type="button" className="btn ghost" onClick={fresh.cancel}>{t('Cancel')}</button>
        </>
      ) : (
        <>
          <p className="muted" style={{ margin: 0 }}>{interp(t('Everything you own is deleted. Type {word} to confirm.'), { word: <b>DELETE</b> })}</p>
          <form className="col" onSubmit={submit}>
            {/* Built-in TOTP on a linked account still needs its code (plan 12 §B9). */}
            {info.user.mfa && <label className="field">{t('Two-step code')}<input className="input" inputMode="numeric" value={f.code} onChange={(e) => setF({ ...f, code: e.target.value })} required /></label>}
            <label className="field">{t('Confirmation')}<input className="input" value={f.confirm} onChange={(e) => setF({ ...f, confirm: e.target.value })} placeholder="DELETE" /></label>
            {err && <div className="form-error">{err}</div>}
            <button className="btn danger" disabled={f.confirm !== 'DELETE'}>{t('Delete account permanently')}</button>
          </form>
        </>
      )}
    </Modal>
  );
}
