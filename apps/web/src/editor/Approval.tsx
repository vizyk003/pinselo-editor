/**
 * Design approvals (Brand Hub, A8): send for approval with a message, reviewers approve or request changes,
 * a status badge in the editor header. Any edit after approval resets it (the server compares document state).
 */
import { BadgeCheck, CircleAlert, Clock } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { api, ApiError, brandHub, type Approval, type Person } from '../api';
import { useToast } from '../components/ui';
import { t } from '../i18n';
import { brandStore } from './brandStore';
import { useEd } from './context';

type Loaded = { approval: Approval | null; required: boolean; suggestedReviewers: Person[] };

function useApproval(designId: string) {
  const [data, setData] = useState<Loaded | null>(null);
  const load = useCallback(() => brandHub.approval(designId).then(setData).catch(() => {}), [designId]);
  useEffect(() => {
    void load();
  }, [load]);
  return { data, load };
}

/** Header badge: pending / approved / changes requested / changed since approval. */
export function ApprovalBadge() {
  const { design } = useEd();
  const { data, load } = useApproval(design.id);
  useEffect(() => {
    const on = () => void load();
    window.addEventListener('oc:approval', on);
    // The document changes as people edit: refresh "approved" vs "changed since" now and then.
    const timer = setInterval(on, 60_000);
    return () => {
      window.removeEventListener('oc:approval', on);
      clearInterval(timer);
    };
  }, [load]);
  const a = data?.approval;
  if (!a && !data?.required) return null;
  const [Icon, label, color] = !a ? [Clock, t('Needs approval'), '#a16207']
    : a.status === 'pending' ? [Clock, t('Awaiting approval'), '#a16207']
    : a.status === 'changes_requested' ? [CircleAlert, t('Changes requested'), '#b91c1c']
    : a.current ? [BadgeCheck, t('Approved'), '#15803d'] : [CircleAlert, t('Changed since approval'), '#a16207'];
  return (
    <span className="badge" role="status" title={a?.note ?? a?.message ?? label} style={{ background: '#fff', color, display: 'inline-flex', alignItems: 'center', gap: 4, padding: '2px 8px' }}>
      <Icon size={13} /> {label}
    </span>
  );
}

/** Approval section of the Share dialog. */
export function ApprovalSection() {
  const { design, role } = useEd();
  const toast = useToast();
  const { data, load } = useApproval(design.id);
  const [message, setMessage] = useState('');
  const [note, setNote] = useState('');
  const [picked, setPicked] = useState<string[] | null>(null);
  const [people, setPeople] = useState<Person[]>([]);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void api.collaborators(design.id).then((r) => setPeople(r.people)).catch(() => {});
  }, [design.id]);
  if (!data) return null;
  const a = data.approval;
  const canSend = role === 'owner' || role === 'edit';
  const candidates = [...new Map([...data.suggestedReviewers, ...people].map((p) => [p.id, p])).values()];
  const reviewers = picked ?? data.suggestedReviewers.map((p) => p.id);
  const done = async (fn: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    try {
      await fn();
      toast(ok);
      await load();
      void brandStore.loadDesign(design.id);
      window.dispatchEvent(new Event('oc:approval'));
    } catch (e) {
      toast(e instanceof ApiError ? e.message : (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="col" style={{ gap: 8 }}>
      <div className="hsep" />
      <h4 style={{ margin: 0 }}>{t('Approval')}</h4>
      {data.required && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Your team requires approval before this design is downloaded, shared publicly, published or scheduled.')}</p>}
      {a && (
        <div className="muted" style={{ fontSize: 13 }}>
          {a.status === 'pending' && t('Sent for approval by {name} to {reviewers}.', { name: a.requestedBy?.name ?? '—', reviewers: a.reviewers.map((r) => r.name).join(', ') })}
          {a.status === 'approved' && (a.current ? t('Approved by {name}.', { name: a.decidedBy?.name ?? '—' }) : t('Approved by {name}, but the design changed since. Send it again.', { name: a.decidedBy?.name ?? '—' }))}
          {a.status === 'changes_requested' && t('{name} requested changes.', { name: a.decidedBy?.name ?? '—' })}
          {a.message && a.status === 'pending' && <div>“{a.message}”</div>}
          {a.note && a.status !== 'pending' && <div>“{a.note}”</div>}
        </div>
      )}
      {a?.status === 'pending' && a.canReview && (
        <div className="col" style={{ gap: 6 }}>
          <textarea className="input" rows={2} placeholder={t('Add a note (optional)')} value={note} onChange={(e) => setNote(e.target.value)} maxLength={2000} />
          <div className="row">
            <button className="btn primary" disabled={busy} onClick={() => done(() => brandHub.decide(design.id, 'approve', note), t('Approved'))}>{t('Approve')}</button>
            <button className="btn" disabled={busy} onClick={() => done(() => brandHub.decide(design.id, 'changes', note), t('Changes requested'))}>{t('Request changes')}</button>
          </div>
        </div>
      )}
      {a?.status === 'pending' && canSend && !a.canReview && (
        <button className="btn sm" disabled={busy} onClick={() => done(() => brandHub.cancelApproval(design.id), t('Approval request withdrawn'))}>{t('Withdraw request')}</button>
      )}
      {canSend && a?.status !== 'pending' && (
        <div className="col" style={{ gap: 6 }}>
          {candidates.length > 0 && (
            <div className="row" style={{ flexWrap: 'wrap', gap: 6 }} role="group" aria-label={t('Reviewers')}>
              {candidates.map((p) => (
                <label key={p.id} className="check" style={{ fontSize: 12 }}>
                  <input type="checkbox" checked={reviewers.includes(p.id)} onChange={(e) => setPicked(e.target.checked ? [...reviewers, p.id] : reviewers.filter((x) => x !== p.id))} /> {p.name}
                </label>
              ))}
            </div>
          )}
          <textarea className="input" rows={2} placeholder={t('Message for reviewers (optional)')} value={message} onChange={(e) => setMessage(e.target.value)} maxLength={2000} />
          <button className="btn" disabled={busy || !reviewers.length} onClick={() => done(() => brandHub.requestApproval(design.id, { message, reviewerIds: reviewers }), t('Sent for approval'))}>
            <BadgeCheck size={15} /> {t('Send for approval')}
          </button>
        </div>
      )}
    </div>
  );
}
