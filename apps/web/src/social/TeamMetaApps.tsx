/**
 * Settings → Connected accounts → Team Meta apps: a team can publish through its own Meta app (the app of the
 * business that owns its Pages) instead of the server's. Owners and admins set the app, then connect Pages and
 * Instagram accounts with a pasted system-user token or with Facebook login through that app. The accounts are
 * shared with the team. The app secret and tokens never come back to the browser.
 */
import { ExternalLink, KeyRound } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { ApiError, type Workspace } from '../api';
import { Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { socialApi, type TeamMetaApp } from './api';
import { ProviderMark } from './AccountBadge';

const message = (e: unknown) => (e instanceof ApiError && e.message ? e.message : t('That didn’t work. Try again.'));

export function TeamMetaApps({ teams, onConnected }: { teams: Workspace[]; onConnected: () => void }) {
  if (!teams.length) return null;
  return (
    <section className="box">
      <h2>{t('Team Meta apps')}</h2>
      <p className="muted">
        {t('A team can publish to Facebook and Instagram through its own Meta app, the one in the Business portfolio that owns its Pages.')}
      </p>
      <div className="accounts-list">
        {teams.map((w) => <TeamRow key={w.id} team={w} onConnected={onConnected} />)}
      </div>
    </section>
  );
}

function TeamRow({ team, onConnected }: { team: Workspace; onConnected: () => void }) {
  const toast = useToast();
  const [state, setState] = useState<TeamMetaApp | null>(null);
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState(false);
  const load = useCallback(() => socialApi.metaApp(team.id).then(setState).catch(() => setState(null)), [team.id]);
  useEffect(() => { void load(); }, [load]);
  if (!state) return null;
  const app = state.app;

  const login = async (provider: 'facebook' | 'instagram') => {
    try {
      location.assign((await socialApi.connectForTeam(provider, team.id)).url);
    } catch (e) {
      toast(message(e));
    }
  };

  return (
    <div className="account-row" style={{ flexWrap: 'wrap' }}>
      <KeyRound size={22} aria-hidden />
      <div className="who">
        <b>{team.name}</b>
        <span className="faint" style={{ fontSize: 12 }}>
          {app
            ? t('Meta app {name} ({id}) · Graph {version} · {n} accounts', {
              name: app.appName ?? '', id: app.appId, version: app.graphVersion ?? t('default'), n: String(state.accounts),
            })
            : state.serverApp ? t('Uses the server’s Meta app') : t('No Meta app yet')}
        </span>
      </div>
      {state.canManage && (
        <>
          {app && <button className="btn sm" onClick={() => setToken(true)}>{t('Connect with a system-user token')}</button>}
          {app && <button className="btn ghost sm" onClick={() => void login('facebook')}>{t('Log in with Facebook')}</button>}
          {app && <button className="btn ghost sm" onClick={() => void login('instagram')}>{t('Log in for Instagram')}</button>}
          <button className="btn ghost sm" onClick={() => setEditing(true)}>{app ? t('Change app') : t('Set up a Meta app')}</button>
          {app && (
            <button className="btn ghost sm" onClick={async () => {
              if (!confirm(t('Remove {team}’s Meta app? Its {n} accounts will need reconnecting.', { team: team.name, n: String(state.accounts) }))) return;
              try {
                await socialApi.removeMetaApp(team.id);
                void load();
                onConnected();
              } catch (e) {
                toast(message(e));
              }
            }}>{t('Remove')}</button>
          )}
        </>
      )}
      {editing && <AppForm team={team} current={state} onClose={() => setEditing(false)} onSaved={() => { setEditing(false); toast(t('Meta app saved')); void load(); onConnected(); }} />}
      {token && <TokenForm team={team} onClose={() => setToken(false)} onDone={(n) => { setToken(false); toast(t('Connected {n} accounts', { n: String(n) })); void load(); onConnected(); }} />}
    </div>
  );
}

function AppForm({ team, current, onClose, onSaved }: { team: Workspace; current: TeamMetaApp; onClose: () => void; onSaved: () => void }) {
  const [appId, setAppId] = useState(current.app?.appId ?? '');
  const [secret, setSecret] = useState('');
  const [version, setVersion] = useState(current.app?.graphVersion ?? 'v27.0');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const sameApp = current.app?.appId === appId.trim();
  return (
    <Modal title={t('Meta app for {team}', { team: team.name })} onClose={onClose} size="sm">
      <form className="planner-form" onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setErr('');
        try {
          await socialApi.saveMetaApp(team.id, { appId: appId.trim(), ...(secret ? { appSecret: secret.trim() } : {}), graphVersion: version.trim() || null });
          onSaved();
        } catch (ex) {
          setErr(message(ex));
        } finally {
          setBusy(false);
        }
      }}>
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {t('Find the App ID and App secret in the Meta app dashboard under App settings → Basic. Pinselo checks them with Meta before saving.')}{' '}
          <a href="https://developers.facebook.com/apps/" target="_blank" rel="noreferrer">{t('Open Meta for Developers')} <ExternalLink size={12} /></a>
        </p>
        <label className="field"><span>{t('App ID')}</span><input className="input" value={appId} onChange={(e) => setAppId(e.target.value)} inputMode="numeric" autoFocus /></label>
        <label className="field">
          <span>{t('App secret')}</span>
          <input className="input" type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off"
            placeholder={current.app && sameApp ? t('Unchanged') : ''} />
        </label>
        <label className="field"><span>{t('Graph API version')}</span><input className="input" value={version} onChange={(e) => setVersion(e.target.value)} placeholder="v27.0" /></label>
        {current.app && !sameApp && current.accounts > 0 && (
          <p className="error" style={{ margin: 0, fontSize: 13 }}>{t('Accounts connected through the old app will need reconnecting.')}</p>
        )}
        {err && <p className="error" role="alert" style={{ margin: 0 }}>{err}</p>}
        <button className="btn primary" disabled={busy || !appId.trim() || (!secret && !(current.app && sameApp))}>{busy ? t('Checking with Meta…') : t('Save')}</button>
      </form>
    </Modal>
  );
}

