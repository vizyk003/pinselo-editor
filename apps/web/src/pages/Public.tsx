/** Public pages: legal documents, help centre (articles, contact, your requests) and the status page. */
import { useAuthGate } from '../auth/AuthGate';
import { BRAND } from '@oc/config';
import { BrandLogo } from '../components/BrandLogo';
import { CheckCircle2, AlertTriangle, XCircle, Wrench, Search } from 'lucide-react';
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { Link, NavLink, useNavigate, useParams } from 'react-router';
import { ApiError, statusApi, supportApi, type StatusInfo, type SupportTicket } from '../api';
import { useAuth } from '../App';
import { ConsentSettingsLink } from '../components/ConsentBanner';
import { interp, timeAgo } from '../components/ui';
import { HELP, HELP_CATEGORIES } from '../content/help';
import { LEGAL } from '../content/legal';
import { track } from '../lib/analytics';
import { fmtDate, t, tk } from '../i18n';

/** Help articles; the plans article only while paid plans exist. */

export function PublicShell({ children, wide }: { children: ReactNode; wide?: boolean }) {
  const { user } = useAuth();
  const { openAuth } = useAuthGate();
  return (
    <div className="public">
      <header className="public-head">
        <Link to="/" className="logo"><BrandLogo /></Link>
        <nav className="row" style={{ gap: 4 }}>
          <NavLink className="btn ghost sm" to="/help">{t('Help')}</NavLink>
          <NavLink className="btn ghost sm" to="/status">{t('Status')}</NavLink>
          <NavLink className="btn ghost sm" to="/developers/api">{t('API')}</NavLink>
        </nav>
        <span className="grow" />
        {user ? <Link className="btn" to="/">{t('Your designs')}</Link> : <><button className="btn ghost" onClick={() => openAuth('login')}>{t('Log in')}</button><button className="btn primary" onClick={() => openAuth('signup')}>{t('Sign up')}</button></>}
      </header>
      <main className={`spage${wide ? ' wide' : ''}`}>{children}</main>
      <footer className="public-foot">
        {LEGAL.map((d) => <Link key={d.slug} to={`/legal/${d.slug}`}>{d.title}</Link>)}
        <ConsentSettingsLink />
      </footer>
    </div>
  );
}

/** Renders `**bold**` and `[Key]` markup used by help articles. */
function Rich({ text }: { text: string }) {
  const parts = text.split(/(\*\*[^*]+\*\*|\[[^\]]+\])/g);
  return <>{parts.map((p, i) => p.startsWith('**') ? <b key={i}>{p.slice(2, -2)}</b> : p.startsWith('[') ? <kbd key={i} className="kbd">{p.slice(1, -1)}</kbd> : p)}</>;
}

export function LegalPage() {
  const { slug } = useParams();
  const doc = LEGAL.find((d) => d.slug === slug) ?? LEGAL[0]!;
  useEffect(() => {
    document.title = `${doc.title} – ${BRAND.name}`;
  }, [doc]);
  return (
    <PublicShell>
      <article className="prose">
        <h1>{doc.title}</h1>
        <p className="faint">{t('Last updated {date}', { date: doc.updated })}</p>
        {doc.sections.map((s) => (
          <section key={s.h}><h2>{s.h}</h2>{s.p.map((p, i) => <p key={i}>{p}</p>)}</section>
        ))}
      </article>
    </PublicShell>
  );
}

export function HelpHome() {
  const [q, setQ] = useState('');
  const results = useMemo(() => {
    const term = q.trim().toLowerCase();
    if (!term) return null;
    const words = term.split(/\s+/);
    return HELP.map((a) => ({ a, score: words.filter((w) => `${a.title} ${a.summary} ${a.body.join(' ')}`.toLowerCase().includes(w)).length }))
      .filter((r) => r.score > 0).sort((x, y) => y.score - x.score).map((r) => r.a);
  }, [q]);
  useEffect(() => {
    if (!q) return;
    const timer = setTimeout(() => track('search', { where: 'help', len: q.length }), 800);
    return () => clearTimeout(timer);
  }, [q]);
  return (
    <PublicShell>
      <h1>{t('How can we help?')}</h1>
      <div className="row input" style={{ height: 46 }}>
        <Search size={18} className="faint" />
        <input aria-label={t('Search help')} className="grow" style={{ border: 0, outline: 0, background: 'transparent' }} placeholder={t('Search for answers')} value={q} onChange={(e) => setQ(e.target.value)} autoFocus />
      </div>
      {results ? (
        results.length ? <div className="list">{results.map((a) => <ArticleRow key={a.slug} a={a} />)}</div> : <div className="empty">{interp(t('No articles match. {ask}.'), { ask: <Link className="linkish" to="/help/contact">{t('Ask us')}</Link> })}</div>
      ) : (
        HELP_CATEGORIES.map((c) => (
          <section key={c} className="box">
            <h2>{c}</h2>
            <div className="list">{HELP.filter((a) => a.category === c).map((a) => <ArticleRow key={a.slug} a={a} />)}</div>
          </section>
        ))
      )}
      <section className="box">
        <h2>{t('Still stuck?')}</h2>
        <div className="row"><Link className="btn primary" to="/help/contact">{t('Contact support')}</Link><Link className="btn" to="/help/requests">{t('Your requests')}</Link></div>
      </section>
    </PublicShell>
  );
}

