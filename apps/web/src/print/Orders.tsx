/** Print order history and detail (customers), the local checkout page and the staff print console (A18). */
import { BRAND } from '@oc/config';
import { Printer } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Link, Route, Routes, useNavigate, useParams, useSearchParams } from 'react-router';
import { ApiError } from '../api';
import { EmptyState, Modal, PageHeader, useToast } from '../components/ui';
import { fmtDate, t } from '../i18n';
import { formatAddress } from './Address';
import { money, printApi, type OrderEvent, type OrderStatus, type PrintOrder, type SavedAddress } from './api';
import { STATUS_LABEL } from './Preflight';

const STATUS_CLASS: Partial<Record<OrderStatus, string>> = { delivered: 'ok', shipped: 'ok', failed: 'warn', cancelled: 'warn', returned: 'warn', pending_payment: 'warn' };

export function StatusPill({ status }: { status: OrderStatus }) {
  return <span className={`pill${STATUS_CLASS[status] ? ` ${STATUS_CLASS[status]}` : ''}`}>{t(STATUS_LABEL[status])}</span>;
}

/** /orders and /orders/:id inside the signed-in shell. */
export function OrdersPage() {
  return (
    <Routes>
      <Route index element={<OrderList />} />
      <Route path=":id" element={<OrderDetail />} />
    </Routes>
  );
}

