/** Settings → Connected accounts (A16): connect/disconnect social networks, share an account with a team. */
import { AlertTriangle, ExternalLink, Unplug } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { ApiError, teamApi, type Workspace } from '../api';
import { Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { socialApi, type Provider, type SocialAccount } from './api';
import { AccountBadge, ProviderMark } from './AccountBadge';
import './planner.css';

export function ConnectedAccountsTab() {
  const toast = useToast();
  const nav = useNavigate();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [accounts, setAccounts] = useState<SocialAccount[] | null>(null);
  const [teams, setTeams] = useState<Workspace[]>([]);
  const [bluesky, setBluesky] = useState(false);
  const reload = useCallback(() => socialApi.accounts().then((r) => setAccounts(r.accounts)).catch(() => setAccounts([])), []);

  useEffect(() => {
    void socialApi.providers().then((r) => setProviders(r.providers)).catch(() => {});
    void teamApi.list().then((r) => setTeams(r.workspaces.filter((w) => !w.personal))).catch(() => {});
    void reload();
    // Back from an OAuth consent screen.
    const q = new URLSearchParams(location.search);
    if (q.get('connected')) toast(t('Connected {network}', { network: q.get('connected')! }));
    if (q.get('error')) toast(q.get('error')!);
    if (q.get('connected') || q.get('error')) nav('/settings/accounts', { replace: true });
  }, [reload, toast, nav]);

  const connect = async (p: Provider) => {
    if (p.auth === 'app-password') return setBluesky(true);
    try {
      const r = await socialApi.connect(p.id);
      if (r.url) location.assign(r.url);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
    }
  };
  const label = (id: string) => providers.find((p) => p.id === id)?.label ?? id;

  return (
    <>
      <section className="box">
        <h2>{t('Connected accounts')}</h2>
        <p className="muted">{t('Schedule designs to these accounts from the editor or the Content Planner. Access tokens are stored encrypted and never leave the server.')}</p>
        {!accounts ? <div className="spinner" /> : (
          <div className="accounts-list">
            {accounts.map((a) => (
              <div key={a.id} className="account-row">
                <AccountBadge account={a} size={34} />
                <div className="who">
                  <b>{a.provider === 'local' ? t('Download & remind') : a.displayName}</b>
                  <span className="faint" style={{ fontSize: 12 }}>
                    {a.provider === 'local' ? t('Built in: at the scheduled time you get an email and a notification with the files and caption.') : `${label(a.provider)}${a.handle ? ` · @${a.handle}` : ''}`}
                  </span>
                  {a.status === 'reauth' && <span className="error" style={{ fontSize: 12 }}><AlertTriangle size={12} /> {t('Access expired — reconnect to keep posting.')}</span>}
                </div>
                {a.provider !== 'local' && a.mine && teams.length > 0 && (
                  <select className="input" style={{ width: 'auto' }} value={a.workspaceId ?? ''} aria-label={t('Share with a team')}
                    onChange={async (e) => {
                      await socialApi.updateAccount(a.id, { workspaceId: e.target.value || null });
                      toast(e.target.value ? t('Your team can now schedule to this account') : t('Only you can use this account'));
                      void reload();
                    }}>
                    <option value="">{t('Only me')}</option>
                    {teams.map((w) => <option key={w.id} value={w.id}>{t('Shared with {team}', { team: w.name })}</option>)}
                  </select>
                )}
                {a.provider !== 'local' && a.status === 'reauth' && a.mine && (
                  <button className="btn sm" onClick={() => { const p = providers.find((x) => x.id === a.provider); if (p) void connect(p); }}>{t('Reconnect')}</button>
                )}
                {a.provider !== 'local' && a.mine && (
                  <button className="btn ghost sm" onClick={async () => {
                    if (!confirm(t('Disconnect {name}? Scheduled posts to it will fail.', { name: a.displayName }))) return;
                    await socialApi.disconnect(a.id);
                    void reload();
                  }}><Unplug size={14} /> {t('Disconnect')}</button>
                )}
                {a.provider !== 'local' && !a.mine && <span className="faint" style={{ fontSize: 12 }}>{t('Shared by a teammate')}</span>}
              </div>
            ))}
          </div>
        )}
      </section>
      <section className="box">
        <h2>{t('Connect a network')}</h2>
        <div className="network-grid">
          {providers.filter((p) => p.id !== 'local').map((p) => (
            <button key={p.id} className="network-card" disabled={!p.configured} onClick={() => connect(p)}
              title={p.configured ? undefined : t('Not configured on this server. An admin sets {vars}.', { vars: p.env.join(', ') })}>
              <ProviderMark provider={p.id} size={30} />
              <span className="t">
                <b>{p.label}</b>
                <span>{!p.configured ? t('Not configured') : p.auth === 'app-password' ? t('Connect with an app password') : t('Connect')}</span>
              </span>
            </button>
          ))}
        </div>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>
          {t('Networks show as “Not configured” until your administrator connects them.')}{' '}
          <Link to="/planner">{t('Open the Content Planner')}</Link>
        </p>
      </section>
      {bluesky && <BlueskyConnect onClose={() => setBluesky(false)} onDone={() => { setBluesky(false); toast(t('Connected {network}', { network: 'Bluesky' })); void reload(); }} />}
    </>
  );
}

function BlueskyConnect({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [handle, setHandle] = useState('');
  const [pw, setPw] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('Connect Bluesky')} onClose={onClose} size="sm">
      <form className="planner-form" onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setErr('');
        try {
          await socialApi.connect('bluesky', { identifier: handle, password: pw });
          onDone();
        } catch (ex) {
          setErr(ex instanceof ApiError ? ex.message : t('That didn’t work. Try again.'));
        } finally {
          setBusy(false);
        }
      }}>
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {t('Use an app password, not your main password.')}{' '}
          <a href="https://bsky.app/settings/app-passwords" target="_blank" rel="noreferrer">{t('Create one in Bluesky')} <ExternalLink size={12} /></a>
        </p>
        <label className="field"><span>{t('Handle')}</span><input className="input" value={handle} onChange={(e) => setHandle(e.target.value)} placeholder="you.bsky.social" autoFocus autoComplete="username" /></label>
        <label className="field"><span>{t('App password')}</span><input className="input" type="password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder="xxxx-xxxx-xxxx-xxxx" autoComplete="current-password" /></label>
        {err && <p className="error" role="alert" style={{ margin: 0 }}>{err}</p>}
        <button className="btn primary" disabled={busy || !handle || !pw}>{busy ? t('Connecting…') : t('Connect')}</button>
      </form>
    </Modal>
  );
}
