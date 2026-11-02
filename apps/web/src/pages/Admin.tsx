/** Staff console (TS-003/005/006): report queue with actions, user enforcement, audit chain, jobs, hash list. */
import { useCallback, useEffect, useState } from 'react';
import { NavLink, Route, Routes } from 'react-router';
import { adminApi, analyticsAdminApi, contentApi, ApiError, statusApi, supportApi, type AdminReport, type StatusInfo, type SupportTicket } from '../api';
import { useAuth } from '../App';
import { Modal, timeAgo, useToast } from '../components/ui';

type Overview = Awaited<ReturnType<typeof adminApi.overview>>;

export function Admin() {
  const { user } = useAuth();
  const [ov, setOv] = useState<Overview | null>(null);
  useEffect(() => {
    adminApi.overview().then(setOv).catch(() => {});
  }, []);
  if (!user?.staff) return <div className="empty">This area is for staff only.</div>;
  return (
    <div className="spage wide">
      <h1>Admin</h1>
      {ov && (
        <div className="stat-row">
          <div className="stat"><div className="n">{ov.users}</div><div className="l">Users</div></div>
          <div className="stat"><div className="n">{ov.designs}</div><div className="l">Designs</div></div>
          <div className="stat"><div className="n">{ov.openReports}</div><div className="l">Open reports</div></div>
          <div className="stat"><div className="n">{ov.flaggedAssets}</div><div className="l">Flagged uploads</div></div>
          <div className="stat"><div className="n">{ov.failedJobs24h}</div><div className="l">Failed jobs (24 h)</div></div>
        </div>
      )}
      <nav className="tabs">
        <NavLink to="/admin" end>Reports</NavLink>
        <NavLink to="/admin/users">Users</NavLink>
        <NavLink to="/admin/audit">Audit log</NavLink>
        <NavLink to="/admin/jobs">Jobs</NavLink>
        <NavLink to="/admin/hashes">Hash list</NavLink>
        <NavLink to="/admin/templates">Templates</NavLink>
        <NavLink to="/admin/analytics">Analytics</NavLink>
        <NavLink to="/admin/support">Support</NavLink>
        <NavLink to="/admin/status">Status</NavLink>
      </nav>
      <Routes>
        <Route index element={<Reports />} />
        <Route path="users" element={<Users />} />
        <Route path="audit" element={<Audit />} />
        <Route path="jobs" element={<Jobs />} />
        <Route path="hashes" element={<Hashes />} />
        <Route path="templates" element={<TemplatesAdmin />} />
        <Route path="analytics" element={<Analytics />} />
        <Route path="support" element={<Support />} />
        <Route path="status" element={<StatusAdmin />} />
      </Routes>
    </div>
  );
}

const ACTIONS: Record<string, Array<{ id: string; label: string }>> = {
  design: [{ id: 'takedown', label: 'Take down design' }, { id: 'disable_user', label: 'Disable owner' }, { id: 'restore', label: 'Restore' }],
  public_link: [{ id: 'takedown', label: 'Take down design' }, { id: 'disable_user', label: 'Disable owner' }, { id: 'restore', label: 'Restore' }],
  asset: [{ id: 'remove_asset', label: 'Remove upload' }, { id: 'disable_user', label: 'Disable owner' }, { id: 'restore', label: 'Restore' }],
  comment: [{ id: 'remove_comment', label: 'Remove comment' }, { id: 'disable_user', label: 'Disable author' }],
  user: [{ id: 'disable_user', label: 'Disable user' }],
};

function Reports() {
  const [status, setStatus] = useState('open');
  const [rows, setRows] = useState<AdminReport[] | null>(null);
  const [open, setOpen] = useState<AdminReport | null>(null);
  const load = useCallback(() => adminApi.reports(status).then((r) => setRows(r.reports)), [status]);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <>
      <div className="row">
        {['open', 'actioned', 'dismissed'].map((s) => <button key={s} className={`btn sm${s === status ? ' active' : ''}`} onClick={() => setStatus(s)} style={{ textTransform: 'capitalize' }}>{s}</button>)}
      </div>
      {!rows ? <div className="spinner" /> : rows.length === 0 ? <div className="empty">No {status} reports.</div> : (
        <table className="table">
          <thead><tr><th>Priority</th><th>Reason</th><th>Target</th><th>Reporter</th><th>Filed</th><th /></tr></thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id}>
                <td><span className={`pill${r.priority >= 60 ? ' warn' : ''}`}>{r.priority}</span></td>
                <td>{r.reason.replace('_', ' ')}</td>
                <td><code>{r.target_type}:{r.target_id.slice(0, 18)}</code></td>
                <td>{r.reporter_name ?? r.reporter_email ?? 'anonymous'}</td>
                <td>{timeAgo(r.created_at)}</td>
                <td><button className="btn sm" onClick={() => setOpen(r)}>{status === 'open' ? 'Review' : 'View'}</button></td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {open && <ReviewDialog report={open} onClose={() => { setOpen(null); void load(); }} />}
    </>
  );
}

