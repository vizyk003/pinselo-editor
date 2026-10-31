/**
 * "Print your design" (A18): product → options and quantity → print check (with fixes) → shipping address →
 * price summary → checkout through the billing provider (Stripe Checkout, or the local confirmation page).
 */
import {
  expectedPages, preflightPrint, preflightTargetFor, quotePrint, type PreflightResult, type PriceBreakdown, type PrintCatalog, type PrintProduct,
} from '@oc/engine';
import { ArrowLeft, Check, Package, Printer } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { ApiError } from '../api';
import { Modal, useToast } from '../components/ui';
import { useDoc, useEd } from '../editor/context';
import { t, tk } from '../i18n';
import { AddressForm, formatAddress } from './Address';
import { money, printApi, type CatalogInfo, type SavedAddress } from './api';
import { PreflightList, usePreflightFixes } from './Preflight';

/** Default catalogue strings, marked for translation (operator catalogues fall back to their own text). */
export const CATALOG_STRINGS = [
  tk('Business cards'), tk('Flyers'), tk('Posters'), tk('Invitations'), tk('Postcards'), tk('Stickers'), tk('T-shirts'), tk('Mugs'), tk('Tote bags'),
  tk('Paper'), tk('Sides'), tk('Front only'), tk('Front and back'), tk('Envelopes'), tk('No envelopes'), tk('White envelopes'), tk('Kraft envelopes'),
  tk('Cut shape'), tk('Square'), tk('Rounded corners'), tk('Circle'), tk('Material'), tk('Matte vinyl'), tk('Glossy vinyl'), tk('Shirt size'),
  tk('Shirt colour'), tk('White'), tk('Black'), tk('Bag colour'), tk('Natural'), tk('Standard'), tk('Express'),
  tk('Standard — 350 gsm silk'), tk('Premium — 450 gsm soft-touch'), tk('Standard — 170 gsm silk'), tk('Premium — 250 gsm matte'),
  tk('Matte — 200 gsm'), tk('Satin — 250 gsm premium'), tk('Premium — 400 gsm textured'), tk('Premium — 400 gsm matte'),
  tk('Crisp cards on thick stock. Standard 350 gsm silk or premium 450 gsm soft-touch.'), tk('Single sheets for events, menus and promotions.'),
  tk('Large-format prints on poster paper.'), tk('5 × 7 in cards for weddings, parties and events, with matching envelopes.'),
  tk('Mail-ready cards with room for an address on the back.'), tk('Waterproof vinyl stickers, cut square, round or with rounded corners.'),
  tk('Unisex cotton tees printed on the front (direct-to-garment). Your design is the print area.'),
  tk('11 oz white ceramic mugs, printed all the way around. Your design is the wrap area.'), tk('Natural cotton totes printed on one side. Your design is the print area.'),
  tk('Contour (die-cut) cutting along your artwork is not available yet; choose the shape closest to your design.'),
];

type Step = 'product' | 'check' | 'ship' | 'review';
const STEPS: Array<{ id: Step; label: string }> = [
  { id: 'product', label: tk('Product') }, { id: 'check', label: tk('Print check') }, { id: 'ship', label: tk('Shipping') }, { id: 'review', label: tk('Review & pay') },
];

/** The product and size closest to the design's dimensions (either orientation). */
function bestMatch(c: PrintCatalog, w: number, h: number): { product: PrintProduct; sizeId: string } {
  let best = { product: c.products[0]!, sizeId: c.products[0]!.sizes[0]!.id, d: Infinity };
  for (const p of c.products) {
    for (const s of p.sizes) {
      const d1 = Math.abs(s.w - w) + Math.abs(s.h - h);
      const d2 = p.rotatable ? Math.abs(s.h - w) + Math.abs(s.w - h) : Infinity;
      const d = Math.min(d1, d2);
      if (d < best.d) best = { product: p, sizeId: s.id, d };
    }
  }
  return best;
}