function ArticleRow({ a }: { a: (typeof HELP)[number] }) {
  return (
    <Link className="item" to={`/help/${a.slug}`}>
      <div className="main"><div style={{ fontWeight: 600 }}>{a.title}</div><div className="sub">{a.summary}</div></div>
    </Link>
  );
}

export function HelpArticlePage() {
  const { slug } = useParams();
  const a = HELP.find((x) => x.slug === slug);
  useEffect(() => {
    if (a) track('help_view', { slug: a.slug });
  }, [a]);
  if (!a) return <PublicShell><div className="empty">{t('Article not found.')} <Link className="linkish" to="/help">{t('Back to help')}</Link></div></PublicShell>;
  const related = HELP.filter((x) => x.category === a.category && x.slug !== a.slug);
  return (
    <PublicShell>
      <Link className="linkish" to="/help">← {t('Help centre')}</Link>
      <article className="prose">
        <p className="faint">{a.category}</p>
        <h1>{a.title}</h1>
        {a.body.map((p, i) => <p key={i}><Rich text={p} /></p>)}
      </article>
      {related.length > 0 && <section className="box"><h2>{t('Related')}</h2><div className="list">{related.map((r) => <ArticleRow key={r.slug} a={r} />)}</div></section>}
    </PublicShell>
  );
}

const TOPICS: Array<[string, string]> = [
  ['account', tk('Account and sign-in')], ['editor', tk('Using the editor')], ['export', tk('Exporting and printing')],
  ['collaboration', tk('Sharing and teams')], ['bug', tk('Something is broken')], ['feature', tk('Feature request')], ['other', tk('Something else')],
];