function TokenForm({ team, onClose, onDone }: { team: Workspace; onClose: () => void; onDone: (n: number) => void }) {
  const [provider, setProvider] = useState<'facebook' | 'instagram'>('facebook');
  const [token, setToken] = useState('');
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <Modal title={t('Connect with a system-user token')} onClose={onClose} size="sm">
      <form className="planner-form" onSubmit={async (e) => {
        e.preventDefault();
        setBusy(true);
        setErr('');
        try {
          onDone((await socialApi.connectSystemUser(team.id, provider, token.trim())).accounts.length);
        } catch (ex) {
          setErr(message(ex));
        } finally {
          setBusy(false);
        }
      }}>
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {t('In Meta Business Settings → System users, generate a token for the team’s app, and assign the Pages to the system user. Pinselo keeps each Page’s own token, not this one.')}{' '}
          <a href="https://business.facebook.com/settings/system-users" target="_blank" rel="noreferrer">{t('Open Business Settings')} <ExternalLink size={12} /></a>
        </p>
        <div className="row" role="radiogroup" aria-label={t('Network')} style={{ gap: 8 }}>
          {(['facebook', 'instagram'] as const).map((p) => (
            <button key={p} type="button" role="radio" aria-checked={provider === p} className={`btn sm${provider === p ? ' primary' : ''}`} onClick={() => setProvider(p)}>
              <ProviderMark provider={p} size={16} /> {p === 'facebook' ? t('Facebook Pages') : 'Instagram'}
            </button>
          ))}
        </div>
        <label className="field">
          <span>{t('System-user token')}</span>
          <input className="input" type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" autoFocus />
        </label>
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>
          {provider === 'facebook'
            ? t('Needs pages_show_list, pages_read_engagement and pages_manage_posts.')
            : t('Needs pages_show_list, instagram_basic and instagram_content_publish.')}
        </p>
        {err && <p className="error" role="alert" style={{ margin: 0 }}>{err}</p>}
        <button className="btn primary" disabled={busy || token.trim().length < 20}>{busy ? t('Connecting…') : t('Connect')}</button>
      </form>
    </Modal>
  );
}
