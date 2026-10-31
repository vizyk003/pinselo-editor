/**
 * "Design with your AI": connect Claude, ChatGPT or Codex to the MCP connector in as few clicks as each client allows,
 * see which ones are connected (and when they were last used), and disconnect them. Shared by the Dashboard card,
 * the editor's "Edit with AI" dialog and kept consistent with Settings → Developers (same connections API).
 *
 * Only officially documented flows are used (checked 2026-10-23):
 * - Claude (claude.ai web and desktop): the documented custom-connector install link
 *   `https://claude.ai/customize/connectors?modal=add-custom-connector&connectorName=…&connectorUrl=…` opens the
 *   "Add custom connector" dialog prefilled; the user confirms there, then approves our consent page.
 *   https://claude.com/docs/connectors/building/directory-vs-custom ("Custom connector install link")
 * - Claude Code: `claude mcp add --transport http <name> <url>`, then `/mcp` in a session to sign in.
 *   https://code.claude.com/docs/en/mcp
 * - ChatGPT: no prefilled link is documented; the furthest link is the Plugins page, where the user picks
 *   "+" → "Add custom MCP server" and pastes the URL (we copy it first).
 *   https://developers.openai.com/apps-sdk/deploy/connect-chatgpt
 * - Codex (CLI, IDE extension and app share the config): `codex mcp add <name> --url <url>`, then
 *   `codex mcp login <name>` for OAuth; or `[mcp_servers.<name>] url = "…"` in ~/.codex/config.toml.
 *   https://learn.chatgpt.com/docs/extend/mcp?surface=cli
 */
import { BRAND } from '@oc/config';
import { AlertTriangle, Check, ChevronDown, ExternalLink, Sparkles, Terminal, Unplug } from 'lucide-react';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { ApiError, oauthApi, type OAuthConnection } from '../api';
import { t, tk } from '../i18n';
import { AiLogo } from './AiLogo';
import { CopyButton, interp, timeAgo, useToast } from './ui';
import './connectAi.css';

export type AiClientId = 'claude' | 'chatgpt' | 'codex';

/** The connector's public URL (what an AI app is told to connect to). */
export const mcpUrl = () => `${location.origin}/mcp`;
/** The short name the CLI commands register the server under. */
export const mcpSlug = () => BRAND.name.toLowerCase().replace(/[^a-z0-9]+/g, '') || 'pinselo';

/** claude.ai's documented install link for a custom connector (name and URL prefilled; the user confirms). */
export const claudeInstallLink = (url = mcpUrl()) =>
  `https://claude.ai/customize/connectors?${new URLSearchParams({ modal: 'add-custom-connector', connectorName: BRAND.name, connectorUrl: url })}`;
export const CHATGPT_PLUGINS = 'https://chatgpt.com/plugins';
export const claudeCodeCommand = (url = mcpUrl()) => `claude mcp add --transport http ${mcpSlug()} ${url}`;
export const codexCommand = (url = mcpUrl()) => `codex mcp add ${mcpSlug()} --url ${url} && codex mcp login ${mcpSlug()}`;
export const codexConfig = (url = mcpUrl()) => `[mcp_servers.${mcpSlug()}]\nurl = "${url}"`;

/**
 * Which assistant a connected app is. A client ID metadata document's domain is verified (Claude and Claude Code
 * publish theirs on claude.ai, ChatGPT on chatgpt.com/openai.com); a dynamically registered client only by the name
 * it gave itself (Codex registers as "Codex…").
 */
export function clientOf(c: Pick<OAuthConnection, 'name' | 'domain'>): AiClientId | null {
  const name = c.name.toLowerCase();
  const domain = (c.domain ?? '').toLowerCase();
  const on = (d: string) => domain === d || domain.endsWith(`.${d}`);
  if (name.includes('codex')) return 'codex';
  if (on('claude.ai') || on('claude.com') || on('anthropic.com')) return 'claude';
  if (on('chatgpt.com') || on('openai.com')) return 'chatgpt';
  if (!domain && name.includes('claude')) return 'claude';
  if (!domain && (name.includes('chatgpt') || name.includes('openai'))) return 'chatgpt';
  return null;
}

/** Whether this server can't be reached by hosted assistants (they connect from the internet, not your machine). */
export function isLocalOrigin(host = location.hostname): boolean {
  return host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local') || /^(127\.|10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|\[?::1\]?$)/.test(host);
}

/** The user's connected apps (live OAuth grants), reloadable. `null` while loading. */
export function useAiConnections() {
  const [apps, setApps] = useState<OAuthConnection[] | null>(null);
  const reload = useCallback(() => oauthApi.connections().then((r) => setApps(r.connections)).catch(() => setApps([])), []);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { apps, reload };
}