function ReviewDialog({ report, onClose }: { report: AdminReport; onClose: () => void }) {
  const toast = useToast();
  const [note, setNote] = useState('');
  const [blockHash, setBlockHash] = useState(report.reason === 'csam' || report.reason === 'ncii');
  const [err, setErr] = useState('');
  const decide = async (action: string) => {
    setErr('');
    try {
      await adminApi.decide(report.id, { action, note, blockHash });
      toast('Decision recorded; the user was notified');
      onClose();
    } catch (e) {
      setErr(e instanceof ApiError ? e.message : 'Write a note of at least 3 characters.');
    }
  };
  const preview = report.target_type === 'design' || report.target_type === 'public_link' ? `/view/${report.target_id}` : report.target_type === 'asset' ? `/api/assets/${report.target_id}` : null;
  return (
    <Modal title={`Report · ${report.reason.replace('_', ' ')}`} onClose={onClose} size="lg">
      <div className="list">
        <div className="item"><div className="main"><div className="sub">Target</div><div><code>{report.target_type}:{report.target_id}</code> {preview && <a className="linkish" href={preview} target="_blank" rel="noreferrer">Open</a>}</div></div></div>
        {report.details && <div className="item"><div className="main"><div className="sub">Details</div><div style={{ whiteSpace: 'pre-wrap' }}>{report.details}</div></div></div>}
        {report.legal && <div className="item"><div className="main"><div className="sub">Legal notice</div><pre style={{ margin: 0, whiteSpace: 'pre-wrap', fontSize: 12 }}>{JSON.stringify(report.legal, null, 2)}</pre></div></div>}
        {report.decision && <div className="item"><div className="main"><div className="sub">Decision</div><div>{report.decision}</div></div></div>}
      </div>
      {report.status === 'open' && (
        <>
          <label className="field">
            Statement of reasons (sent to the affected user)
            <textarea className="input" rows={3} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Which policy, what was found, what we did." />
          </label>
          {report.target_type === 'asset' && (
            <label className="check"><input type="checkbox" checked={blockHash} onChange={(e) => setBlockHash(e.target.checked)} /> Add this file’s hash to the blocklist so re-uploads are refused</label>
          )}
          {err && <div className="form-error">{err}</div>}
          <div className="row" style={{ flexWrap: 'wrap' }}>
            {(ACTIONS[report.target_type] ?? []).map((a) => <button key={a.id} className={`btn${a.id === 'restore' ? '' : ' danger'}`} onClick={() => decide(a.id)}>{a.label}</button>)}
            <span className="grow" />
            <button className="btn" onClick={() => decide('dismiss')}>No violation</button>
          </div>
        </>
      )}
    </Modal>
  );
}

