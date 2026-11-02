/**
 * Account settings: profile, security (password, 2FA, passkeys, sessions), email, privacy (export/delete).
 * With Supabase Auth (plan 12), the email, security and delete parts come from auth/SupabaseSettings.tsx.
 */
import { startRegistration } from '@simplewebauthn/browser';
import { Download, KeyRound, LogOut, ShieldCheck, Trash2 } from 'lucide-react';
import QRCode from 'qrcode';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { NavLink, Route, Routes, useNavigate } from 'react-router';
import { accountApi, ApiError, authApi, type AccountInfo, type SessionRow } from '../api';
import { useAuth } from '../App';
import { authModeOf, useAuthConfigState } from '../auth/authConfig';
import { accountAuth, resendConfirmation, SupabaseDeleteAccount, SupabaseEmailField, SupabaseSecurityTab } from '../auth/SupabaseSettings';
import { authErrorText } from '../auth/SupabaseAuth';
import { ConsentSettingsLink } from '../components/ConsentBanner';
import { SignInUnavailable } from './Auth';
import { interp, Modal, PageHeader, timeAgo, useScrollFade, useToast } from '../components/ui';
import { chooseLocale, getLocale, LOCALES, t, tn } from '../i18n';
import { DevelopersTab } from './Developers';
import { ConnectedAccountsTab } from '../social/ConnectedAccounts';
import { AccessibilityTab } from './AccessibilitySettings';

const errText = (e: unknown) => (e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));

export function Settings() {
  const [info, setInfo] = useState<AccountInfo | null>(null);
  const tabsRow = useScrollFade();
  const reload = useCallback(() => accountApi.get().then(setInfo), []);
  useEffect(() => {
    void reload();
  }, [reload]);
  return (
    <>
    <PageHeader title={t('Settings')} />
    <div className="spage">
      <nav className="tabs" aria-label={t('Settings sections')} ref={tabsRow}>
        <NavLink to="/settings/account" end>{t('Your account')}</NavLink>
        <NavLink to="/settings/security">{t('Login & security')}</NavLink>
        <NavLink to="/settings/notifications">{t('Email')}</NavLink>
        <NavLink to="/settings/accessibility">{t('Accessibility')}</NavLink>
        <NavLink to="/settings/privacy">{t('Privacy & data')}</NavLink>
        <NavLink to="/settings/accounts">{t('Connected accounts')}</NavLink>
        <NavLink to="/settings/developers">{t('Developers')}</NavLink>
      </nav>
      {!info ? (
        <div className="spinner" />
      ) : (
        <Routes>
          <Route path="account" element={<AccountTab info={info} reload={reload} />} />
          <Route path="security" element={<SecurityTab info={info} reload={reload} />} />
          <Route path="notifications" element={<EmailTab info={info} reload={reload} />} />
          <Route path="privacy" element={<PrivacyTab info={info} />} />
          <Route path="accessibility" element={<AccessibilityTab />} />
          <Route path="developers" element={<DevelopersTab />} />
          <Route path="accounts" element={<ConnectedAccountsTab />} />
          <Route path="*" element={<AccountTab info={info} reload={reload} />} />
        </Routes>
      )}
    </div>
    </>
  );
}

/**
 * Built-in or Supabase settings. GET /api/account says which for this account; a server without that field falls back
 * to the sign-in config, which may still be loading or have failed (then "try again", never the built-in parts: C4).
 */
function useAuthMode(info: AccountInfo): 'builtin' | 'supabase' | 'loading' | 'failed' {
  const s = useAuthConfigState();
  return accountAuth(info)?.mode ?? (s.kind === 'ready' ? authModeOf(s.config) : s.kind);
}

export function Panel({ title, sub, children, danger }: { title: string; sub?: ReactNode; children: ReactNode; danger?: boolean }) {
  return (
    <section className={`box${danger ? ' danger' : ''}`}>
      <h2>{title}</h2>
      {sub && <p className="muted">{sub}</p>}
      {children}
    </section>
  );
}