interface ClientDef { id: AiClientId; name: string; blurb: string; mark: string }
const CLIENTS: ClientDef[] = [
  { id: 'claude', name: 'Claude', blurb: tk('Claude on the web and desktop, plus Claude Code'), mark: 'C' },
  { id: 'chatgpt', name: 'ChatGPT', blurb: tk('ChatGPT on the web, as a custom MCP plugin'), mark: 'G' },
  { id: 'codex', name: 'Codex', blurb: tk('The Codex CLI, IDE extension and app'), mark: '>_' },
];

/** Read-only command or config with a copy button. */
function Snippet({ text, label }: { text: string; label: string }) {
  return (
    <div className="row connect-ai-snippet">
      <pre className="grow code-block" aria-label={label}>{text}</pre>
      <CopyButton text={text} />
    </div>
  );
}

function Steps({ children }: { children: ReactNode }) {
  return <ol className="connect-ai-steps">{children}</ol>;
}

/** One assistant: status, the one-click (or shortest) action, the remaining steps and disconnect. */
function ClientTile({ def, connections, onDisconnect }: { def: ClientDef; connections: OAuthConnection[]; onDisconnect: (c: OAuthConnection) => void }) {
  const toast = useToast();
  const url = mcpUrl();
  const [open, setOpen] = useState(false);
  const connected = connections.length > 0;
  const lastUsed = connections.map((c) => c.lastUsedAt).filter((x): x is string => !!x).sort().pop() ?? null;
  const stepsId = `connect-ai-steps-${def.id}`;
  const copyAndOpen = async (href: string) => {
    try {
      await navigator.clipboard.writeText(url);
      toast(t('Connector URL copied'));
    } catch {
      toast(t('Copy the connector URL below'));
    }
    window.open(href, '_blank', 'noopener');
    setOpen(true);
  };
  // Once connected, connecting again is no longer the main thing to do here.
  const cta = connected ? 'btn' : 'btn primary';
  const primary = def.id === 'claude'
    ? <a className={cta} href={claudeInstallLink(url)} target="_blank" rel="noopener noreferrer" onClick={() => setOpen(true)}><ExternalLink size={14} aria-hidden /> {t('Add to Claude')}</a>
    : def.id === 'chatgpt'
      ? <button type="button" className={cta} onClick={() => void copyAndOpen(CHATGPT_PLUGINS)}><ExternalLink size={14} aria-hidden /> {t('Copy URL and open ChatGPT')}</button>
      : <button type="button" className={cta} onClick={() => { void navigator.clipboard.writeText(codexCommand(url)).then(() => toast(t('Command copied — paste it into your terminal')), () => toast(t('Copy the command below'))); setOpen(true); }}><Terminal size={14} aria-hidden /> {t('Copy the Codex command')}</button>;

  return (
    <article className={`connect-ai-tile${connected ? ' on' : ''}`} aria-labelledby={`connect-ai-${def.id}`}>
      <div className="connect-ai-head">
        <span className={`connect-ai-mark ${def.id}`} aria-hidden>{def.id === 'codex' ? def.mark : <AiLogo mark={def.id} size={22} />}</span>
        <div className="grow" style={{ minWidth: 0 }}>
          <h3 id={`connect-ai-${def.id}`}>{def.name}</h3>
          <p className="faint">{t(def.blurb)}</p>
        </div>
      </div>
      <p className="connect-ai-status" role="status">
        {connected ? (
          <span className="pill ok"><Check size={11} aria-hidden /> {t('Connected')}</span>
        ) : <span className="pill">{t('Not connected')}</span>}
        <span className="faint">
          {connected ? (lastUsed ? t('Last used {when}', { when: timeAgo(lastUsed) }) : t('Not used yet')) : ''}
        </span>
      </p>
      <div className="connect-ai-actions">
        {primary}
        <button type="button" className="btn ghost sm" aria-expanded={open} aria-controls={stepsId} onClick={() => setOpen((o) => !o)}>
          {t('How it works')} <ChevronDown size={14} aria-hidden className={open ? 'flip' : ''} />
        </button>
      </div>
      {open && (
        <div id={stepsId} className="connect-ai-detail">
          {def.id === 'claude' && (
            <>
              <Steps>
                <li>{t('In Claude, check the prefilled name and URL and select Add.')}</li>
                <li>{t('Select Connect, sign in to {brand} and select Allow.', { brand: BRAND.name })}</li>
                <li>{t('Ask Claude to design something. You’ll see its edits live.')}</li>
              </Steps>
              <h4>{t('Claude Code')}</h4>
              <p className="faint">{interp(t('Run this command, then type {cmd} in Claude Code to sign in.'), { cmd: <code>/mcp</code> })}</p>
              <Snippet text={claudeCodeCommand(url)} label={t('Claude Code command')} />
            </>
          )}
          {def.id === 'chatgpt' && (
            <>
              <Steps>
                <li>{t('On the Plugins page that opened, select + and then Add custom MCP server.')}</li>
                <li>{t('Paste the connector URL, choose OAuth and continue.')}</li>
                <li>{t('Sign in to {brand}, select Allow, then create the plugin.', { brand: BRAND.name })}</li>
              </Steps>
              <p className="faint">{t('ChatGPT offers custom MCP servers on some plans only, and doesn’t support a one-click link yet.')}</p>
            </>
          )}
          {def.id === 'codex' && (
            <>
              <Steps>
                <li>{t('Paste the command into a terminal and run it.')}</li>
                <li>{t('Your browser opens: sign in to {brand} and select Allow.', { brand: BRAND.name })}</li>
                <li>{t('Start Codex and ask it to design something.')}</li>
              </Steps>
              <Snippet text={codexCommand(url)} label={t('Codex command')} />
              <p className="faint">{interp(t('Or add this to {file}, then run {cmd}:'), { file: <code>~/.codex/config.toml</code>, cmd: <code>codex mcp login {mcpSlug()}</code> })}</p>
              <Snippet text={codexConfig(url)} label={t('Codex configuration')} />
            </>
          )}
          <div className="field">
            <label htmlFor={`connect-ai-url-${def.id}`} className="connect-ai-label">{t('Connector URL')}</label>
            <div className="row">
              <input id={`connect-ai-url-${def.id}`} className="input grow" readOnly value={url} onFocus={(e) => e.currentTarget.select()} style={{ minWidth: 0 }} />
              <CopyButton text={url} />
            </div>
          </div>
        </div>
      )}
      {connections.length > 0 && (
        <ul className="plain connect-ai-conns" aria-label={t('{name} connections', { name: def.name })}>
          {connections.map((c) => (
            <li key={c.clientId} className="row">
              <span className="grow" style={{ minWidth: 0, overflowWrap: 'anywhere' }}>
                {c.name}
                {c.domain ? <span className="faint"> · {c.domain}</span> : null}
              </span>
              <button type="button" className="btn sm ghost" aria-label={t('Disconnect {name}', { name: c.name })} onClick={() => onDisconnect(c)}>
                <Unplug size={14} aria-hidden /> {t('Disconnect')}
              </button>
            </li>
          ))}
        </ul>
      )}
    </article>
  );
}

