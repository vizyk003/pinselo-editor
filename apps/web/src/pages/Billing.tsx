/** Pricing page, billing settings, local checkout and the global upgrade prompt (PL-020..026). */
import { useAuthGate } from '../auth/AuthGate';
import { BrandLogo } from '../components/BrandLogo';
import { Check, Crown } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { ApiError, billingApi, plansEnabled, type BillingInfo, type PlanInfo } from '../api';
import { useAuth } from '../App';
import { interp, Modal, useToast } from '../components/ui';
import { track } from '../lib/analytics';
import { fmtDate, fmtNumber, getLocale, t, tn } from '../i18n';
import { fmtBytes } from './home/uploadsModel';

const money = (cents: number, currency = 'usd') => fmtNumber(cents / 100, { style: 'currency', currency: currency.toUpperCase(), maximumFractionDigits: cents % 100 ? 2 : 0 });
/** Storage amounts use the same byte format as Uploads (one formatter app-wide). */
const gb = (b: number) => fmtBytes(b, getLocale());

function usePlans() {
  const [plans, setPlans] = useState<PlanInfo[] | null>(null);
  useEffect(() => {
    billingApi.plans().then((r) => setPlans(r.plans)).catch(() => setPlans([]));
  }, []);
  return plans;
}

async function startCheckout(plan: 'pro' | 'teams', interval: 'month' | 'year', workspaceId?: string) {
  track('checkout_start', { plan, interval });
  const { url } = await billingApi.checkout({ plan, interval, workspaceId });
  location.href = url;
}

export function PlanCards({ current, onChoose, interval }: { current?: string; interval: 'month' | 'year'; onChoose: (p: PlanInfo) => void }) {
  const plans = usePlans();
  if (!plans) return <div className="spinner" />;
  return (
    <div className="plans">
      {plans.map((p) => {
        const per = p.price[interval];
        const monthly = interval === 'year' ? per / 12 : per;
        return (
          <div key={p.id} className={`plan${p.id === 'pro' ? ' featured' : ''}`}>
            <div className="row"><h3>{p.name}</h3>{p.id === 'pro' && <span className="badge">{t('Most popular')}</span>}</div>
            <p className="muted">{p.tagline}</p>
            <div className="price">
              {per === 0 ? <b>{money(0)}</b> : <><b>{money(monthly)}</b><span className="faint"> {p.id === 'teams' ? t('/ person / month') : t('/ month')}</span></>}
            </div>
            <div className="faint" style={{ fontSize: 12, minHeight: 16 }}>
              {per > 0 && (interval === 'year' ? t('{price} billed yearly', { price: money(per) }) : t('billed monthly'))}{p.minSeats > 1 ? ` · ${tn(p.minSeats, 'from {n} person', 'from {n} people')}` : ''}
            </div>
            {current === p.id ? (
              <button className="btn tall" disabled>{t('Your plan')}</button>
            ) : p.id === 'free' ? (
              <button className="btn tall" disabled={Boolean(current)} onClick={() => onChoose(p)}>{current ? t('Included') : t('Get started')}</button>
            ) : (
              <button className="btn primary tall" onClick={() => onChoose(p)}>{p.trialDays ? t('Start {days}-day free trial', { days: p.trialDays }) : t('Get {plan}', { plan: p.name })}</button>
            )}
            <ul>{p.highlights.map((h) => <li key={h}><Check size={15} /> {h}</li>)}</ul>
          </div>
        );
      })}
    </div>
  );
}

function IntervalToggle({ value, onChange }: { value: 'month' | 'year'; onChange: (v: 'month' | 'year') => void }) {
  return (
    <div className="segmented" role="radiogroup" aria-label={t('Billing period')}>
      <button role="radio" aria-checked={value === 'month'} className={value === 'month' ? 'on' : ''} onClick={() => onChange('month')}>{t('Monthly')}</button>
      <button role="radio" aria-checked={value === 'year'} className={value === 'year' ? 'on' : ''} onClick={() => onChange('year')}>{t('Yearly')} <span className="badge">{t('save up to 33%')}</span></button>
    </div>
  );
}