function AccountTab({ info, reload }: { info: AccountInfo; reload: () => void }) {
  const { user, setUser } = useAuth();
  const toast = useToast();
  const [name, setName] = useState(info.user.name);
  const supabase = useAuthMode(info) === 'supabase';
  const resend = () => supabase
    ? resendConfirmation(info.user.email).then(() => toast(t('Confirmation email sent')), (e: unknown) => toast(authErrorText(e)))
    : authApi.resendVerify().then(() => toast(t('Confirmation email sent')));
  return (
    <>
      {!info.user.verified && (
        <div className="banner">
          <span className="grow">{t('Confirm your email address to receive share and comment notifications.')}</span>
          <button className="btn sm" onClick={resend}>{t('Resend email')}</button>
        </div>
      )}
      <Panel title={t('Profile')}>
        <label className="field">
          {t('Name')}
          <div className="row">
            <input className="input grow" value={name} onChange={(e) => setName(e.target.value)} maxLength={80} />
            <button className="btn" disabled={!name.trim() || name === info.user.name} onClick={async () => {
              await accountApi.update({ name: name.trim() });
              setUser({ ...user!, name: name.trim() });
              toast(t('Name updated'));
              reload();
            }}>{t('Save')}</button>
          </div>
        </label>
        {supabase ? <SupabaseEmailField info={info} /> : (
          <label className="field">
            {t('Email')}
            <input className="input" value={info.user.email} disabled />
          </label>
        )}
        <label className="field">
          {t('Language')}
          <select className="input" value={getLocale()} onChange={async (e) => {
            // Read the choice now: once this handler yields, React resets the controlled select to the current locale.
            const code = e.target.value;
            try {
              await accountApi.update({ locale: code });
            } catch {
              toast(t('Couldn’t save your language.'));
              return;
            }
            setUser({ ...user!, locale: code });
            await chooseLocale(code);
          }}>
            {LOCALES.map((l) => <option key={l.code} value={l.code} lang={l.code}>{l.name}</option>)}
          </select>
        </label>
      </Panel>
      {info.oauth.length > 0 && (
        <Panel title={t('Connected sign-in')}>
          <div className="list">
            {info.oauth.map((o) => (
              <div className="item" key={o.provider}><div className="main"><div style={{ textTransform: 'capitalize' }}>{o.provider}</div><div className="sub">{o.email}</div></div></div>
            ))}
          </div>
        </Panel>
      )}
    </>
  );
}

function SecurityTab(p: { info: AccountInfo; reload: () => void }) {
  const mode = useAuthMode(p.info);
  if (mode === 'loading') return <div className="spinner" />;
  if (mode === 'failed') return <SignInUnavailable compact />;
  return mode === 'supabase' ? <SupabaseSecurityTab {...p} /> : <BuiltinSecurityTab {...p} />;
}