function Users() {
  const [q, setQ] = useState('');
  const [rows, setRows] = useState<Awaited<ReturnType<typeof adminApi.users>>['users']>([]);
  const load = useCallback(() => adminApi.users(q).then((r) => setRows(r.users)), [q]);
  useEffect(() => {
    const t = setTimeout(() => void load(), 250);
    return () => clearTimeout(t);
  }, [load]);
  return (
    <>
      <input className="input" placeholder="Search by email or name" value={q} onChange={(e) => setQ(e.target.value)} />
      <table className="table">
        <thead><tr><th>User</th><th>Designs</th><th>Joined</th><th>Status</th><th /></tr></thead>
        <tbody>
          {rows.map((u) => (
            <tr key={u.id}>
              <td><div>{u.name} {u.is_staff && <span className="pill">staff</span>}</div><div className="faint">{u.email}</div></td>
              <td>{u.designs}</td>
              <td>{timeAgo(u.created_at)}</td>
              <td>{u.disabled_at ? <span className="pill warn" title={u.disabled_reason ?? ''}>disabled</span> : u.email_verified_at ? <span className="pill ok">verified</span> : <span className="pill">unverified</span>}</td>
              <td>
                {u.disabled_at
                  ? <button className="btn sm" onClick={() => adminApi.userAction(u.id, 'enable').then(load)}>Enable</button>
                  : !u.is_staff && <button className="btn sm danger" onClick={() => { const reason = prompt('Reason (shown in the audit log)'); if (reason) void adminApi.userAction(u.id, 'disable', reason).then(load); }}>Disable</button>}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

function Audit() {
  const [data, setData] = useState<Awaited<ReturnType<typeof adminApi.audit>> | null>(null);
  useEffect(() => {
    adminApi.audit().then(setData);
  }, []);
  if (!data) return <div className="spinner" />;
  return (
    <>
      <div className={`banner`} style={data.brokenAt ? { background: 'color-mix(in srgb, var(--danger) 14%, transparent)' } : undefined}>
        {data.brokenAt ? `Hash chain broken at event #${data.brokenAt} — the log was altered.` : 'Hash chain verified: no events were altered or removed.'}
      </div>
      <table className="table">
        <thead><tr><th>#</th><th>When</th><th>Actor</th><th>Action</th><th>Target</th><th>IP</th></tr></thead>
        <tbody>
          {data.events.map((e) => (
            <tr key={e.id}><td>{e.id}</td><td>{timeAgo(e.created_at)}</td><td>{e.actor ?? '—'}</td><td><code>{e.action}</code></td><td><code>{e.target ?? ''}</code></td><td>{e.ip ?? ''}</td></tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

function Jobs() {
  const [rows, setRows] = useState<Awaited<ReturnType<typeof adminApi.jobs>>['jobs']>([]);
  useEffect(() => {
    const load = () => adminApi.jobs().then((r) => setRows(r.jobs));
    void load();
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
  }, []);
  return (
    <table className="table">
      <thead><tr><th>Queue</th><th>Status</th><th>Attempts</th><th>Label</th><th>Created</th><th>Error</th></tr></thead>
      <tbody>
        {rows.map((j) => (
          <tr key={j.id}>
            <td><code>{j.queue}</code></td>
            <td><span className={`pill${j.status === 'failed' ? ' warn' : j.status === 'done' ? ' ok' : ''}`}>{j.status}</span></td>
            <td>{j.attempts}</td><td>{j.label}</td><td>{timeAgo(j.created_at)}</td><td className="faint">{j.error}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Hashes() {
  const toast = useToast();
  const [f, setF] = useState({ hash: '', source: 'internal', category: 'csam' });
  return (
    <section className="box">
      <h2>Add a perceptual hash</h2>
      <p className="muted">Uploads within 6 bits of a listed 64-bit dHash are refused and escalated. Industry lists (NCMEC, StopNCII) are imported by the same API.</p>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <input className="input" style={{ width: 220, fontFamily: 'monospace' }} placeholder="16 hex characters" value={f.hash} onChange={(e) => setF({ ...f, hash: e.target.value.trim().toLowerCase() })} />
        <input className="input" style={{ width: 130 }} value={f.source} onChange={(e) => setF({ ...f, source: e.target.value })} />
        <select className="input" style={{ width: 140 }} value={f.category} onChange={(e) => setF({ ...f, category: e.target.value })}>
          {['csam', 'ncii', 'violence', 'other'].map((c) => <option key={c}>{c}</option>)}
        </select>
        <button className="btn primary" disabled={!/^[0-9a-f]{16}$/.test(f.hash)} onClick={() => adminApi.addHash(f).then(() => { toast('Hash added'); setF({ ...f, hash: '' }); })}>Add</button>
      </div>
    </section>
  );
}

function Bars({ data, k, label }: { data: Array<Record<string, number | string>>; k: string; label: string }) {
  const max = Math.max(1, ...data.map((d) => Number(d[k])));
  const total = data.reduce((a, d) => a + Number(d[k]), 0);
  return (
    <section className="box">
      <div className="row"><h2 className="grow">{label}</h2><b>{total}</b></div>
      <div className="bars" role="img" aria-label={`${label} per day, total ${total}`}>
        {data.map((d) => <span key={String(d.day)} style={{ height: `${(Number(d[k]) / max) * 100}%` }} title={`${d.day}: ${d[k]}`} />)}
      </div>
    </section>
  );
}

function Analytics() {
  const [days, setDays] = useState(30);
  const [data, setData] = useState<Awaited<ReturnType<typeof analyticsAdminApi.get>> | null>(null);
  useEffect(() => {
    analyticsAdminApi.get(days).then(setData);
  }, [days]);
  if (!data) return <div className="spinner" />;
  const f = data.funnel;
  const pct = (a: number, b: number) => (b ? `${Math.round((a / b) * 100)}%` : '—');
  return (
    <>
      <div className="row">{[7, 30, 90].map((d) => <button key={d} className={`btn sm${d === days ? ' active' : ''}`} onClick={() => setDays(d)}>{d} days</button>)}</div>
      <div className="stat-row">
        <div className="stat"><div className="n">{f.signed_up}</div><div className="l">Signed up</div></div>
        <div className="stat"><div className="n">{f.created_design}</div><div className="l">Created a design · {pct(f.created_design, f.signed_up)}</div></div>
        <div className="stat"><div className="n">{f.exported}</div><div className="l">Exported · {pct(f.exported, f.created_design)}</div></div>
      </div>
      <Bars data={data.daily} k="active" label="Active users" />
      <Bars data={data.daily} k="signups" label="Sign-ups" />
      <Bars data={data.daily} k="designs" label="Designs created" />
      <Bars data={data.daily} k="exports" label="Background exports" />
      <section className="box">
        <h2>Product events (consented users)</h2>
        <table className="table"><tbody>{data.events.map((e) => <tr key={e.name}><td><code>{e.name}</code></td><td>{e.n}</td></tr>)}</tbody></table>
      </section>
    </>
  );
}

function Support() {
  const toast = useToast();
  const [status, setStatus] = useState('open');
  const [rows, setRows] = useState<SupportTicket[] | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const load = useCallback(() => supportApi.adminList(status).then((r) => setRows(r.tickets)), [status]);
  useEffect(() => {
    void load();
  }, [load]);
  const send = (t: SupportTicket, close: boolean) => supportApi.adminReply(t.id, draft[t.id] ?? '', close).then(() => { toast('Reply sent'); setDraft({ ...draft, [t.id]: '' }); void load(); });
  return (
    <>
      <div className="row">{['open', 'answered', 'closed'].map((s) => <button key={s} className={`btn sm${s === status ? ' active' : ''}`} onClick={() => setStatus(s)} style={{ textTransform: 'capitalize' }}>{s}</button>)}</div>
      {!rows ? <div className="spinner" /> : rows.length === 0 ? <div className="empty">No {status} tickets.</div> : rows.map((t) => (
        <section className="box" key={t.id}>
          <div className="row"><h2 className="grow">{t.subject}</h2><span className="pill">{t.topic}</span></div>
          <div className="faint" style={{ fontSize: 12 }}>{t.email} · #{t.id.slice(0, 8).toUpperCase()} · {timeAgo(t.created_at)}</div>
          {t.messages?.map((m, i) => <div key={i} className={`msg${m.fromStaff ? ' staff' : ''}`}><div className="faint" style={{ fontSize: 12 }}>{m.fromStaff ? 'Staff' : 'Customer'} · {timeAgo(m.at)}</div><div style={{ whiteSpace: 'pre-wrap' }}>{m.body}</div></div>)}
          {status !== 'closed' && (
            <>
              <textarea className="input" rows={3} placeholder="Reply (emailed to the customer)" value={draft[t.id] ?? ''} onChange={(e) => setDraft({ ...draft, [t.id]: e.target.value })} />
              <div className="row"><span className="grow" /><button className="btn" disabled={!draft[t.id]?.trim()} onClick={() => send(t, true)}>Reply and close</button><button className="btn primary" disabled={!draft[t.id]?.trim()} onClick={() => send(t, false)}>Reply</button></div>
            </>
          )}
        </section>
      ))}
    </>
  );
}

function StatusAdmin() {
  const toast = useToast();
  const [s, setS] = useState<StatusInfo | null>(null);
  const [f, setF] = useState({ title: '', impact: 'minor', components: [] as string[], status: 'investigating', body: '' });
  const [upd, setUpd] = useState<Record<string, { status: string; body: string }>>({});
  const [err, setErr] = useState('');
  const load = () => statusApi.get().then(setS);
  useEffect(() => {
    void load();
  }, []);
  if (!s) return <div className="spinner" />;
  return (
    <>
      <section className="box">
        <h2>Components</h2>
        <div className="list">{s.components.map((c) => <div className="item" key={c.id}><div className="main">{c.name}</div><span className="pill">{c.state}</span></div>)}</div>
        <a className="linkish" href="/status" target="_blank" rel="noreferrer">Open public status page</a>
      </section>
      <section className="box">
        <h2>Open an incident</h2>
        <input className="input" placeholder="Title, e.g. “Exports are slow”" value={f.title} onChange={(e) => setF({ ...f, title: e.target.value })} />
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <select className="input" style={{ width: 150 }} value={f.impact} onChange={(e) => setF({ ...f, impact: e.target.value })}>{['minor', 'major', 'critical', 'maintenance'].map((x) => <option key={x}>{x}</option>)}</select>
          <select className="input" style={{ width: 160 }} value={f.status} onChange={(e) => setF({ ...f, status: e.target.value })}>{['investigating', 'identified', 'monitoring', 'scheduled'].map((x) => <option key={x}>{x}</option>)}</select>
        </div>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          {s.components.map((c) => (
            <label key={c.id} className="check"><input type="checkbox" checked={f.components.includes(c.id)} onChange={(e) => setF({ ...f, components: e.target.checked ? [...f.components, c.id] : f.components.filter((x) => x !== c.id) })} /> {c.name}</label>
          ))}
        </div>
        <textarea className="input" rows={3} placeholder="What users see" value={f.body} onChange={(e) => setF({ ...f, body: e.target.value })} />
        {err && <div className="form-error">{err}</div>}
        <div><button className="btn primary" onClick={() => statusApi.open(f).then(() => { toast('Incident published'); setF({ ...f, title: '', body: '', components: [] }); void load(); }).catch(() => setErr('Fill in the title, at least one component and the message.'))}>Publish</button></div>
      </section>
      {s.incidents.filter((i) => !i.resolved_at).map((i) => (
        <section className="box" key={i.id}>
          <div className="row"><h2 className="grow">{i.title}</h2><span className="pill warn">{i.status}</span></div>
          <div className="row">
            <select className="input" style={{ width: 160 }} value={upd[i.id]?.status ?? 'monitoring'} onChange={(e) => setUpd({ ...upd, [i.id]: { status: e.target.value, body: upd[i.id]?.body ?? '' } })}>{['investigating', 'identified', 'monitoring', 'resolved'].map((x) => <option key={x}>{x}</option>)}</select>
            <input className="input grow" placeholder="Update" value={upd[i.id]?.body ?? ''} onChange={(e) => setUpd({ ...upd, [i.id]: { status: upd[i.id]?.status ?? 'monitoring', body: e.target.value } })} />
            <button className="btn" onClick={() => upd[i.id]?.body && statusApi.update(i.id, upd[i.id]!).then(() => { void load(); })}>Post</button>
          </div>
        </section>
      ))}
    </>
  );
}

function TemplatesAdmin() {
  const [rows, setRows] = useState<Awaited<ReturnType<typeof contentApi.adminTemplates>>['templates'] | null>(null);
  const load = useCallback(() => contentApi.adminTemplates().then((r) => setRows(r.templates)), []);
  useEffect(() => {
    void load();
  }, [load]);
  if (!rows) return <div className="spinner" />;
  return (
    <>
      <p className="muted" style={{ margin: 0 }}>Publish new ones from any design: File → Publish as template → Public catalogue.</p>
      <table className="table">
        <thead><tr><th /><th>Template</th><th>Category</th><th>Uses</th><th>Published</th><th /></tr></thead>
        <tbody>
          {rows.map((t) => (
            <tr key={t.id}>
              <td><img src={`/api/templates/${t.id}/thumbnail.png`} alt="" style={{ width: 56, height: 40, objectFit: 'contain', background: 'var(--surface-3)', borderRadius: 4 }} /></td>
              <td>{t.title}<div className="faint">{t.tags.join(', ')}</div></td>
              <td>{t.category}</td>
              <td>{t.uses}</td>
              <td><input type="checkbox" checked={t.published} onChange={(e) => contentApi.updateTemplate(t.id, { published: e.target.checked }).then(load)} aria-label="Published" /></td>
              <td><button className="btn sm danger" onClick={() => confirm(`Delete “${t.title}”?`) && contentApi.deleteTemplate(t.id).then(load)}>Delete</button></td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}
