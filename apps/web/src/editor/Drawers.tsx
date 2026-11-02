import { Check, MessageSquare, RotateCcw, Save, Trash2, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { api, type Comment } from '../api';
import { useAuth } from '../App';
import { Avatar, EmptyState, PopoverButton, timeAgo, useToast } from '../components/ui';
import { fmtDate, t } from '../i18n';
import { useEd, useEditorState } from './context';
import { COMMENTS_REV, commentsChangedBy, type CommentsAwareness } from './liveComments';
import { LayersPanel } from './Panels';

export function useComments(designId: string, awareness?: CommentsAwareness | null) {
  const [comments, setComments] = useState<Comment[]>([]);
  const fetchComments = useCallback(() => api.comments(designId).then((r) => setComments(r.comments)).catch(() => {}), [designId]);
  useEffect(() => {
    void fetchComments();
    const timer = setInterval(fetchComments, 15_000);
    return () => clearInterval(timer);
  }, [fetchComments]);
  useEffect(() => {
    if (!awareness) return undefined;
    const seen = new Map<number, unknown>();
    commentsChangedBy(awareness, seen);
    const onChange = () => { if (commentsChangedBy(awareness, seen)) void fetchComments(); };
    awareness.on('change', onChange);
    return () => awareness.off('change', onChange);
  }, [awareness, fetchComments]);
  /** After this client changed a comment: reload, and tell everyone else in the design to. */
  const reload = useCallback(() => {
    if (awareness) awareness.setLocalStateField(COMMENTS_REV, (Number(awareness.getLocalState()?.[COMMENTS_REV]) || 0) + 1);
    return fetchComments();
  }, [awareness, fetchComments]);
  return { comments, reload };
}

const REACTIONS = ['👍', '❤️', '🎉', '😄', '👀', '✅'];

/** Comments with tasks (CO-030): assign + due date, reactions, edit, "assigned to me" filter. */
export function CommentsDrawer({ comments, reload, onClose }: { comments: Comment[]; reload: () => void; onClose: () => void }) {
  const { editor, design, role } = useEd();
  const { user } = useAuth();
  const toast = useToast();
  const state = useEditorState();
  const [body, setBody] = useState('');
  const [assignee, setAssignee] = useState('');
  const [due, setDue] = useState('');
  const [filter, setFilter] = useState<'open' | 'mine' | 'all'>('open');
  const [people, setPeople] = useState<Array<{ id: string; name: string; color: string }>>([]);
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null);
  useEffect(() => { void api.collaborators(design.id).then((r) => setPeople(r.people)).catch(() => {}); }, [design.id]);
  const canComment = role !== 'view';
  const target = state.selection[0];
  const roots = comments.filter((c) => !c.parent_id && (filter === 'all' || !c.resolved) && (filter !== 'mine' || c.assignee_id === user?.id));
  const replies = (id: string) => comments.filter((c) => c.parent_id === id);
  const post = async (parentId?: string, text = body) => {
    if (!text.trim()) return;
    try {
      await api.addComment(design.id, {
        body: text.trim(), parentId, pageId: state.activePageId, ...(target && !parentId ? { elementId: target } : {}),
        ...(!parentId && assignee ? { assigneeId: assignee } : {}), ...(!parentId && assignee && due ? { dueOn: due } : {}),
      });
    } catch (e) {
      toast((e as Error).message);
      return;
    }
    if (!parentId) { setBody(''); setAssignee(''); setDue(''); }
    reload();
  };
  const today = new Date().toISOString().slice(0, 10);
  const Body = ({ c }: { c: Comment }) => editing?.id === c.id ? (
    <form className="col" onSubmit={async (e) => { e.preventDefault(); await api.updateComment(c.id, { body: editing.text }); setEditing(null); reload(); }}>
      <textarea className="input" rows={2} value={editing.text} onChange={(e) => setEditing({ id: c.id, text: e.target.value })} autoFocus />
      <div className="row"><button className="btn sm primary">{t('Save')}</button><button type="button" className="btn sm ghost" onClick={() => setEditing(null)}>{t('Cancel')}</button></div>
    </form>
  ) : <div style={{ whiteSpace: 'pre-wrap' }}>{c.body}{c.edited_at && <span className="faint" style={{ fontSize: 11 }}> {t('(edited)')}</span>}</div>;
  const Reactions = ({ c }: { c: Comment }) => (
    <div className="row" style={{ gap: 4, flexWrap: 'wrap' }}>
      {c.reactions.map((r) => (
        <button key={r.emoji} className={`chip ${r.mine ? 'on' : ''}`} disabled={!canComment} onClick={async () => { await api.react(c.id, r.emoji); reload(); }} aria-pressed={r.mine} aria-label={t('{emoji} reaction, {n}', { emoji: r.emoji, n: r.count })}>{r.emoji} {r.count}</button>
      ))}
      {canComment && (
        <PopoverButton className="chip" title={t('Add reaction')} label="☺+" content={(close) => (
          <div className="row" style={{ gap: 2 }}>{REACTIONS.map((e) => <button key={e} className="btn ghost sm" style={{ fontSize: 18 }} onClick={async () => { close(); await api.react(c.id, e); reload(); }}>{e}</button>)}</div>
        )} />
      )}
    </div>
  );
  return (
    <aside className="drawer" aria-label={t('Comments')}>
      <div className="drawer-head">
        <MessageSquare size={16} />
        <h3>{t('Comments')}</h3>
        <select className="input" style={{ width: 'auto', minWidth: 0, flexShrink: 1, height: 30, fontSize: 12 }} value={filter} onChange={(e) => setFilter(e.target.value as typeof filter)} aria-label={t('Show')}>
          <option value="open">{t('Open')}</option>
          <option value="mine">{t('Assigned to me')}</option>
          <option value="all">{t('All, including resolved')}</option>
        </select>
        <button className="btn ghost sm icon" onClick={onClose} aria-label={t('Close')}><X size={16} /></button>
      </div>
      <div className="drawer-body">
        {canComment && (
          <div className="col">
            <textarea className="input" rows={3} placeholder={target ? t('Comment on the selected element… use @name to mention') : t('Comment on this page…')} value={body} onChange={(e) => setBody(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void post(); }} />
            <div className="row" style={{ gap: 6 }}>
              <select className="input grow" style={{ height: 30 }} value={assignee} onChange={(e) => setAssignee(e.target.value)} aria-label={t('Assign to')}>
                <option value="">{t('No assignee')}</option>
                {people.map((p) => <option key={p.id} value={p.id}>{p.id === user?.id ? t('{name} (you)', { name: p.name }) : p.name}</option>)}
              </select>
              {assignee && <input className="input" type="date" style={{ height: 30, width: 140 }} value={due} min={today} onChange={(e) => setDue(e.target.value)} aria-label={t('Due date')} />}
            </div>
            <button className="btn primary sm" onClick={() => post()} disabled={!body.trim()}>{assignee ? t('Assign task') : t('Comment')}</button>
          </div>
        )}
        {!roots.length && <EmptyState icon={<MessageSquare size={20} />}>{filter === 'mine' ? t('Nothing assigned to you here.') : t('No comments yet.')}</EmptyState>}
        {roots.map((c) => {
          const overdue = c.due_on && !c.resolved && c.due_on < today;
          return (
            <div key={c.id} className={`comment ${c.resolved ? 'resolved' : ''}`}>
              <div className="who">
                <Avatar name={c.author} color={c.author_color} size={22} />
                {c.author}
                <span className="when">{timeAgo(c.created_at)}</span>
              </div>
              {c.assignee && (
                <div className="row" style={{ gap: 6, fontSize: 12 }}>
                  <span className={`chip ${c.resolved ? '' : 'task'}`}><Check size={12} /> {c.assignee_id === user?.id ? t('Assigned to you') : t('Assigned to {name}', { name: c.assignee })}</span>
                  {c.due_on && <span className={`chip ${overdue ? 'danger' : ''}`}>{t('Due {date}', { date: fmtDate(`${c.due_on}T12:00:00`) })}</span>}
                </div>
              )}
              <Body c={c} />
              {c.element_id && (
                <button className="btn ghost sm" style={{ justifySelf: 'start' }} onClick={() => editor.doc.elements[c.element_id as never] && editor.select([c.element_id as never])}>{t('Show element')}</button>
              )}
              <Reactions c={c} />
              {replies(c.id).map((r) => (
                <div key={r.id} style={{ borderLeft: '2px solid var(--border)', paddingLeft: 8 }}>
                  <div className="who" style={{ fontSize: 12 }}><Avatar name={r.author} color={r.author_color} size={18} />{r.author}<span className="when">{timeAgo(r.created_at)}</span></div>
                  <Body c={r} />
                  <Reactions c={r} />
                </div>
              ))}
              {canComment && <Reply onSend={(text) => post(c.id, text)} />}
              <div className="row">
                {canComment && (
                  <button className="btn ghost sm" onClick={async () => { await api.resolveComment(c.id, !c.resolved); reload(); }}>
                    <Check size={14} /> {c.resolved ? t('Reopen') : c.assignee ? t('Mark done') : t('Resolve')}
                  </button>
                )}
                {c.author_id === user?.id && !editing && <button className="btn ghost sm" onClick={() => setEditing({ id: c.id, text: c.body })}>{t('Edit')}</button>}
                {c.author_id === user?.id && (
                  <button className="btn ghost sm danger" onClick={async () => { await api.deleteComment(c.id); reload(); }} aria-label={t('Delete comment')}><Trash2 size={14} /></button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </aside>
  );
}

function Reply({ onSend }: { onSend: (text: string) => void }) {
  const [text, setText] = useState('');
  return (
    <form className="row" onSubmit={(e) => { e.preventDefault(); onSend(text); setText(''); }}>
      <input className="input" style={{ height: 30 }} placeholder={t('Reply…')} value={text} onChange={(e) => setText(e.target.value)} />
    </form>
  );
}

export function VersionsDrawer({ onClose }: { onClose: () => void }) {
  const { editor, design, readOnly } = useEd();
  const toast = useToast();
  const [versions, setVersions] = useState<Array<{ id: string; label: string | null; created_at: string; author: string | null }> | null>(null);
  const reload = useCallback(() => api.versions(design.id).then((r) => setVersions(r.versions)), [design.id]);
  useEffect(() => {
    void reload();
  }, [reload]);
  const restore = async (vid: string) => {
    const { doc } = await api.version(design.id, vid);
    // Restore is a normal (undoable) edit on the live document, so collaborators see it immediately.
    editor.store.checkpoint();
    editor.store.replaceContent(doc);
    editor.store.checkpoint();
    editor.clearSelection();
    toast(t('Version restored — press ⌘Z to undo'));
  };
  return (
    <aside className="drawer" aria-label={t('Version history')}>
      <div className="drawer-head">
        <RotateCcw size={16} />
        <h3>{t('Version history')}</h3>
        <button className="btn ghost sm icon" onClick={onClose} aria-label={t('Close')}><X size={16} /></button>
      </div>
      <div className="drawer-body">
        {!readOnly && (
          <button className="btn sm" onClick={async () => {
            const label = prompt(t('Name this version (optional)')) ?? undefined;
            await api.saveVersion(design.id, label || undefined);
            void reload();
            toast(t('Version saved'));
          }}><Save size={14} /> {t('Save current version')}</button>
        )}
        <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Versions are saved automatically every 10 minutes while you edit.')}</p>
        {!versions && <div className="empty">{t('Loading…')}</div>}
        {versions && !versions.length && <div className="empty">{t('No versions yet.')}</div>}
        {versions?.map((v) => (
          <div key={v.id} className="comment">
            <div className="who">{v.label ?? new Date(v.created_at).toLocaleString()}<span className="when">{timeAgo(v.created_at)}</span></div>
            <div className="faint" style={{ fontSize: 12 }}>{v.author ? t('by {author}', { author: v.author }) : t('automatic')}</div>
            {!readOnly && <button className="btn sm" style={{ justifySelf: 'start' }} onClick={() => restore(v.id)}><RotateCcw size={14} /> {t('Restore')}</button>}
          </div>
        ))}
      </div>
    </aside>
  );
}

export function LayersDrawer({ onClose }: { onClose: () => void }) {
  return (
    <aside className="drawer" aria-label={t('Layers')}>
      <div className="drawer-head">
        <h3>{t('Layers')}</h3>
        <button className="btn ghost sm icon" onClick={onClose} aria-label={t('Close')}><X size={16} /></button>
      </div>
      <div className="drawer-body"><LayersPanel /></div>
    </aside>
  );
}
