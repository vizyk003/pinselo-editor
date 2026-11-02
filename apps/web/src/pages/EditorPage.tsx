/**
 * Loads a design: ticket → Hocuspocus room `design:{id}` (+ IndexedDB for offline-lite, E23) → Editor (Rust core via @oc/engine).
 */
import { track } from '../lib/analytics';
import { HocuspocusProvider } from '@hocuspocus/provider';
import { Editor } from '@oc/engine';
import { useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router';
import { IndexeddbPersistence } from 'y-indexeddb';
import * as Y from 'yjs';
import { accessApi, api, ApiError, type DesignSummary, type Role } from '../api';
import { useAuth } from '../App';
import { EditorShell } from '../editor/EditorShell';
import { DocEditor } from '../doc/DocEditor';
import { loadEngine, type Engine } from '../lib/engine';
import { brandStore } from '../editor/brandStore';
import { runPlaceIntent } from '../editor/placeIntent';
import { withoutPlace } from './home/startMedia';
import { intentDialog, withoutIntents, type IntentDialog } from '../editor/fileMenu';
import { t } from '../i18n';

interface Loaded { editor: Editor; engine: Engine; provider: HocuspocusProvider; design: DesignSummary & { role: Role }; intent: IntentDialog | null }

export default function EditorPage() {
  const { id } = useParams();
  const { user } = useAuth();
  const nav = useNavigate();
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState('');
  /** Bumped when the server changes our role mid-session (CO-006) so the editor re-opens with the new rights. */
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    if (!id || !user) return;
    let cancelled = false;
    let provider: HocuspocusProvider | null = null;
    let idb: IndexeddbPersistence | null = null;
    const ydoc = new Y.Doc();
    (async () => {
      try {
        const [{ design }, engine] = await Promise.all([api.design(id), loadEngine()]);
        if (cancelled) return;
        idb = new IndexeddbPersistence(`oc:design:${id}`, ydoc);
        const proto = location.protocol === 'https:' ? 'wss' : 'ws';
        provider = new HocuspocusProvider({
          url: `${proto}://${location.host}/collab`,
          name: `design:${id}`,
          document: ydoc,
          // Fetched on every (re)connect: after a revocation the server closes the socket and a fresh ticket
          // carries the current role — or fails when access is gone.
          token: async () => {
            try {
              const tkt = await api.ticket(id);
              if (tkt.role !== design.role && !cancelled) setEpoch((n) => n + 1);
              return tkt.token;
            } catch (e) {
              if (e instanceof ApiError && (e.status === 403 || e.status === 404) && !cancelled) {
                provider?.destroy();
                setLoaded(null);
                setError(t('You no longer have access to this design.'));
              }
              throw e;
            }
          },
        });
        await new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => (ydoc.share.size ? resolve() : reject(new Error(t('Could not reach the collaboration server.')))), 8000);
          provider!.on('synced', () => {
            clearTimeout(timer);
            resolve();
          });
          provider!.on('authenticationFailed', () => reject(new Error(t('You no longer have access to this design.'))));
        });
        if (cancelled) return;
        const editor = new Editor({ ydoc, actor: user.id });
        void brandStore.loadKits();
        void brandStore.loadDesign(id);
        provider.awareness?.setLocalStateField('user', { id: user.id, name: user.name, color: user.color });
        // The designs row owns the title (renames from Home); mirror it into the doc if they differ.
        if (design.role !== 'view' && design.role !== 'comment' && editor.doc.meta.title !== design.title) {
          editor.apply([{ op: 'meta', patch: { title: design.title } }], true);
        }
        // Started from an upload (g15): place the photo/video full-bleed once, then drop the one-shot param.
        const place = new URLSearchParams(location.search).get('place');
        if (place) {
          if (design.role !== 'view' && design.role !== 'comment') await runPlaceIntent(editor, place).catch(() => false);
          if (cancelled) return;
          nav({ pathname: location.pathname, search: withoutPlace(location.search) }, { replace: true });
        }
        // Deep links from dashboard cards (g88): ?download=1 opens Download, ?publishTemplate=1 Publish as template.
        // One-shot like `place`: dropped from the URL so a reload or a re-open after a role change doesn't repeat them.
        const search = place ? withoutPlace(location.search) : location.search;
        const intent = intentDialog(search, design.role !== 'view' && design.role !== 'comment');
        const rest = withoutIntents(search);
        if (rest !== null) nav({ pathname: location.pathname, search: rest }, { replace: true });
        track('editor_open', { role: design.role });
        setLoaded({ editor, engine, provider, design, intent });
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) setError(t('This design does not exist or is not shared with you.'));
        else setError((e as Error).message);
      }
    })();
    return () => {
      cancelled = true;
      provider?.destroy();
      void idb?.destroy();
      ydoc.destroy();
    };
  }, [id, user, epoch]);

  if (error)
    return (
      <div className="auth">
        <div className="auth-card">
          <h2 style={{ margin: 0 }}>{t('Can’t open design')}</h2>
          <p className="muted">{error}</p>
          <RequestAccess designId={id!} />
          <button className="btn" onClick={() => nav('/')}>{t('Back to home')}</button>
        </div>
      </div>
    );
  if (!loaded)
    return (
      <div className="auth">
        <div className="col" style={{ alignItems: 'center' }}>
          <img src="/favicon.svg" alt="" width={48} height={48} style={{ animation: 'pulse 1.2s ease-in-out infinite' }} />
          <span className="muted">{t('Opening design…')}</span>
        </div>
        <style>{'@keyframes pulse{50%{transform:scale(.88);opacity:.6}}'}</style>
      </div>
    );
  if (loaded.editor.doc.meta.surface === 'sheet')
    return (
      <div className="auth">
        <div className="auth-card">
          <p className="muted">{t('Sheets are no longer supported. This design’s data has not been deleted.')}</p>
          <button className="btn" onClick={() => nav('/')}>{t('Back to home')}</button>
        </div>
      </div>
    );
  if (loaded.editor.doc.meta.surface === 'doc') return <DocEditor key={`${id}:${epoch}`} ydoc={loaded.editor.store.ydoc} provider={loaded.provider} design={loaded.design} user={user!} />;
  return <EditorShell key={`${id}:${epoch}`} {...loaded} />;
}

/** Ask the owner for access (CO-043). */
function RequestAccess({ designId }: { designId: string }) {
  const [role, setRole] = useState<'view' | 'comment' | 'edit'>('edit');
  const [message, setMessage] = useState('');
  const [state, setState] = useState<'idle' | 'sent' | 'error'>('idle');
  if (state === 'sent') return <p role="status">{t('Request sent. You’ll get a notification when the owner responds.')}</p>;
  return (
    <form className="col" onSubmit={async (e) => {
      e.preventDefault();
      try { await accessApi.request(designId, role, message || undefined); setState('sent'); } catch { setState('error'); }
    }}>
      <div className="row">
        <select className="input" value={role} onChange={(e) => setRole(e.target.value as typeof role)} aria-label={t('Access level')}>
          <option value="edit">{t('Can edit')}</option><option value="comment">{t('Can comment')}</option><option value="view">{t('Can view')}</option>
        </select>
        <button className="btn primary">{t('Request access')}</button>
      </div>
      <input className="input" value={message} maxLength={500} onChange={(e) => setMessage(e.target.value)} placeholder={t('Add a note (optional)')} aria-label={t('Note')} />
      {state === 'error' && <div style={{ color: 'var(--danger)', fontSize: 13 }}>{t('Couldn’t send the request. The design may not exist.')}</div>}
    </form>
  );
}