function BuiltinSecurityTab({ info, reload }: { info: AccountInfo; reload: () => void }) {
  const toast = useToast();
  const [sessionsVersion, setSessionsVersion] = useState(0);
  const [pw, setPw] = useState({ current: '', next: '' });
  const [pwErr, setPwErr] = useState('');
  const [totp, setTotp] = useState(false);
  const [disable, setDisable] = useState(false);

  const addPasskey = async () => {
    try {
      const options = await authApi.passkeyRegisterOptions();
      const response = await startRegistration({ optionsJSON: options as never });
      await authApi.passkeyRegisterVerify(response);
      toast(t('Passkey added'));
      reload();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('Passkey setup was cancelled.'));
    }
  };

  return (
    <>
      <Panel title={info.hasPassword ? t('Change password') : t('Set a password')} sub={info.hasPassword ? t('Changing it signs out your other sessions.') : t('You sign in with links or a provider. Add a password as another option.')}>
        <form className="col" onSubmit={async (e) => {
          e.preventDefault();
          setPwErr('');
          try {
            await accountApi.password(info.hasPassword ? pw.current : undefined, pw.next);
            setPw({ current: '', next: '' });
            toast(t('Password saved'));
            reload();
            setSessionsVersion((v) => v + 1);
          } catch (x) {
            setPwErr(errText(x));
          }
        }}>
          {info.hasPassword && <label className="field">{t('Current password')}<input className="input" type="password" autoComplete="current-password" value={pw.current} onChange={(e) => setPw({ ...pw, current: e.target.value })} required /></label>}
          <label className="field">{t('New password')}<input className="input" type="password" autoComplete="new-password" minLength={10} value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} required /></label>
          {pwErr && <div className="form-error">{pwErr}</div>}
          <div><button className="btn">{t('Save password')}</button></div>
        </form>
      </Panel>

      <Panel title={t('Two-step verification')} sub={t('Ask for a code from an authenticator app when you log in with a password, link or provider.')}>
        {info.user.mfa ? (
          <div className="row">
            <ShieldCheck size={18} color="#16a34a" />
            <span className="grow">{t('On')} · {tn(info.recoveryCodesLeft, '{n} recovery code left', '{n} recovery codes left')}</span>
            <button className="btn danger" onClick={() => setDisable(true)}>{t('Turn off')}</button>
          </div>
        ) : (
          <div><button className="btn primary" onClick={() => setTotp(true)}>{t('Turn on')}</button></div>
        )}
      </Panel>

      <Panel title={t('Passkeys')} sub={t('Sign in with your fingerprint, face or device PIN. Passkeys can’t be phished.')}>
        {info.passkeys.length > 0 && <div className="list">
          {info.passkeys.map((p) => (
            <div className="item" key={p.id}>
              <KeyRound size={18} />
              <div className="main"><div>{p.name ?? t('Passkey')}</div><div className="sub">{t('Added {when}', { when: timeAgo(p.created_at) })}{p.last_used_at ? ` · ${t('last used {when}', { when: timeAgo(p.last_used_at) })}` : ''}</div></div>
              <button className="btn ghost sm danger" onClick={async () => { await accountApi.removePasskey(p.id); reload(); }}>{t('Remove')}</button>
            </div>
          ))}
        </div>}
        <div><button className="btn" onClick={addPasskey} disabled={!window.PublicKeyCredential}>{t('Add a passkey')}</button></div>
      </Panel>

      <SessionsPanel version={sessionsVersion} revokeOthers={async (done) => { await accountApi.revokeOthers(); toast(t('Logged out of other sessions')); done(); }} />
      {totp && <TotpSetup onClose={() => { setTotp(false); reload(); }} />}
      {disable && <TotpDisable onClose={() => { setDisable(false); reload(); }} />}
    </>
  );
}

/**
 * Where you're logged in (Pinselo sessions). `revokeOthers` ends the other sessions, then calls `done` to reload;
 * `version` reloads after other changes (a new password). `alwaysOffer`: Supabase mode, where other Supabase sessions may exist.
 */
export function SessionsPanel({ version = 0, alwaysOffer, revokeOthers }: { version?: number; alwaysOffer?: boolean; revokeOthers: (done: () => void) => void | Promise<void> }) {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const loadSessions = useCallback(() => accountApi.sessions().then((r) => setSessions(r.sessions)), []);
  useEffect(() => {
    void loadSessions();
  }, [loadSessions, version]);
  return (
    <Panel title={t('Where you’re logged in')}>
      <div className="list">
        {sessions.map((s) => (
          <div className="item" key={s.id}>
            <div className="main">
              <div>{s.device} {s.current && <span className="pill ok">{t('This device')}</span>}</div>
              <div className="sub">{s.ip ?? t('unknown IP')} · {t('active {when}', { when: timeAgo(s.last_seen_at) })}</div>
            </div>
            {!s.current && <button className="btn ghost sm" onClick={async () => { await accountApi.revokeSession(s.id); void loadSessions(); }}>{t('Log out')}</button>}
          </div>
        ))}
      </div>
      {(sessions.length > 1 || alwaysOffer) && <div><button className="btn" onClick={() => void revokeOthers(() => void loadSessions())}><LogOut size={16} /> {t('Log out everywhere else')}</button></div>}
    </Panel>
  );
}