/** Public /pricing page. */
export function Pricing() {
  const { user } = useAuth();
  const { requireAuth, openAuth } = useAuthGate();
  const nav = useNavigate();
  const [params] = useSearchParams();
  const toast = useToast();
  const [interval, setInterval] = useState<'month' | 'year'>('year');
  const [teamPick, setTeamPick] = useState(false);
  const [current, setCurrent] = useState<string | undefined>();
  useEffect(() => {
    if (user) billingApi.get().then((b) => setCurrent(b.entitlements.plan)).catch(() => {});
  }, [user]);
  useEffect(() => {
    track('upgrade_view', { from: 'pricing' });
    if (params.get('checkout') === 'cancelled') toast(t('Checkout cancelled — nothing was charged.'));
  }, [params, toast]);
  const choose = async (p: PlanInfo) => {
    if (!requireAuth('/pricing', 'signup')) return;
    if (p.id === 'free') return nav('/');
    if (p.id === 'teams') return setTeamPick(true);
    try {
      await startCheckout('pro', interval);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('Checkout is unavailable right now.'));
    }
  };
  return (
    <div className="pricing">
      <header className="pricing-head">
        <Link to="/" className="logo"><BrandLogo /></Link>
        <span className="grow" />
        {user ? <Link className="btn" to="/">{t('Back to designs')}</Link> : <><button className="btn ghost" onClick={() => openAuth('login', '/pricing')}>{t('Log in')}</button><button className="btn primary" onClick={() => openAuth('signup', '/pricing')}>{t('Sign up')}</button></>}
      </header>
      <section className="pricing-hero">
        <h1>{t('Design anything, together. Free.')}</h1>
        <p className="muted">{t('Every editor feature is free. Upgrade for premium content, more storage and team controls.')}</p>
        <IntervalToggle value={interval} onChange={setInterval} />
      </section>
      <PlanCards interval={interval} onChoose={choose} current={current} />
      <p className="faint center" style={{ fontSize: 12 }}>{interp(t('Prices exclude tax where applicable. Cancel any time; you keep your plan until the end of the period. See the {terms}.'), { terms: <Link className="linkish" to="/legal/terms">{t('Terms')}</Link> })}</p>
      {teamPick && <TeamPicker interval={interval} onClose={() => setTeamPick(false)} />}
    </div>
  );
}

function TeamPicker({ interval, onClose }: { interval: 'month' | 'year'; onClose: () => void }) {
  const [info, setInfo] = useState<BillingInfo | null>(null);
  const [err, setErr] = useState('');
  useEffect(() => {
    billingApi.get().then(setInfo);
  }, []);
  const teams = info?.manageable.filter((w) => !w.personal) ?? [];
  return (
    <Modal title={t('Which team?')} onClose={onClose} size="sm">
      {!info ? <div className="spinner" /> : teams.length === 0 ? (
        <>
          <p className="muted" style={{ margin: 0 }}>{t('Teams is billed per team. Create a team first (sidebar → Create a team), then upgrade it here.')}</p>
          <Link className="btn primary" to="/">{t('Go to Home')}</Link>
        </>
      ) : (
        <div className="list">
          {teams.map((w) => (
            <div className="item" key={w.id}>
              <div className="main"><div>{w.name}</div><div className="sub">{t('Currently {plan}', { plan: w.plan })}</div></div>
              <button className="btn primary sm" disabled={w.plan === 'teams'} onClick={() => startCheckout('teams', interval, w.id).catch((e) => setErr(e instanceof ApiError ? e.message : t('Checkout failed.')))}>{t('Upgrade')}</button>
            </div>
          ))}
        </div>
      )}
      {err && <div className="form-error">{err}</div>}
    </Modal>
  );
}

