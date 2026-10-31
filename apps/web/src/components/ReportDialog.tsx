/** "Report" flow (TS-005): reason picker, details, and a full DMCA / DSA notice form for IP claims. */
import { Flag } from 'lucide-react';
import { useState } from 'react';
import { ApiError, reportApi, type ReportInput, type ReportReason } from '../api';
import { useAuth } from '../App';
import { Modal } from './ui';
import { t, tk } from '../i18n';

const REASONS: Array<{ id: ReportReason; label: string }> = [
  { id: 'csam', label: tk('Child sexual abuse material') },
  { id: 'ncii', label: tk('Intimate image shared without consent') },
  { id: 'violence', label: tk('Violence or threats') },
  { id: 'self_harm', label: tk('Self-harm or suicide') },
  { id: 'hate', label: tk('Hate speech') },
  { id: 'harassment', label: tk('Harassment or bullying') },
  { id: 'phishing', label: tk('Phishing or scam') },
  { id: 'spam', label: tk('Spam') },
  { id: 'impersonation', label: tk('Impersonation') },
  { id: 'privacy', label: tk('Shares my private information') },
  { id: 'copyright', label: tk('Copyright infringement') },
  { id: 'trademark', label: tk('Trademark infringement') },
  { id: 'other', label: tk('Something else') },
];

export function ReportButton({ targetType, targetId, className }: { targetType: ReportInput['targetType']; targetId: string; className?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button className={className ?? 'btn ghost sm'} onClick={() => setOpen(true)} title={t('Report this content')}><Flag size={14} /> {t('Report')}</button>
      {open && <ReportDialog targetType={targetType} targetId={targetId} onClose={() => setOpen(false)} />}
    </>
  );
}

export function ReportDialog({ targetType, targetId, onClose }: { targetType: ReportInput['targetType']; targetId: string; onClose: () => void }) {
  const { user } = useAuth();
  const [reason, setReason] = useState<ReportReason | null>(null);
  const [details, setDetails] = useState('');
  const [email, setEmail] = useState('');
  const [legal, setLegal] = useState({ name: '', address: '', work: '', goodFaith: false, accurate: false, signature: '' });
  const [err, setErr] = useState('');
  const [done, setDone] = useState(false);
  const ip = reason === 'copyright' || reason === 'trademark';

  if (done) {
    return (
      <Modal title={t('Thanks for your report')} onClose={onClose} size="sm">
        <p className="muted" style={{ margin: 0 }}>{t('Our team reviews reports in order of severity.')} {email || user ? t('We’ll email you the outcome.') : ''}</p>
        {(reason === 'csam' || reason === 'ncii') && <p className="muted" style={{ margin: 0 }}>{t('We hid the public link while we review it.')}</p>}
        <button className="btn primary" onClick={onClose}>{t('Done')}</button>
      </Modal>
    );
  }
  return (
    <Modal title={t('Report content')} onClose={onClose} size="md">
      <div className="col" role="radiogroup" aria-label={t('Reason')} style={{ gap: 4 }}>
        {REASONS.map((r) => (
          <label key={r.id} className="check" style={{ padding: '4px 0' }}>
            <input type="radio" name="reason" checked={reason === r.id} onChange={() => setReason(r.id)} /> {t(r.label)}
          </label>
        ))}
      </div>
      {reason && (
        <>
          <label className="field">{t('What’s wrong? (optional)')}<textarea className="input" rows={3} maxLength={5000} value={details} onChange={(e) => setDetails(e.target.value)} /></label>
          {!user && <label className="field">{t('Your email (to hear back)')}<input className="input" type="email" value={email} onChange={(e) => setEmail(e.target.value)} /></label>}
          {ip && (
            <div className="col box" style={{ padding: 14 }}>
              <strong>{t('Notice of claimed infringement')}</strong>
              <label className="field">{t('Your full legal name')}<input className="input" value={legal.name} onChange={(e) => setLegal({ ...legal, name: e.target.value })} /></label>
              <label className="field">{t('Postal address')}<input className="input" value={legal.address} onChange={(e) => setLegal({ ...legal, address: e.target.value })} /></label>
              <label className="field">{t('The original work (URL or description)')}<textarea className="input" rows={2} value={legal.work} onChange={(e) => setLegal({ ...legal, work: e.target.value })} /></label>
              <label className="check"><input type="checkbox" checked={legal.goodFaith} onChange={(e) => setLegal({ ...legal, goodFaith: e.target.checked })} /> {t('I believe in good faith that this use isn’t authorised by the owner, its agent or the law.')}</label>
              <label className="check"><input type="checkbox" checked={legal.accurate} onChange={(e) => setLegal({ ...legal, accurate: e.target.checked })} /> {t('The information is accurate, and under penalty of perjury I am the owner or authorised to act for the owner.')}</label>
              <label className="field">{t('Signature (type your full name)')}<input className="input" value={legal.signature} onChange={(e) => setLegal({ ...legal, signature: e.target.value })} /></label>
            </div>
          )}
          {err && <div className="form-error">{err}</div>}
          <button className="btn primary" onClick={async () => {
            setErr('');
            try {
              const r = await reportApi.file({ targetType, targetId, reason, details: details || undefined, email: email || undefined, legal: ip ? { ...legal, location: location.href } : undefined });
              if (!r.ok) return setErr(r.message ?? t('Check the form.'));
              setDone(true);
            } catch (e) {
              setErr(e instanceof ApiError ? e.message : t('Couldn’t send the report.'));
            }
          }}>{t('Send report')}</button>
        </>
      )}
    </Modal>
  );
}