function TotpSetup({ onClose }: { onClose: () => void }) {
  const [setup, setSetup] = useState<{ secret: string; qr: string } | null>(null);
  const [code, setCode] = useState('');
  const [err, setErr] = useState('');
  const [codes, setCodes] = useState<string[] | null>(null);
  useEffect(() => {
    // Only the live effect shows its answer (StrictMode mounts twice in dev). The server returns the same pending
    // secret to a repeated setup call, so whichever request it stored, the key shown is the one it checks (AUTH-01).
    let live = true;
    accountApi.totpSetup().then(async (s) => {
      const qr = await QRCode.toDataURL(s.otpauthUrl, { margin: 1, width: 200 });
      if (live) setSetup({ secret: s.secret, qr });
    }).catch(() => {});
    return () => { live = false; };
  }, []);
  if (codes) {
    return (
      <Modal title={t('Save your recovery codes')} onClose={onClose} size="sm">
        <p className="muted" style={{ margin: 0 }}>{t('Each code works once if you lose your phone. Store them somewhere safe — you won’t see them again.')}</p>
        <div className="codes">{codes.map((c) => <span key={c}>{c}</span>)}</div>
        <div className="row">
          <button className="btn" onClick={() => void navigator.clipboard.writeText(codes.join('\n'))}>{t('Copy')}</button>
          <button className="btn" onClick={() => {
            const a = document.createElement('a');
            a.href = URL.createObjectURL(new Blob([codes.join('\n')], { type: 'text/plain' }));
            a.download = 'recovery-codes.txt';
            a.click();
          }}><Download size={16} /> {t('Download')}</button>
          <span className="grow" />
          <button className="btn primary" onClick={onClose}>{t('I saved them')}</button>
        </div>
      </Modal>
    );
  }
  return (
    <Modal title={t('Turn on two-step verification')} onClose={onClose} size="sm">
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
      ) : <div className="spinner" />}
      <form className="row" onSubmit={async (e) => {
        e.preventDefault();
        setErr('');
        try {
          setCodes((await accountApi.totpEnable(code)).recoveryCodes);
        } catch (x) {
          setErr(errText(x));
        }
      }}>
        <input className="input grow" inputMode="numeric" autoComplete="one-time-code" placeholder="123456" value={code} onChange={(e) => setCode(e.target.value)} autoFocus />
        <button className="btn primary" disabled={code.replace(/\s/g, '').length !== 6}>{t('Turn on')}</button>
      </form>
      {err && <div className="form-error">{err}</div>}
    </Modal>
  );
}

export function TotpDisable({ onClose }: { onClose: () => void }) {
  const [code, setCode] = useState('');
  const [err, setErr] = useState('');
  return (
    <Modal title={t('Turn off two-step verification')} onClose={onClose} size="sm">
      <p className="muted" style={{ margin: 0 }}>{t('Enter a current code from your authenticator app to confirm.')}</p>
      <form className="row" onSubmit={async (e) => {
        e.preventDefault();
        try {
          await accountApi.totpDisable(code);
          onClose();
        } catch (x) {
          setErr(errText(x));
        }
      }}>
        <input className="input grow" inputMode="numeric" value={code} onChange={(e) => setCode(e.target.value)} autoFocus />
        <button className="btn danger">{t('Turn off')}</button>
      </form>
      {err && <div className="form-error">{err}</div>}
    </Modal>
  );
}

function EmailTab({ info, reload }: { info: AccountInfo; reload: () => void }) {
  const toast = useToast();
  const prefs = info.prefs ?? { mentions: true, comments: true, shares: true, product: false };
  const set = async (k: keyof typeof prefs, v: boolean) => {
    await accountApi.prefs({ ...prefs, [k]: v });
    toast(t('Saved'));
    reload();
  };
  const rows: Array<[keyof typeof prefs, string, string]> = [
    ['mentions', t('Mentions'), t('Someone @mentions you in a comment.')],
    ['comments', t('Comments and replies'), t('New comments on your designs and replies to your threads.')],
    ['shares', t('Shares and invites'), t('Someone shares a design or invites you to a team.')],
    ['product', t('Product news'), t('Occasional updates about new features. Off by default.')],
  ];
  return (
    <Panel title={t('Email notifications')} sub={t('Security emails (sign-in links, new logins, password changes) are always sent.')}>
      <div className="list">
        {rows.map(([k, label, help]) => (
          <label className="item" key={k} style={{ cursor: 'pointer' }}>
            <div className="main"><div>{label}</div><div className="sub">{help}</div></div>
            <input type="checkbox" checked={prefs[k]} onChange={(e) => void set(k, e.target.checked)} style={{ width: 18, height: 18, accentColor: 'var(--accent)' }} />
          </label>
        ))}
      </div>
    </Panel>
  );
}