/** The assistants side by side (all three unless `clients` narrows them), with live connection status. */
export function ConnectAi({ connections, reload, clients = CLIENTS.map((d) => d.id) }: { connections: OAuthConnection[] | null; reload: () => Promise<unknown>; clients?: AiClientId[] }) {
  const toast = useToast();
  const disconnect = async (c: OAuthConnection) => {
    try {
      await oauthApi.disconnect(c.clientId);
      toast(t('{name} disconnected', { name: c.name }));
      await reload();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('That didn’t work. Try again.'));
    }
  };
  const mine = (id: AiClientId) => (connections ?? []).filter((c) => clientOf(c) === id);
  // Apps whose tile isn't shown here still get listed, so a connection never silently disappears.
  const others = (connections ?? []).filter((c) => { const id = clientOf(c); return !id || !clients.includes(id); });
  return (
    <>
      {isLocalOrigin() && (
        <p className="connect-ai-note" role="note">
          <AlertTriangle size={14} aria-hidden />
          <span>{interp(t('This server runs at {url}, which only this computer can reach. Claude and ChatGPT connect from the internet, so they need a public https address; Claude Code and Codex on this computer work as is.'), { url: <code>{location.origin}</code> })}</span>
        </p>
      )}
      <div className="connect-ai-grid" aria-busy={connections === null}>
        {CLIENTS.filter((d) => clients.includes(d.id)).map((d) => <ClientTile key={d.id} def={d} connections={mine(d.id)} onDisconnect={(c) => void disconnect(c)} />)}
      </div>
      {others.length > 0 && (
        <p className="faint connect-ai-others">
          {t('Also connected: {apps}.', { apps: others.map((c) => c.name).join(', ') })}{' '}
          <a href="/settings/developers">{t('Manage in Settings')}</a>
        </p>
      )}
    </>
  );
}

/** The Dashboard section: a short pitch and the two chat assistants (Codex lives in Settings → Developers). */
export function DesignWithAiCard() {
  const { apps, reload } = useAiConnections();
  return (
    <section className="box connect-ai-card" aria-labelledby="design-with-ai">
      <div className="connect-ai-intro">
        <span className="connect-ai-spark" aria-hidden><Sparkles size={18} /></span>
        <div>
          <h2 id="design-with-ai">{t('Design with your AI')}</h2>
          <p className="muted">{t('Connect the assistant you already use. It works in your account, can only do what you allow, and you can undo its changes.')}</p>
        </div>
      </div>
      <ConnectAi connections={apps} reload={reload} clients={['claude', 'chatgpt']} />
    </section>
  );
}
