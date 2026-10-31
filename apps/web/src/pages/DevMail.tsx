/** Development mailbox: read outbox mail (sign-in links, invites) when no SMTP server is configured. */
import { useEffect, useState } from 'react';
import { useSearchParams } from 'react-router';
import { devApi } from '../api';
import { timeAgo } from '../components/ui';

type Msg = Awaited<ReturnType<typeof devApi.mail>>['messages'][number];

export function DevMail() {
  const [params, setParams] = useSearchParams();
  const to = params.get('to') ?? '';
  const [msgs, setMsgs] = useState<Msg[] | null>(null);
  const [sel, setSel] = useState<Msg | null>(null);
  const [err, setErr] = useState('');
  useEffect(() => {
    const load = () => devApi.mail(to || undefined).then((r) => {
      setMsgs(r.messages);
      setSel((s) => s ?? r.messages[0] ?? null);
    }).catch(() => setErr('The dev mailbox is only available in development without SMTP.'));
    void load();
    const t = setInterval(load, 3000);
    return () => clearInterval(t);
  }, [to]);
  return (
    <div className="spage wide">
      <h1>Dev mailbox</h1>
      <p className="muted" style={{ margin: 0 }}>Every email the app sends lands here while OC_SMTP_URL is unset. Links open in this browser.</p>
      <input className="input" placeholder="Filter by recipient" value={to} onChange={(e) => setParams(e.target.value ? { to: e.target.value } : {})} />
      {err && <div className="form-error">{err}</div>}
      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(220px, 320px) minmax(0, 1fr)', gap: 16 }}>
        <div className="list box" style={{ padding: '6px 14px', maxHeight: 560, overflow: 'auto' }}>
          {msgs?.length === 0 && <div className="empty">No mail yet.</div>}
          {msgs?.map((m) => (
            <button key={m.id} className="item" style={{ background: sel?.id === m.id ? 'var(--accent-soft)' : 'none', border: 0, textAlign: 'left', cursor: 'pointer', borderRadius: 8, padding: '10px 8px' }} onClick={() => setSel(m)}>
              <div className="main"><div style={{ fontWeight: 600 }}>{m.subject}</div><div className="sub">{m.to} · {timeAgo(m.createdAt)}</div></div>
            </button>
          ))}
        </div>
        {sel && <iframe className="mail-frame" title={sel.subject} srcDoc={sel.html.replace('<head>', '<head><base target="_top">')} sandbox="allow-top-navigation-by-user-activation allow-popups" />}
      </div>
    </div>
  );
}
