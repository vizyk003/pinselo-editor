/** Shipping address form with client-side validation (the API validates again). */
import { validateAddress, type AddressField, type AddressProblem, type ShippingAddress } from '@oc/engine';
import { useMemo, useState } from 'react';
import { ApiError } from '../api';
import { getLocale, t } from '../i18n';
import { printApi, type SavedAddress } from './api';

export function countryName(code: string): string {
  try {
    return new Intl.DisplayNames([getLocale()], { type: 'region' }).of(code) ?? code;
  } catch {
    return code;
  }
}

const PROBLEM: Record<AddressProblem | 'too_many', () => string> = {
  required: () => t('Required'),
  invalid: () => t('Check this'),
  unsupported: () => t('We don’t ship there yet'),
  too_long: () => t('Too long'),
  too_many: () => t('You have too many saved addresses. Delete one first.'),
};

export function formatAddress(a: ShippingAddress): string {
  return [a.name, a.company, a.line1, a.line2, [a.postalCode, a.city, a.state].filter(Boolean).join(' '), countryName(a.country)].filter(Boolean).join(', ');
}

export function AddressForm({ countries, initial, onSaved, onCancel, id }: {
  countries: string[]; initial?: Partial<ShippingAddress>; id?: string; onSaved: (a: SavedAddress) => void; onCancel?: () => void;
}) {
  const [a, setA] = useState<Partial<ShippingAddress>>(initial ?? { country: countries.includes('US') ? 'US' : countries[0] });
  const [errors, setErrors] = useState<Partial<Record<AddressField, string>>>({});
  const [busy, setBusy] = useState(false);
  const [makeDefault, setMakeDefault] = useState(false);
  const sortedCountries = useMemo(() => [...countries].sort((x, y) => countryName(x).localeCompare(countryName(y))), [countries]);
  const set = (k: AddressField) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setA((p) => ({ ...p, [k]: e.target.value }));
  const field = (k: AddressField, label: string, opts: { autoComplete?: string; optional?: boolean; type?: string } = {}) => (
    <label className="field" style={{ flex: 1, minWidth: 140 }}>
      {label}{opts.optional && <span className="faint"> ({t('optional')})</span>}
      <input className="input" type={opts.type ?? 'text'} value={(a[k] as string | undefined) ?? ''} onChange={set(k)} autoComplete={opts.autoComplete} aria-invalid={Boolean(errors[k])} />
      {errors[k] && <span className="form-error">{errors[k]}</span>}
    </label>
  );
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const v = validateAddress(a, countries);
    if (!v.ok) {
      setErrors(Object.fromEntries(Object.entries(v.errors).map(([k, p]) => [k, PROBLEM[p]()])));
      return;
    }
    setErrors({});
    setBusy(true);
    try {
      const r = await printApi.saveAddress(v.address, id, makeDefault);
      onSaved(r.address);
    } catch (err) {
      const fields = (err as { body?: { fields?: Record<string, AddressProblem | 'too_many'> } }).body?.fields;
      if (fields) setErrors(Object.fromEntries(Object.entries(fields).map(([k, p]) => [k, (PROBLEM[p] ?? PROBLEM.invalid)()])));
      else setErrors({ name: err instanceof ApiError ? err.message : t('Couldn’t save the address.') });
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="col" style={{ gap: 8 }} onSubmit={submit} noValidate>
      <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>{field('name', t('Full name'), { autoComplete: 'name' })}{field('company', t('Company'), { autoComplete: 'organization', optional: true })}</div>
      {field('line1', t('Address'), { autoComplete: 'address-line1' })}
      {field('line2', t('Apartment, suite, etc.'), { autoComplete: 'address-line2', optional: true })}
      <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>
        {field('city', t('City'), { autoComplete: 'address-level2' })}
        {field('state', t('State / province'), { autoComplete: 'address-level1', optional: !['US', 'CA', 'AU', 'BR', 'MX', 'IN'].includes(a.country ?? '') })}
        {field('postalCode', t('Postal code'), { autoComplete: 'postal-code' })}
      </div>
      <label className="field">{t('Country')}
        <select className="input" value={a.country ?? ''} onChange={set('country')} autoComplete="country">
          {sortedCountries.map((c) => <option key={c} value={c}>{countryName(c)}</option>)}
        </select>
        {errors.country && <span className="form-error">{errors.country}</span>}
      </label>
      <div className="row" style={{ flexWrap: 'wrap', gap: 8 }}>{field('phone', t('Phone (for the courier)'), { autoComplete: 'tel', optional: true, type: 'tel' })}{field('email', t('Email'), { autoComplete: 'email', optional: true, type: 'email' })}</div>
      <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={makeDefault} onChange={(e) => setMakeDefault(e.target.checked)} /> {t('Use as my default address')}</label>
      <div className="row" style={{ justifyContent: 'flex-end', gap: 8 }}>
        {onCancel && <button type="button" className="btn" onClick={onCancel}>{t('Cancel')}</button>}
        <button className="btn primary" disabled={busy}>{busy ? t('Saving…') : t('Save address')}</button>
      </div>
    </form>
  );
}