export function HelpContact() {
  const { user } = useAuth();
  const nav = useNavigate();
  const [f, setF] = useState({ topic: 'editor', subject: '', body: '', email: '' });
  const [err, setErr] = useState('');
  const [done, setDone] = useState<string | null>(null);
  const suggestions = useMemo(() => {
    const words = f.subject.toLowerCase().split(/\s+/).filter((w) => w.length > 3);
    return words.length ? HELP.filter((a) => words.some((w) => a.title.toLowerCase().includes(w) || a.summary.toLowerCase().includes(w))).slice(0, 3) : [];
  }, [f.subject]);
  if (done) {
    return (
      <PublicShell>
        <h1>{t('Thanks — we got it')}</h1>
        <p className="muted">{interp(user ? t('Your reference is {ref}. We usually reply within one business day by email and in Your requests.') : t('Your reference is {ref}. We usually reply within one business day by email.'), { ref: <b>#{done}</b> })}</p>
        <div className="row"><button className="btn primary" onClick={() => nav(user ? '/help/requests' : '/help')}>{user ? t('Your requests') : t('Back to help')}</button></div>
      </PublicShell>
    );
  }
  return (
    <PublicShell>
      <h1>{t('Contact support')}</h1>
      <form className="col box" onSubmit={async (e) => {
        e.preventDefault();
        setErr('');
        try {
          setDone((await supportApi.create({ topic: f.topic, subject: f.subject, body: f.body, email: user ? undefined : f.email })).ref);
        } catch (x) {
          setErr(x instanceof ApiError ? x.message ?? t('Check the form.') : t('Couldn’t send. Try again.'));
        }
      }}>
        <label className="field">{t('Topic')}<select className="input" value={f.topic} onChange={(e) => setF({ ...f, topic: e.target.value })}>{TOPICS.map(([v, l]) => <option key={v} value={v}>{t(l)}</option>)}</select></label>
        <label className="field">{t('Subject')}<input className="input" required minLength={3} maxLength={200} value={f.subject} onChange={(e) => setF({ ...f, subject: e.target.value })} /></label>
        {suggestions.length > 0 && (
          <div className="banner"><span className="grow">{t('These might help:')} {suggestions.map((s, i) => <span key={s.slug}>{i > 0 && ', '}<Link className="linkish" to={`/help/${s.slug}`}>{s.title}</Link></span>)}</span></div>
        )}
        <label className="field">{t('How can we help?')}<textarea className="input" required minLength={10} rows={7} value={f.body} onChange={(e) => setF({ ...f, body: e.target.value })} placeholder={t('What happened, what you expected, and the design link if relevant.')} /></label>
        {!user && <label className="field">{t('Your email')}<input className="input" type="email" required value={f.email} onChange={(e) => setF({ ...f, email: e.target.value })} /></label>}
        {err && <div className="form-error">{err}</div>}
        <div><button className="btn primary">{t('Send')}</button></div>
      </form>
    </PublicShell>
  );
}

export function HelpRequests() {
  const { user } = useAuth();
  const { openAuth } = useAuthGate();
  const [tickets, setTickets] = useState<SupportTicket[] | null>(null);
  const [reply, setReply] = useState<Record<string, string>>({});
  const load = () => supportApi.mine().then((r) => setTickets(r.tickets));
  useEffect(() => {
    if (user) void load();
  }, [user]);
  if (!user) return <PublicShell><div className="empty">{interp(t('{login} to see your requests.'), { login: <button className="linkish" onClick={() => openAuth('login', '/help/requests')}>{t('Log in')}</button> })}</div></PublicShell>;
  return (
    <PublicShell>
      <h1>{t('Your requests')}</h1>
      {!tickets ? <div className="spinner" /> : tickets.length === 0 ? <div className="empty">{t('No requests yet.')}</div> : tickets.map((tkt) => (
        <section className="box" key={tkt.id}>
          <div className="row"><h2 className="grow">{tkt.subject}</h2><span className={`pill${tkt.status === 'answered' ? ' ok' : ''}`}>{tkt.status}</span></div>
          <div className="faint" style={{ fontSize: 12 }}>#{tkt.id.slice(0, 8).toUpperCase()} · {timeAgo(tkt.created_at)}</div>
          {tkt.messages?.map((m, i) => (
            <div key={i} className={`msg${m.fromStaff ? ' staff' : ''}`}><div className="faint" style={{ fontSize: 12 }}>{m.fromStaff ? t('{brand} support', { brand: BRAND.name }) : t('You')} · {timeAgo(m.at)}</div><div style={{ whiteSpace: 'pre-wrap' }}>{m.body}</div></div>
          ))}
          {tkt.status !== 'closed' && (
            <div className="row">
              <input className="input grow" placeholder={t('Reply')} value={reply[tkt.id] ?? ''} onChange={(e) => setReply({ ...reply, [tkt.id]: e.target.value })} />
              <button className="btn" disabled={!reply[tkt.id]?.trim()} onClick={() => supportApi.reply(tkt.id, reply[tkt.id]!).then(() => { setReply({ ...reply, [tkt.id]: '' }); void load(); })}>{t('Send')}</button>
            </div>
          )}
        </section>
      ))}
    </PublicShell>
  );
}

const STATE: Record<string, { label: string; icon: ReactNode; cls: string }> = {
  operational: { label: tk('Operational'), icon: <CheckCircle2 size={18} />, cls: 'ok' },
  degraded: { label: tk('Degraded'), icon: <AlertTriangle size={18} />, cls: 'warnish' },
  outage: { label: tk('Outage'), icon: <XCircle size={18} />, cls: 'bad' },
  maintenance: { label: tk('Maintenance'), icon: <Wrench size={18} />, cls: 'warnish' },
};

export function StatusPage() {
  const [s, setS] = useState<StatusInfo | null>(null);
  useEffect(() => {
    const load = () => statusApi.get().then(setS).catch(() => setS(null));
    void load();
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  }, []);
  return (
    <PublicShell>
      <h1>{t('System status')}</h1>
      {!s ? <div className="box"><div className="row status-bad"><XCircle size={20} /> {t('We can’t reach the status service — something is likely wrong.')}</div></div> : (
        <>
          <div className={`status-banner ${STATE[s.overall]!.cls}`}>
            {STATE[s.overall]!.icon}
            <b>{s.overall === 'operational' ? t('All systems operational') : s.overall === 'degraded' ? t('Some systems are degraded') : t('We’re having an outage')}</b>
            <span className="grow" /><span className="faint" style={{ fontSize: 12 }}>{t('Checked {when}', { when: timeAgo(s.checkedAt) })}</span>
          </div>
          <section className="box">
            <div className="list">
              {s.components.map((c) => (
                <div className="item" key={c.id}><div className="main">{c.name}</div><span className={`status-${STATE[c.state]!.cls} row`} style={{ gap: 6 }}>{STATE[c.state]!.icon}{t(STATE[c.state]!.label)}</span></div>
              ))}
            </div>
          </section>
          <h2 style={{ margin: 0 }}>{t('Incidents')}</h2>
          {s.incidents.length === 0 ? <p className="muted" style={{ margin: 0 }}>{t('No incidents in the last 14 days.')}</p> : s.incidents.map((i) => (
            <section key={i.id} className="box">
              <div className="row"><h2 className="grow">{i.title}</h2><span className={`pill${i.resolved_at ? ' ok' : ' warn'}`}>{i.status}</span></div>
              {i.updates?.map((u, k) => <p key={k} style={{ margin: 0 }}><b style={{ textTransform: 'capitalize' }}>{u.status}</b> — {u.body} <span className="faint">({fmtDate(u.at, { dateStyle: 'medium', timeStyle: 'short' })})</span></p>)}
            </section>
          ))}
        </>
      )}
    </PublicShell>
  );
}