function PrivacyTab({ info }: { info: AccountInfo }) {
  const toast = useToast();
  const [job, setJob] = useState<string | null>(null);
  const [del, setDel] = useState(false);
  // A Supabase-linked account proves itself with a recent Supabase sign-in instead of a password (plan 12 §B9).
  const supabaseLinked = useAuthMode(info) === 'supabase' && Boolean(accountAuth(info)?.supabaseLinked);
  return (
    <>
      <Panel title={t('Cookies and analytics')} sub={t('Choose whether we may record which features you use. Essential cookies are always on.')}>
        <div><ConsentSettingsLink /></div>
      </Panel>
      <Panel title={t('Download your data')} sub={t('A zip with your profile, designs (as JSON), comments and uploaded files. We’ll email you a link when it’s ready.')}>
        <div className="row">
          <button className="btn" disabled={Boolean(job)} onClick={async () => { setJob((await accountApi.exportData()).jobId); toast(t('Export started — we’ll email you')); }}>
            <Download size={16} /> {job ? t('Preparing…') : t('Request export')}
          </button>
          {job && <a className="linkish" href={`/api/account/exports/${job}`}>{t('Check status / download')}</a>}
        </div>
      </Panel>
      <Panel danger title={t('Delete account')} sub={t('Your designs, uploads and comments are deleted permanently. Team designs you created stay with the team. This can’t be undone.')}>
        <div><button className="btn danger" onClick={() => setDel(true)}><Trash2 size={16} /> {t('Delete my account')}</button></div>
      </Panel>
      {del && (supabaseLinked
        ? <SupabaseDeleteAccount info={info} onClose={() => setDel(false)} />
        : <DeleteAccount hasPassword={info.hasPassword} mfa={Boolean(info.user.mfa)} onClose={() => setDel(false)} />)}
    </>
  );
}

function DeleteAccount({ hasPassword, mfa, onClose }: { hasPassword: boolean; mfa: boolean; onClose: () => void }) {
  const { setUser } = useAuth();
  const nav = useNavigate();
  const [f, setF] = useState({ password: '', code: '', confirm: '' });
  const [err, setErr] = useState('');
  return (
    <Modal title={t('Delete your account?')} onClose={onClose} size="sm">
      <p className="muted" style={{ margin: 0 }}>{interp(t('Everything you own is deleted. Type {word} to confirm.'), { word: <b>DELETE</b> })}</p>
      <form className="col" onSubmit={async (e) => {
        e.preventDefault();
        try {
          await accountApi.deleteAccount({ password: f.password || undefined, code: f.code || undefined, confirm: 'DELETE' });
          setUser(null);
          nav('/', { replace: true });
        } catch (x) {
          setErr(errText(x));
        }
      }}>
        {hasPassword && <label className="field">{t('Password')}<input className="input" type="password" value={f.password} onChange={(e) => setF({ ...f, password: e.target.value })} required /></label>}
        {mfa && <label className="field">{t('Two-step code')}<input className="input" inputMode="numeric" value={f.code} onChange={(e) => setF({ ...f, code: e.target.value })} required /></label>}
        <label className="field">{t('Confirmation')}<input className="input" value={f.confirm} onChange={(e) => setF({ ...f, confirm: e.target.value })} placeholder="DELETE" /></label>
        {err && <div className="form-error">{err}</div>}
        <button className="btn danger" disabled={f.confirm !== 'DELETE'}>{t('Delete account permanently')}</button>
      </form>
    </Modal>
  );
}