export function PrintDialog({ onClose, onBack }: { onClose: () => void; onBack?: () => void }) {
  const { design, engine } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const fixes = usePreflightFixes();
  const [info, setInfo] = useState<CatalogInfo | null>(null);
  const [step, setStep] = useState<Step>('product');
  const [productId, setProductId] = useState('');
  const [sizeId, setSizeId] = useState('');
  const [options, setOptions] = useState<Record<string, string>>({});
  const [quantity, setQuantity] = useState(0);
  const [ack, setAck] = useState(false);
  const [addresses, setAddresses] = useState<SavedAddress[] | null>(null);
  const [countries, setCountries] = useState<string[]>([]);
  const [addressId, setAddressId] = useState('');
  const [adding, setAdding] = useState(false);
  const [shipping, setShipping] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  useEffect(() => {
    printApi.catalog().then((c) => {
      setInfo(c);
      const m = bestMatch(c.catalog, doc.meta.dims.w, doc.meta.dims.h);
      setProductId(m.product.id);
      setSizeId(m.sizeId);
      setQuantity(m.product.quantities[0] ?? m.product.minQuantity);
      setShipping(c.catalog.shipping.methods[0]?.id ?? '');
    }).catch(() => setErr(t('Printing isn’t available right now.')));
    printApi.addresses().then((r) => {
      setAddresses(r.addresses);
      setCountries(r.countries);
      setAddressId(r.addresses.find((a) => a.is_default)?.id ?? r.addresses[0]?.id ?? '');
      if (!r.addresses.length) setAdding(true);
    }).catch(() => setAddresses([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const catalog = info?.catalog;
  const product = catalog?.products.find((p) => p.id === productId);
  const pickProduct = (p: PrintProduct) => {
    setProductId(p.id);
    setSizeId(p.sizes[0]!.id);
    setOptions({});
    setQuantity(p.quantities[0] ?? p.minQuantity);
    setAck(false);
  };
  const preflight: PreflightResult | null = useMemo(() => {
    if (!product) return null;
    try {
      return preflightPrint(doc, preflightTargetFor(product, sizeId, options), { availableFonts: engine.fonts });
    } catch {
      return null;
    }
  }, [doc, product, sizeId, options, engine]);
  const address = addresses?.find((a) => a.id === addressId)?.address;
  const quote: PriceBreakdown | null = useMemo(() => {
    if (!catalog || !product) return null;
    try {
      return quotePrint(catalog, { productId, sizeId, options, quantity }, address ? { country: address.country, state: address.state } : {}, shipping || undefined, info!.currency);
    } catch {
      return null;
    }
  }, [catalog, product, productId, sizeId, options, quantity, address, shipping, info]);

  const qtyValid = product ? quantity >= product.minQuantity && quantity <= product.maxQuantity : false;
  const canNext: Record<Step, boolean> = {
    product: Boolean(product && qtyValid && quote),
    check: Boolean(preflight && preflight.ok && (preflight.counts.warning === 0 || ack)),
    ship: Boolean(addressId && shipping && !adding),
    review: true,
  };
  const idx = STEPS.findIndex((s) => s.id === step);
  const go = (d: 1 | -1) => setStep(STEPS[Math.max(0, Math.min(STEPS.length - 1, idx + d))]!.id);

  const placeOrder = async () => {
    if (!product) return;
    setBusy(true);
    setErr('');
    try {
      const r = await printApi.order({ designId: design.id, productId, sizeId, options, quantity, shippingMethod: shipping, addressId, acknowledgeWarnings: ack });
      location.href = r.url;
    } catch (e) {
      const body = (e as { body?: { error?: string; message?: string } }).body;
      if (body?.error === 'preflight_failed' || body?.error === 'preflight_warnings') setStep('check');
      setErr(e instanceof ApiError ? (e.message || t('Couldn’t place the order.')) : t('Couldn’t place the order.'));
      setBusy(false);
    }
  };

  const title = t('Print your design');
  if (!info || !catalog || !product) {
    return (
      <Modal title={title} onClose={onClose} size="md">
        {err ? <p className="form-error">{err}</p> : <div className="spinner" />}
      </Modal>
    );
  }

  return (
    <Modal title={title} onClose={onClose} size="lg">
      <ol className="row" style={{ listStyle: 'none', margin: 0, padding: 0, gap: 6, flexWrap: 'wrap', fontSize: 12 }} aria-label={t('Steps')}>
        {STEPS.map((s, i) => (
          <li key={s.id} className={`chip${s.id === step ? ' on' : ''}`} style={{ display: 'flex', alignItems: 'center', gap: 4, height: 26, cursor: 'default' }} aria-current={s.id === step ? 'step' : undefined}>
            {i < idx ? <Check size={12} /> : <span>{i + 1}</span>} {t(s.label)}
          </li>
        ))}
      </ol>

      {step === 'product' && (
        <>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(130px, 1fr))', gap: 8 }} role="radiogroup" aria-label={t('Product')}>
            {catalog.products.map((p) => (
              <button key={p.id} type="button" role="radio" aria-checked={p.id === productId} className={`chip${p.id === productId ? ' on' : ''}`}
                style={{ height: 'auto', padding: '10px 12px', borderRadius: 10, textAlign: 'start', display: 'flex', flexDirection: 'column', alignItems: 'flex-start', gap: 2 }}
                onClick={() => pickProduct(p)}>
                <Package size={16} /> <span>{t(p.name)}</span>
                <span className="faint" style={{ fontWeight: 400, fontSize: 11 }}>{t('from {price}', { price: money(Math.min(...Object.values(p.prices).flat().map((x) => x.unit)), info.currency) })}</span>
              </button>
            ))}
          </div>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t(product.description)}</p>
          {product.note && <div className="banner">{t(product.note)}</div>}
          <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>
            <label className="field" style={{ flex: 1, minWidth: 160 }}>{t('Size')}
              <select className="input" value={sizeId} onChange={(e) => { setSizeId(e.target.value); setAck(false); }}>
                {product.sizes.map((s) => <option key={s.id} value={s.id}>{t(s.label)}</option>)}
              </select>
            </label>
            {product.options.map((g) => (
              <label key={g.id} className="field" style={{ flex: 1, minWidth: 160 }}>{t(g.label)}
                <select className="input" value={options[g.id] ?? g.default ?? g.choices[0]?.id} onChange={(e) => { setOptions((o) => ({ ...o, [g.id]: e.target.value })); setAck(false); }}>
                  {g.choices.map((c) => <option key={c.id} value={c.id}>{t(c.label)}</option>)}
                </select>
              </label>
            ))}
            <label className="field" style={{ flex: 1, minWidth: 120 }}>{t('Quantity')}
              <input className="input" type="number" list={`oc-qty-${product.id}`} min={product.minQuantity} max={product.maxQuantity} value={quantity || ''} onChange={(e) => setQuantity(Math.floor(+e.target.value))} aria-invalid={!qtyValid} />
              <datalist id={`oc-qty-${product.id}`}>{product.quantities.map((q) => <option key={q} value={q} />)}</datalist>
              {!qtyValid && <span className="form-error">{t('Order between {min} and {max}.', { min: product.minQuantity, max: product.maxQuantity })}</span>}
            </label>
          </div>
          {quote && (
            <p className="muted" style={{ margin: 0 }}>
              {t('{total} for {n} ({unit} each, before shipping)', { total: money(quote.subtotal, quote.currency), n: quantity, unit: money(Math.round(quote.subtotal / quantity), quote.currency) })}
            </p>
          )}
          {expectedPages(product, options) > 1 && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Page 1 prints on the front and page 2 on the back.')}</p>}
        </>
      )}

      {step === 'check' && preflight && (
        <>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>
            {t('We checked your design against {product} ({size}): {bleed} mm bleed, {safe} mm safe zone.', {
              product: t(product.name), size: t(product.sizes.find((s) => s.id === sizeId)?.label ?? sizeId), bleed: product.bleedMm, safe: product.safeMm,
            })}
          </p>
          <PreflightList result={preflight} fixes={fixes} />
          {preflight.ok && preflight.counts.warning > 0 && (
            <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> {t('I’ve reviewed the warnings and want to print as is')}</label>
          )}
          {!preflight.ok && <p className="form-error" style={{ margin: 0 }}>{t('Fix the problems above to continue.')}</p>}
        </>
      )}

      {step === 'ship' && (
        <>
          {addresses === null ? <div className="spinner" /> : adding ? (
            <AddressForm countries={countries} onCancel={addresses.length ? () => setAdding(false) : undefined}
              onSaved={(a) => { setAddresses((l) => [a, ...(l ?? []).map((x) => (a.is_default ? { ...x, is_default: false } : x))]); setAddressId(a.id); setAdding(false); }} />
          ) : (
            <div className="col" style={{ gap: 6 }} role="radiogroup" aria-label={t('Shipping address')}>
              {addresses.map((a) => (
                <label key={a.id} className="row" style={{ gap: 8, fontSize: 13, alignItems: 'flex-start' }}>
                  <input type="radio" name="oc-address" checked={a.id === addressId} onChange={() => setAddressId(a.id)} />
                  <span className="grow">{formatAddress(a.address)}{a.is_default && <span className="pill" style={{ marginInlineStart: 6 }}>{t('Default')}</span>}</span>
                </label>
              ))}
              <button type="button" className="btn sm" style={{ alignSelf: 'flex-start' }} onClick={() => setAdding(true)}>{t('Add a new address')}</button>
            </div>
          )}
          {!adding && (
            <label className="field">{t('Delivery')}
              <select className="input" value={shipping} onChange={(e) => setShipping(e.target.value)}>
                {catalog.shipping.methods.map((m) => <option key={m.id} value={m.id}>{t('{method} — {min}–{max} business days after printing', { method: t(m.label), min: m.days[0], max: m.days[1] })}</option>)}
              </select>
            </label>
          )}
        </>
      )}

      {step === 'review' && quote && address && (
        <>
          <table className="table" style={{ fontSize: 13 }}>
            <tbody>
              {quote.lines.map((l, i) => (
                <tr key={i}>
                  <td>{l.kind === 'product' ? t('{product}, {size} × {n}', { product: t(product.name), size: t(product.sizes.find((s) => s.id === sizeId)?.label ?? ''), n: quantity })
                    : l.kind === 'shipping' ? t('Shipping ({method})', { method: t(catalog.shipping.methods.find((m) => m.id === shipping)?.label ?? shipping) })
                      : l.kind === 'tax' ? t('Estimated tax ({rate}%)', { rate: quote.taxRate }) : l.label}</td>
                  <td style={{ textAlign: 'end' }}>{money(l.amount, quote.currency)}</td>
                </tr>
              ))}
              <tr><td><b>{t('Total')}</b></td><td style={{ textAlign: 'end' }}><b>{money(quote.total, quote.currency)}</b></td></tr>
            </tbody>
          </table>
          <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('Ship to: {address}', { address: formatAddress(address) })}</p>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>
            {info.payment === 'local'
              ? t('Payments aren’t configured on this server, so the order is confirmed without a charge (development mode).')
              : t('You’ll pay securely with Stripe. Tax is an estimate and is finalised at checkout.')}
            {' '}{t('We print the design as it is when your payment goes through.')}
          </p>
        </>
      )}

      {err && <p className="form-error" style={{ margin: 0 }}>{err}</p>}

      <div className="row" style={{ gap: 8 }}>
        {idx > 0 ? <button type="button" className="btn" onClick={() => go(-1)}><ArrowLeft size={14} /> {t('Back')}</button>
          : onBack && <button type="button" className="btn" onClick={onBack}><ArrowLeft size={14} /> {t('Download')}</button>}
        <span className="grow" />
        {step !== 'review'
          ? <button type="button" className="btn primary" disabled={!canNext[step]} onClick={() => go(1)}>{t('Continue')}</button>
          : <button type="button" className="btn primary" disabled={busy || !quote || !canNext.check || !canNext.ship} onClick={() => void placeOrder().catch(() => toast(t('Couldn’t place the order.')))}>
            <Printer size={16} /> {busy ? t('Placing order…') : t('Pay {total}', { total: quote ? money(quote.total, quote.currency) : '' })}
          </button>}
      </div>
    </Modal>
  );
}