/** Settings → Billing. */
export function BillingTab() {
  const toast = useToast();
  const [params] = useSearchParams();
  const [ws, setWs] = useState<string | undefined>();
  const [info, setInfo] = useState<BillingInfo | null>(null);
  const [interval, setInterval] = useState<'month' | 'year'>('year');
  const load = () => billingApi.get(ws).then(setInfo);
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ws]);
  useEffect(() => {
    if (params.get('checkout') === 'success') toast(t('Thanks! Your plan is active.'));
  }, [params, toast]);
  if (!info) return <div className="spinner" />;
  const sub = info.subscription;
  const plan = sub && ['active', 'trialing', 'past_due'].includes(sub.status) ? sub.plan : 'free';
  const personal = info.manageable.find((m) => m.personal)?.id;
  const isTeam = info.workspaceId !== personal;
  const pct = Math.min(100, (info.usage.storageBytes / info.entitlements.storageBytes) * 100);
  const act = (p: Promise<unknown>, ok: string) => p.then(() => { toast(ok); return load(); }).catch((e) => toast(e instanceof ApiError ? e.message : t('That didn’t work.')));

  return (
    <>
      {info.manageable.length > 1 && (
        <label className="field">
          {t('Workspace')}
          <select className="input" value={info.workspaceId} onChange={(e) => setWs(e.target.value)}>
            {info.manageable.map((m) => <option key={m.id} value={m.id}>{m.personal ? t('Personal') : m.name}</option>)}
          </select>
        </label>
      )}
      <section className="box">
        <div className="row">
          <Crown size={18} color={plan === 'free' ? 'var(--text-3)' : '#f59e0b'} />
          <h2 className="grow">{plan === 'free' ? t('Free plan') : plan === 'pro' ? t('Pro') : t('Teams')}</h2>
          {sub?.status === 'trialing' && <span className="pill ok">{t('Trial')}</span>}
          {sub?.status === 'past_due' && <span className="pill warn">{t('Payment failed')}</span>}
          {sub?.cancel_at_period_end && <span className="pill warn">{t('Cancels at period end')}</span>}
        </div>
        {sub && plan !== 'free' && (
          <p className="muted" style={{ margin: 0 }}>
            {(() => { const date = sub.current_period_end ? fmtDate(sub.current_period_end) : '—'; return sub.status === 'trialing' ? t('Trial ends {date}', { date }) : sub.cancel_at_period_end ? t('Ends {date}', { date }) : t('Renews {date}', { date }); })()}
            {plan === 'teams' && ` · ${tn(sub.seats, '{n} seat ({used} in use)', '{n} seats ({used} in use)', { used: info.members })}`} · {sub.interval === 'year' ? t('billed yearly') : t('billed monthly')}
          </p>
        )}
        <div className="col" style={{ gap: 6 }}>
          <div className="row"><span className="grow">{t('Storage')}</span><span className="faint">{t('{used} of {total}', { used: gb(info.usage.storageBytes), total: gb(info.entitlements.storageBytes) })}</span></div>
          <div className="meter" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}><span style={{ width: `${pct}%` }} className={pct > 90 ? 'warn' : ''} /></div>
        </div>
        <div className="row" style={{ flexWrap: 'wrap' }}>
          {plan !== 'free' && info.provider === 'stripe' && (
            <button className="btn" onClick={() => billingApi.portal(info.workspaceId).then((r) => { location.href = r.url; })}>{t('Manage payment and invoices')}</button>
          )}
          {plan !== 'free' && info.provider === 'local' && (
            sub?.cancel_at_period_end
              ? <button className="btn" onClick={() => act(billingApi.localCancel(info.workspaceId, true), t('Plan resumed'))}>{t('Resume plan')}</button>
              : <button className="btn danger" onClick={() => confirm(t('Cancel at the end of this period? You keep everything until then.')) && act(billingApi.localCancel(info.workspaceId), t('Plan will end at period end'))}>{t('Cancel plan')}</button>
          )}
          {plan === 'teams' && info.provider === 'local' && (
            <button className="btn" onClick={() => { const n = Number(prompt(t('Number of seats'), String(sub?.seats ?? 3))); if (n) void act(billingApi.localSeats(info.workspaceId, n), t('Seats updated')); }}>{t('Change seats')}</button>
          )}
        </div>
        {info.provider === 'local' && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Payments are not configured on this server: plan changes are applied without charge (development mode).')}</p>}
      </section>

      {plan === 'free' && (
        <section className="box">
          <div className="row"><h2 className="grow">{t('Upgrade')}</h2><IntervalToggle value={interval} onChange={setInterval} /></div>
          <PlanCards current={plan} interval={interval} onChoose={(p) => {
            if (p.id === 'free') return;
            if (p.id === 'teams' && !isTeam) return toast(t('Switch to a team workspace above (or create a team) to upgrade it to Teams.'));
            if (p.id === 'pro' && isTeam) return toast(t('Pro is for your personal workspace. Teams upgrades this team.'));
            startCheckout(p.id, interval, info.workspaceId).catch((e) => toast(e instanceof ApiError ? e.message : t('Checkout failed.')));
          }} />
        </section>
      )}

      {info.invoices.length > 0 && (
        <section className="box">
          <h2>{t('Invoices')}</h2>
          <table className="table">
            <thead><tr><th>{t('Date')}</th><th>{t('Amount')}</th><th>{t('Status')}</th><th /></tr></thead>
            <tbody>
              {info.invoices.map((i) => (
                <tr key={i.id}>
                  <td>{fmtDate(i.created_at)}</td><td>{money(i.amount_cents, i.currency)}</td>
                  <td><span className={`pill${i.status === 'paid' ? ' ok' : ' warn'}`}>{i.status}</span></td>
                  <td>{i.url && <a className="linkish" href={i.url} target="_blank" rel="noreferrer">{t('View')}</a>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </>
  );
}

/** /billing/checkout?session= — the local provider's confirmation page (no payment is taken). */
export function LocalCheckout() {
  const [params] = useSearchParams();
  const nav = useNavigate();
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  return (
    <div className="auth">
      <div className="auth-card">
        <Link to="/" className="logo"><BrandLogo /></Link>
        <h1 className="auth-title">{t('Confirm your plan')}</h1>
        <p className="muted" style={{ margin: 0 }}>{t('This server has no payment provider configured, so the plan is activated without a charge. In production this step is Stripe Checkout.')}</p>
        {err && <div className="form-error">{err}</div>}
        <button className="btn primary tall" disabled={busy} onClick={() => {
          setBusy(true);
          billingApi.localConfirm(params.get('session') ?? '').then(() => nav('/settings/billing?checkout=success', { replace: true })).catch((e) => { setErr(e instanceof ApiError ? e.message : t('Checkout failed.')); setBusy(false); });
        }}>{t('Activate plan')}</button>
        <Link className="btn ghost" to="/pricing">{t('Cancel')}</Link>
      </div>
    </div>
  );
}

/** Listens for 402 responses anywhere in the app and offers an upgrade (or, without paid plans, just explains the limit). */
export function UpgradeHost() {
  const [prompt, setPrompt] = useState<{ message: string } | null>(null);
  const nav = useNavigate();
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ message?: string }>).detail;
      setPrompt({ message: d?.message ?? t('This needs a paid plan.') });
      if (plansEnabled()) track('upgrade_view', { from: 'limit' });
    };
    window.addEventListener('oc:upgrade', on);
    return () => window.removeEventListener('oc:upgrade', on);
  }, []);
  if (!prompt) return null;
  // Everything is free: only fair-use limits (storage) remain and there is nothing to buy.
  if (!plansEnabled()) return (
    <Modal title={t('Limit reached')} onClose={() => setPrompt(null)} size="sm">
      <p style={{ margin: 0 }}>{prompt.message}</p>
      <div className="row"><span className="grow" /><button className="btn primary" onClick={() => setPrompt(null)}>{t('Close')}</button></div>
    </Modal>
  );
  return (
    <Modal title={t('Upgrade to keep going')} onClose={() => setPrompt(null)} size="sm">
      <div className="row" style={{ alignItems: 'flex-start' }}><Crown size={22} color="#f59e0b" /><p style={{ margin: 0 }}>{prompt.message}</p></div>
      <p className="muted" style={{ margin: 0 }}>{t('Pro includes premium templates, 1 TB of storage, 4K export and unlimited version history. Try it free for 30 days.')}</p>
      <div className="row"><span className="grow" /><button className="btn" onClick={() => setPrompt(null)}>{t('Not now')}</button><button className="btn primary" onClick={() => { setPrompt(null); nav('/pricing'); }}>{t('See plans')}</button></div>
    </Modal>
  );
}
