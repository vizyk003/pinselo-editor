/**
 * AI assistants in the editor (AI connector): they join as collaborators whose presence carries `ai: true`.
 * The header shows "Claude is editing" while one is working, and a bar offers to undo an assistant's latest
 * session (restores the checkpoint it saved before its first change, as one undoable edit).
 */
import { Sparkles, Undo2, X } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { api } from '../api';
import { AiLogo, aiMarkFor } from '../components/AiLogo';
import { timeAgo, useToast } from '../components/ui';
import { t } from '../i18n';
import { useEd, type Presence } from './context';

export const isAi = (u: Presence['user']) => Boolean(u.ai);

/** Avatar for an assistant: a sparkle in the assistant colour instead of initials. */
export function AiAvatar({ user }: { user: Presence['user'] }) {
  const title = user.onBehalfOf ? t('{name} (working for {person})', { name: user.name, person: user.onBehalfOf }) : user.name;
  const mark = aiMarkFor(user.name);
  if (mark) return <span className="avatar ai-avatar" title={title} style={{ background: '#fff', color: '#000', boxShadow: `0 0 0 2px ${user.color}` }}><AiLogo mark={mark} size={15} /></span>;
  return <span className="avatar ai-avatar" title={title} style={{ background: user.color }}><Sparkles size={14} /></span>;
}

/** "Claude is editing" while an assistant is connected to this design. */
export function AiPill({ presence }: { presence: Presence[] }) {
  const ai = presence.find((p) => isAi(p.user));
  if (!ai) return null;
  return (
    <span className="ai-pill" role="status" style={{ ['--ai' as string]: ai.user.color }}>
      <span className="ai-dot" aria-hidden /> {aiMarkFor(ai.user.name) ? <AiLogo mark={aiMarkFor(ai.user.name)!} size={13} style={{ verticalAlign: '-2px' }} /> : <Sparkles size={13} />} {t('{name} is editing', { name: ai.user.name })}
    </span>
  );
}

interface Checkpoint { id: string; agent: string; createdAt: string }
const DISMISSED = 'oc.aiUndo.dismissed';
const dismissed = (id: string) => {
  try {
    return (localStorage.getItem(DISMISSED) ?? '').split(',').includes(id);
  } catch {
    return false;
  }
};
const dismiss = (id: string) => {
  try {
    localStorage.setItem(DISMISSED, [...(localStorage.getItem(DISMISSED) ?? '').split(',').filter(Boolean).slice(-50), id].join(','));
  } catch {
    /* private mode */
  }
};

/** Offer to undo an assistant's latest session on this design (shown for a day, until kept or undone). */
export function AiUndoBar({ aiActive }: { aiActive: boolean }) {
  const { editor, design, readOnly } = useEd();
  const toast = useToast();
  const [cp, setCp] = useState<Checkpoint | null>(null);
  const load = useCallback(() => api.aiCheckpoint(design.id).then((r) => setCp(r.checkpoint && !dismissed(r.checkpoint.id) && Date.now() - Date.parse(r.checkpoint.createdAt) < 86_400_000 ? r.checkpoint : null)).catch(() => {}), [design.id]);
  // On open, and whenever an assistant joins or leaves (a new session saves a new checkpoint).
  useEffect(() => {
    void load();
  }, [load, aiActive]);
  if (!cp || readOnly) return null;
  const undo = async () => {
    const { doc } = await api.version(design.id, cp.id);
    editor.store.checkpoint();
    editor.store.replaceContent(doc);
    editor.store.checkpoint();
    editor.clearSelection();
    dismiss(cp.id);
    setCp(null);
    toast(t('{name}’s changes were undone — press ⌘Z to bring them back', { name: cp.agent }));
  };
  const keep = () => {
    dismiss(cp.id);
    setCp(null);
  };
  return (
    <div className="ai-undo-bar" role="region" aria-label={t('AI changes')}>
      <Sparkles size={15} aria-hidden />
      <span className="grow">{aiActive ? t('{name} is changing this design. You can undo everything it does.', { name: cp.agent }) : t('{name} changed this design {when}.', { name: cp.agent, when: timeAgo(cp.createdAt) })}</span>
      <button className="btn sm" onClick={() => void undo()}><Undo2 size={14} /> {t('Undo {name}’s changes', { name: cp.agent })}</button>
      {!aiActive && <button className="btn ghost sm icon" onClick={keep} title={t('Keep changes')} aria-label={t('Keep changes')}><X size={15} /></button>}
    </div>
  );
}