function OrderList() {
  const [orders, setOrders] = useState<PrintOrder[] | null>(null);
  const [addresses, setAddresses] = useState<SavedAddress[]>([]);
  useEffect(() => {
    printApi.orders().then((r) => setOrders(r.orders)).catch(() => setOrders([]));
    printApi.addresses().then((r) => setAddresses(r.addresses)).catch(() => {});
  }, []);
  return (
    <>
    <PageHeader title={t('Print orders')} />
    <div className="spage">
      {!orders ? <div className="spinner" /> : orders.length === 0 ? (
        <EmptyState icon={<Printer size={22} />}>{t('No print orders yet. Open a design and choose Download → Print your design.')}</EmptyState>
      ) : (
        <table className="table">
          <thead><tr><th>{t('Order')}</th><th>{t('Product')}</th><th>{t('Date')}</th><th>{t('Total')}</th><th>{t('Status')}</th></tr></thead>
          <tbody>
            {orders.map((o) => (
              <tr key={o.id}>
                <td><Link className="linkish" to={`/orders/${o.id}`}>{o.number}</Link></td>
                <td>{o.product_name} × {o.quantity}<div className="faint" style={{ fontSize: 12 }}>{o.design_title}</div></td>
                <td>{fmtDate(o.created_at)}</td>
                <td>{money(o.total_cents, o.currency)}</td>
                <td><StatusPill status={o.status} /></td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {addresses.length > 0 && (
        <section className="box">
          <h2>{t('Saved addresses')}</h2>
          {addresses.map((a) => (
            <div key={a.id} className="row" style={{ fontSize: 13 }}>
              <span className="grow">{formatAddress(a.address)}{a.is_default && <span className="pill" style={{ marginInlineStart: 6 }}>{t('Default')}</span>}</span>
              <button className="btn sm ghost" onClick={() => void printApi.deleteAddress(a.id).then(() => setAddresses((l) => l.filter((x) => x.id !== a.id)))}>{t('Delete')}</button>
            </div>
          ))}
        </section>
      )}
    </div>
    </>
  );
}

function Timeline({ events }: { events: OrderEvent[] }) {
  return (
    <ol className="col" style={{ gap: 6, margin: 0, paddingInlineStart: 18, fontSize: 13 }}>
      {events.map((e, i) => (
        <li key={i}>
          <b>{t(STATUS_LABEL[e.status] ?? e.status)}</b> <span className="faint">· {fmtDate(e.created_at, { dateStyle: 'medium', timeStyle: 'short' } as Intl.DateTimeFormatOptions)}</span>
          {e.actor && <span className="faint"> · {e.actor}</span>}
          {e.note && <div className="muted">{e.note}</div>}
        </li>
      ))}
    </ol>
  );
}

function OrderDetail() {
  const { id = '' } = useParams();
  const [params] = useSearchParams();
  const toast = useToast();
  const [data, setData] = useState<Awaited<ReturnType<typeof printApi.get>> | null>(null);
  const [missing, setMissing] = useState(false);
  const load = useCallback(() => printApi.get(id).then(setData).catch(() => setMissing(true)), [id]);
  useEffect(() => {
    void load();
    // Stripe redirects back before the webhook lands: poll briefly while payment is pending.
    if (params.get('checkout') !== 'success') return;
    const timer = setInterval(() => void load(), 3000);
    const stop = setTimeout(() => clearInterval(timer), 60_000);
    return () => { clearInterval(timer); clearTimeout(stop); };
  }, [load, params]);
  if (missing) return <div className="empty">{t('Order not found.')}</div>;
  if (!data) return <div className="spinner" />;
  const o = data.order;
  return (
    <div className="spage">
      <div className="row"><Link className="linkish" to="/orders">← {t('All orders')}</Link></div>
      <div className="row" style={{ gap: 10 }}>
        <h1 className="grow">{t('Order {number}', { number: o.number })}</h1>
        <StatusPill status={o.status} />
      </div>
      {params.get('checkout') === 'cancelled' && o.status === 'pending_payment' && <div className="banner">{t('Payment was cancelled. You can pay whenever you’re ready.')}</div>}
      {params.get('checkout') === 'success' && o.status === 'pending_payment' && <div className="banner">{t('Confirming your payment…')}</div>}
      <section className="box">
        <div className="row" style={{ alignItems: 'flex-start', flexWrap: 'wrap', gap: 16 }}>
          <div className="grow col" style={{ gap: 4, minWidth: 220 }}>
            <b>{o.product_name} × {o.quantity}</b>
            <span className="muted" style={{ fontSize: 13 }}>{o.design_id ? <Link className="linkish" to={`/design/${o.design_id}`}>{o.design_title}</Link> : o.design_title}</span>
            <span className="muted" style={{ fontSize: 13 }}>{t('Ship to: {address}', { address: formatAddress(o.address) })}</span>
            {o.tracking.map((tr, i) => (
              <span key={i} style={{ fontSize: 13 }}>{t('Tracking')}: {tr.url ? <a className="linkish" href={tr.url} target="_blank" rel="noreferrer">{tr.code ?? tr.url}</a> : tr.code} {tr.carrier && <span className="faint">({tr.carrier})</span>}</span>
            ))}
          </div>
          <table className="table" style={{ width: 'auto', fontSize: 13 }}>
            <tbody>
              <tr><td>{t('Subtotal')}</td><td style={{ textAlign: 'end' }}>{money(o.subtotal_cents, o.currency)}</td></tr>
              <tr><td>{t('Shipping')}</td><td style={{ textAlign: 'end' }}>{money(o.shipping_cents, o.currency)}</td></tr>
              {o.tax_cents > 0 && <tr><td>{t('Estimated tax')}</td><td style={{ textAlign: 'end' }}>{money(o.tax_cents, o.currency)}</td></tr>}
              <tr><td><b>{t('Total')}</b></td><td style={{ textAlign: 'end' }}><b>{money(o.total_cents, o.currency)}</b></td></tr>
            </tbody>
          </table>
        </div>
        <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
          {o.status === 'pending_payment' && <button className="btn primary" onClick={() => printApi.pay(o.id).then((r) => { location.href = r.url; }).catch((e) => toast(e instanceof ApiError ? e.message : t('Checkout failed.')))}>{t('Pay now')}</button>}
          {o.hasFile && <a className="btn" href={printApi.fileUrl(o.id)}>{t('Download print file')}</a>}
          {data.canCancel && o.status !== 'cancelled' && (
            <button className="btn danger" onClick={() => confirm(o.status === 'pending_payment' ? t('Cancel this order?') : t('Cancel this order? Your payment will be refunded.')) &&
              void printApi.cancel(o.id).then(load).catch((e) => toast(e instanceof ApiError ? e.message : t('Couldn’t cancel the order.')))}>{t('Cancel order')}</button>
          )}
        </div>
      </section>
      <section className="box">
        <h2>{t('History')}</h2>
        <Timeline events={data.events} />
      </section>
    </div>
  );
}

/** /print/checkout?order=&session= — the local provider's confirmation page (no payment is taken). */
export function PrintCheckout() {
  const [params] = useSearchParams();
  const nav = useNavigate();
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const order = params.get('order') ?? '';
  const [o, setO] = useState<PrintOrder | null>(null);
  useEffect(() => {
    if (order) printApi.get(order).then((r) => setO(r.order)).catch(() => {});
  }, [order]);
  return (
    <div className="auth">
      <div className="auth-card">
        <Link to="/" className="logo"><img src="/favicon.svg" alt="" />{BRAND.name}</Link>
        <h1 className="auth-title">{t('Confirm your print order')}</h1>
        {o && <p style={{ margin: 0 }}><b>{o.product_name} × {o.quantity}</b> — {money(o.total_cents, o.currency)}</p>}
        <p className="muted" style={{ margin: 0 }}>{t('This server has no payment provider configured, so the order is confirmed without a charge. In production this step is Stripe Checkout.')}</p>
        {err && <div className="form-error">{err}</div>}
        <button className="btn primary tall" disabled={busy} onClick={() => {
          setBusy(true);
          printApi.localPay(order, params.get('session') ?? '').then(() => nav(`/orders/${order}?checkout=success`, { replace: true }))
            .catch((e) => { setErr(e instanceof ApiError ? e.message : t('Checkout failed.')); setBusy(false); });
        }}>{t('Confirm order')}</button>
        <Link className="btn ghost" to={`/orders/${order}?checkout=cancelled`}>{t('Cancel')}</Link>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- staff console

export function AdminPrint() {
  const [status, setStatus] = useState('');
  const [q, setQ] = useState('');
  const [data, setData] = useState<Awaited<ReturnType<typeof printApi.admin.orders>> | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const load = useCallback(() => printApi.admin.orders(status, q).then(setData), [status, q]);
  useEffect(() => {
    void load();
  }, [load]);
  const statuses = Object.keys(STATUS_LABEL) as OrderStatus[];
  return (
    <>
      <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
        <button className={`btn sm${!status ? ' active' : ''}`} onClick={() => setStatus('')}>{t('All')}</button>
        {statuses.map((s) => <button key={s} className={`btn sm${s === status ? ' active' : ''}`} onClick={() => setStatus(s)}>{t(STATUS_LABEL[s])}{data?.counts[s] ? ` (${data.counts[s]})` : ''}</button>)}
        <span className="grow" />
        <input className="input" style={{ width: 200 }} placeholder={t('Order number or email')} value={q} onChange={(e) => setQ(e.target.value)} aria-label={t('Search orders')} />
      </div>
      {data && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Fulfilment: {providers}', { providers: data.fulfilment.join(', ') })}</p>}
      {!data ? <div className="spinner" /> : data.orders.length === 0 ? <div className="empty">{t('No print orders.')}</div> : (
        <table className="table">
          <thead><tr><th>{t('Order')}</th><th>{t('Customer')}</th><th>{t('Product')}</th><th>{t('Total')}</th><th>{t('Provider')}</th><th>{t('Status')}</th><th>{t('Created')}</th></tr></thead>
          <tbody>
            {data.orders.map((o) => (
              <tr key={o.id} onClick={() => setOpen(o.id)} style={{ cursor: 'pointer' }}>
                <td><button className="linkish" onClick={() => setOpen(o.id)}>{o.number}</button></td>
                <td>{o.email}</td>
                <td>{o.product_name} × {o.quantity}</td>
                <td>{money(o.total_cents, o.currency)}</td>
                <td>{o.provider ?? '—'}</td>
                <td><StatusPill status={o.status} /></td>
                <td>{fmtDate(o.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {open && <AdminOrder id={open} onClose={() => setOpen(null)} onChanged={() => void load()} />}
    </>
  );
}

function AdminOrder({ id, onClose, onChanged }: { id: string; onClose: () => void; onChanged: () => void }) {
  const toast = useToast();
  const [data, setData] = useState<Awaited<ReturnType<typeof printApi.admin.get>> | null>(null);
  const [next, setNext] = useState<OrderStatus | ''>('');
  const [note, setNote] = useState('');
  const [tracking, setTracking] = useState({ carrier: '', code: '', url: '' });
  const [refund, setRefund] = useState(true);
  const load = useCallback(() => printApi.admin.get(id).then((r) => { setData(r); setNext(''); }), [id]);
  useEffect(() => {
    void load();
  }, [load]);
  if (!data) return <Modal title={t('Print order')} onClose={onClose}><div className="spinner" /></Modal>;
  const o = data.order;
  const save = async () => {
    if (!next) return;
    try {
      const tr = tracking.code || tracking.url ? { carrier: tracking.carrier || undefined, code: tracking.code || undefined, url: tracking.url || undefined } : undefined;
      await printApi.admin.setStatus(o.id, { status: next, note: note || undefined, tracking: tr, refund: next === 'cancelled' ? refund : undefined });
      setNote('');
      setTracking({ carrier: '', code: '', url: '' });
      toast(t('Order updated'));
      onChanged();
      await load();
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('Couldn’t update the order.'));
    }
  };
  return (
    <Modal title={t('Order {number}', { number: o.number })} onClose={onClose} size="lg">
      <div className="row"><StatusPill status={o.status} /><span className="grow" /><span className="faint" style={{ fontSize: 12 }}>{o.provider ?? '—'}{o.provider_order_id ? ` · ${o.provider_order_id}` : ''}</span></div>
      <div style={{ fontSize: 13 }} className="col">
        <span><b>{o.product_name} × {o.quantity}</b> — {Object.entries(o.options).map(([k, v]) => `${k}: ${v}`).join(', ')}</span>
        <span>{o.email} · {money(o.total_cents, o.currency)} · {t(o.shipping_method)}</span>
        <span className="muted">{formatAddress(o.address)}{o.address.phone ? ` · ${o.address.phone}` : ''}</span>
        {o.error && <span className="form-error">{o.error}</span>}
        {o.preflight && o.preflight.issues.length > 0 && <span className="faint">{t('Print check at order time: {n} issues', { n: o.preflight.issues.length })}</span>}
      </div>
      <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
        {o.file_sha && <a className="btn sm" href={printApi.admin.fileUrl(o.id)}>{t('Download print file')}</a>}
        {o.design_id && <a className="btn sm" href={`/design/${o.design_id}`} target="_blank" rel="noreferrer">{t('Open design')}</a>}
        {(o.status === 'failed' || o.status === 'paid') && <button className="btn sm" onClick={() => void printApi.admin.retry(o.id).then(() => { toast(t('Fulfilment queued')); void load(); })}>{t('Retry fulfilment')}</button>}
      </div>
      {(o.next?.length ?? 0) > 0 && (
        <div className="col" style={{ gap: 6 }}>
          <label className="field">{t('Change status')}
            <select className="input" value={next} onChange={(e) => setNext(e.target.value as OrderStatus)}>
              <option value="">—</option>
              {o.next!.map((s) => <option key={s} value={s}>{t(STATUS_LABEL[s])}</option>)}
            </select>
          </label>
          {next === 'shipped' && (
            <div className="row" style={{ gap: 6 }}>
              <input className="input" placeholder={t('Carrier')} value={tracking.carrier} onChange={(e) => setTracking((x) => ({ ...x, carrier: e.target.value }))} aria-label={t('Carrier')} />
              <input className="input" placeholder={t('Tracking number')} value={tracking.code} onChange={(e) => setTracking((x) => ({ ...x, code: e.target.value }))} aria-label={t('Tracking number')} />
              <input className="input" placeholder={t('Tracking link')} value={tracking.url} onChange={(e) => setTracking((x) => ({ ...x, url: e.target.value }))} aria-label={t('Tracking link')} />
            </div>
          )}
          {next === 'cancelled' && o.status !== 'pending_payment' && <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={refund} onChange={(e) => setRefund(e.target.checked)} /> {t('Refund the payment')}</label>}
          <input className="input" placeholder={t('Note for the history (optional)')} value={note} onChange={(e) => setNote(e.target.value)} aria-label={t('Note')} />
          <button className="btn primary" disabled={!next} onClick={() => void save()}>{t('Update order')}</button>
        </div>
      )}
      <Timeline events={data.events} />
    </Modal>
  );
}
