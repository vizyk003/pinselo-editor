/** Settings → Developers: AI assistants (MCP connector, connected apps), personal access tokens for the public API and webhook endpoints. */
import { Trash2, Unplug } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiError, developerApi, oauthApi, type DeveloperData } from '../api';
import { ConnectAi, mcpUrl, useAiConnections } from '../components/ConnectAi';
import { CopyButton, interp, Modal, timeAgo, useToast } from '../components/ui';
import { fmtDate, t } from '../i18n';
import { AppMark, SCOPE_TEXT } from './OAuthConsent';

const errText = (e: unknown) => (e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
const CONNECTOR_SCOPES = ['designs:read', 'designs:write', 'assets:write', 'exports'];

/** The same connect tiles as the Dashboard ("Design with your AI"), every connected app, and the PAT route. */
function AiAssistants({ onCodexToken }: { onCodexToken: () => void }) {
  const toast = useToast();
  const { apps, reload } = useAiConnections();
  return (
    <>
      <section className="box">
        <h2>{t('AI assistants')}</h2>
        <p className="muted">{t('Connect Claude, ChatGPT, Codex or another MCP client to design with AI. The assistant works in your account and can only do what you allow.')}</p>
        <ConnectAi connections={apps} reload={reload} />
        <p className="faint" style={{ margin: 0, fontSize: 13 }}>
          {interp(t('Other MCP clients: use the connector URL {url} with OAuth, or send a personal access token with the designs:read, designs:write, assets:write and exports scopes.'), { url: <code>{mcpUrl()}</code> })}{' '}
          <button type="button" className="linkish" onClick={onCodexToken}>{t('Create one below')}</button>
        </p>
      </section>

      <section className="box">
        <h2>{t('Connected apps')}</h2>
        <p className="muted">{t('Apps you’ve allowed to access your account. Disconnecting one signs it out right away.')}</p>
        {!apps ? <div className="spinner" /> : apps.length === 0 ? <p className="faint" style={{ margin: 0 }}>{t('No apps connected yet.')}</p> : (
          <div className="list">
            {apps.map((a) => (
              <div className="item" key={a.clientId}>
                <AppMark name={a.name} logo={a.logo} size={32} />
                <div className="main">
                  <div style={{ overflowWrap: 'anywhere' }}>{a.name}{a.domain ? <span className="faint"> · {a.domain}</span> : null}</div>
                  <div className="sub">{a.scopes.map((s) => (SCOPE_TEXT[s] ? t(SCOPE_TEXT[s]) : s)).join(' · ')}</div>
                  <div className="sub">{t('Connected {date}', { date: fmtDate(a.connectedAt) })} · {a.lastUsedAt ? t('used {when}', { when: timeAgo(a.lastUsedAt) }) : t('never used')}</div>
                </div>
                <button className="btn sm ghost" aria-label={t('Disconnect {name}', { name: a.name })} onClick={async () => {
                  try {
                    await oauthApi.disconnect(a.clientId);
                    toast(t('{name} disconnected', { name: a.name }));
                    void reload();
                  } catch (err) { toast(errText(err)); }
                }}><Unplug size={14} /> {t('Disconnect')}</button>
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
}

export function DevelopersTab() {
  const toast = useToast();
  const [data, setData] = useState<DeveloperData | null>(null);
  const reload = useCallback(() => developerApi.get().then(setData), []);
  useEffect(() => { void reload(); }, [reload]);
  const [tok, setTok] = useState({ name: '', scopes: ['designs:read'] as string[], expires: '90', workspace: '' });
  const [hook, setHook] = useState({ url: '', events: ['design.created'] as string[] });
  const [secret, setSecret] = useState<{ title: string; value: string } | null>(null);
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const tokenName = useRef<HTMLInputElement>(null);
  if (!data) return <div className="spinner" />;
  return (
    <>
      <AiAssistants onCodexToken={() => {
        setTok({ name: 'Codex', scopes: CONNECTOR_SCOPES, expires: '90', workspace: '' });
        tokenName.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
        tokenName.current?.focus({ preventScroll: true });
      }} />

      <section className="box">
        <h2>{t('API access tokens')}</h2>
        <p className="muted">{interp(t('Use tokens with the {api}. Send {header}. A token can do anything its scopes allow on your designs, so keep it secret.'), { api: <a href="/developers/api" target="_blank" rel="noreferrer">{t('Pinselo REST API')}</a>, header: <code>Authorization: Bearer &lt;token&gt;</code> })}</p>
        {data.tokens.length > 0 && (
          <div className="list">
            {data.tokens.map((tok0) => (
              <div className="item" key={tok0.id}>
                <div className="main">
                  <div>{tok0.name} <span className="muted">({tok0.prefix}…)</span></div>
                  <div className="sub">{tok0.workspace_name ? `${t('Only {workspace}', { workspace: tok0.workspace_name })} · ` : ''}{tok0.scopes.join(', ')} · {tok0.last_used_at ? t('used {when}', { when: timeAgo(tok0.last_used_at) }) : t('never used')}{tok0.expires_at ? ` · ${t('expires {date}', { date: fmtDate(tok0.expires_at) })}` : ''}</div>
                </div>
                <button className="btn sm ghost" aria-label={t('Revoke {name}', { name: tok0.name })} onClick={async () => { await developerApi.revokeToken(tok0.id); toast(t('Token revoked')); void reload(); }}><Trash2 size={14} /> {t('Revoke')}</button>
              </div>
            ))}
          </div>
        )}
        <form className="col" style={{ gap: 12 }} onSubmit={async (e) => {
          e.preventDefault();
          try {
            const r = await developerApi.createToken({ name: tok.name.trim(), scopes: tok.scopes, expiresInDays: tok.expires ? Number(tok.expires) : undefined, workspaceId: tok.workspace || undefined });
            setSecret({ title: t('Your new token'), value: r.token });
            setTok({ name: '', scopes: ['designs:read'], expires: '90', workspace: '' });
            void reload();
          } catch (err) { toast(errText(err)); }
        }}>
          <label className="field">{t('Name')}<input ref={tokenName} className="input" value={tok.name} onChange={(e) => setTok({ ...tok, name: e.target.value })} maxLength={80} placeholder={t('e.g. Zapier, CMS sync')} required /></label>
          <label className="field">{t('Designs')}
            <select className="input" value={tok.workspace} onChange={(e) => {
              const workspace = e.target.value;
              // A token limited to one workspace only reads and exports.
              setTok({ ...tok, workspace, scopes: workspace ? tok.scopes.filter((s) => data.workspaceScopes.includes(s)) : tok.scopes });
            }}>
              <option value="">{t('All designs I can open')}</option>
              {data.workspaces.map((w) => <option key={w.id} value={w.id}>{w.personal ? t('Only my personal designs') : t('Only {workspace}', { workspace: w.name })}</option>)}
            </select>
            {tok.workspace && <span className="faint" style={{ fontSize: 13 }}>{t('Designs elsewhere are invisible to this token. It can read and export designs, nothing else.')}</span>}
          </label>
          <fieldset className="field"><legend>{t('Scopes')}</legend>
            <div className="row" style={{ flexWrap: 'wrap', gap: 12 }}>
              {data.scopes.filter((s) => !tok.workspace || data.workspaceScopes.includes(s)).map((s) => <label key={s} className="row" style={{ gap: 4 }}><input type="checkbox" checked={tok.scopes.includes(s)} onChange={() => setTok({ ...tok, scopes: toggle(tok.scopes, s) })} />{s}</label>)}
            </div>
          </fieldset>
          <label className="field">{t('Expires')}
            <select className="input" value={tok.expires} onChange={(e) => setTok({ ...tok, expires: e.target.value })}>
              <option value="30">{t('In 30 days')}</option><option value="90">{t('In 90 days')}</option><option value="365">{t('In a year')}</option><option value="">{t('Never')}</option>
            </select>
          </label>
          <div><button className="btn primary" disabled={!tok.name.trim() || !tok.scopes.length}>{t('Create token')}</button></div>
        </form>
      </section>

      <section className="box">
        <h2>{t('Webhooks')}</h2>
        <p className="muted">{interp(t('We POST a signed JSON event to your URL when things happen. Verify the {header} header with the signing secret: HMAC-SHA256 of {payload}.'), { header: <code>OC-Signature</code>, payload: <code>"&lt;t&gt;.&lt;body&gt;"</code> })}</p>
        {data.webhooks.length > 0 && (
          <div className="list">
            {data.webhooks.map((w) => (
              <div className="item" key={w.id} style={{ display: 'block' }}>
                <div className="row">
                  <div className="main grow">
                    <div style={{ wordBreak: 'break-all' }}>{w.url}</div>
                    <div className="sub">{w.events.join(', ')}{!w.active && <strong> · {t('paused after repeated failures')}</strong>}</div>
                  </div>
                  <button className="btn sm" onClick={async () => { await developerApi.pingWebhook(w.id); toast(t('Test event sent')); setTimeout(() => void reload(), 1500); }}>{t('Send test')}</button>
                  <button className="btn sm ghost" aria-label={t('Delete webhook')} onClick={async () => { await developerApi.deleteWebhook(w.id); void reload(); }}><Trash2 size={14} /></button>
                </div>
                {w.deliveries?.length ? (
                  <details><summary className="muted">{t('Recent deliveries')}</summary>
                    <ul className="plain">
                      {w.deliveries.map((d) => <li key={d.id}><code>{d.event}</code> · {d.status}{d.response_code ? ` (${d.response_code})` : ''}{d.error && d.status !== 'delivered' ? ` · ${d.error}` : ''} · {timeAgo(d.created_at)}</li>)}
                    </ul>
                  </details>
                ) : null}
              </div>
            ))}
          </div>
        )}
        <form className="col" style={{ gap: 12 }} onSubmit={async (e) => {
          e.preventDefault();
          try {
            const r = await developerApi.createWebhook({ url: hook.url.trim(), events: hook.events });
            setSecret({ title: t('Webhook signing secret'), value: r.secret });
            setHook({ url: '', events: ['design.created'] });
            void reload();
          } catch (err) { toast(errText(err)); }
        }}>
          <label className="field">{t('Endpoint URL')}<input className="input" type="url" value={hook.url} onChange={(e) => setHook({ ...hook, url: e.target.value })} placeholder="https://example.com/pinselo" required /></label>
          <fieldset className="field"><legend>{t('Events')}</legend>
            <div className="row" style={{ flexWrap: 'wrap', gap: 12 }}>
              {data.events.filter((x) => x !== 'ping').map((ev) => <label key={ev} className="row" style={{ gap: 4 }}><input type="checkbox" checked={hook.events.includes(ev)} onChange={() => setHook({ ...hook, events: toggle(hook.events, ev) })} />{ev}</label>)}
            </div>
          </fieldset>
          <div><button className="btn primary" disabled={!hook.url.trim() || !hook.events.length}>{t('Add webhook')}</button></div>
        </form>
      </section>

      {secret && (
        <Modal title={secret.title} onClose={() => setSecret(null)}>
          <p>{t('Copy it now — you won’t be able to see it again.')}</p>
          <div className="row">
            <input className="input grow" readOnly value={secret.value} onFocus={(e) => e.currentTarget.select()} aria-label={secret.title} />
            <CopyButton text={secret.value} />
          </div>
        </Modal>
      )}
    </>
  );
}
