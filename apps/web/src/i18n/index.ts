/**
 * Internationalisation (I18N-001..006). Gettext-style: the English source string is the key, so
 * anything not yet translated falls back to readable English. Catalogs (`locales/<code>.json`) map
 * English → translation; plural entries map to Intl.PluralRules categories. `{name}` placeholders
 * are interpolated. The active locale drives `<html lang dir>` and every Intl formatter.
 *
 *   t('Share')                         t('Hi {name}', { name })
 *   const LABELS = [tk('Photos')] … t(LABELS[0])   (module-level constants: mark with tk, translate at use)
 *   tn(n, '{n} page', '{n} pages')     fmtDate(d), fmtNumber(n), fmtRelative(iso)
 *
 * `pnpm i18n:check` (tools/scripts/i18n-extract.ts) lists the strings and any missing or stale keys.
 */
import { useSyncExternalStore } from 'react';

export interface LocaleInfo { code: string; name: string; dir: 'ltr' | 'rtl' }
export const LOCALES: LocaleInfo[] = [
  { code: 'en', name: 'English', dir: 'ltr' },
  { code: 'es', name: 'Español', dir: 'ltr' },
  { code: 'fr', name: 'Français', dir: 'ltr' },
  { code: 'de', name: 'Deutsch', dir: 'ltr' },
  { code: 'pt-BR', name: 'Português (Brasil)', dir: 'ltr' },
  { code: 'it', name: 'Italiano', dir: 'ltr' },
  { code: 'ja', name: '日本語', dir: 'ltr' },
  { code: 'zh-CN', name: '简体中文', dir: 'ltr' },
  { code: 'ar', name: 'العربية', dir: 'rtl' },
];

type Plural = Partial<Record<Intl.LDMLPluralRule, string>>;
type Catalog = Record<string, string | Plural>;

const loaders = import.meta.glob<{ default: Catalog }>('./locales/*.json');
let locale = 'en';
let catalog: Catalog = {};
let plural = new Intl.PluralRules('en');
let version = 0;
const listeners = new Set<() => void>();
const STORE_KEY = 'oc.locale';

/** Best supported match for a BCP-47 tag (exact, then language-only). */
export function matchLocale(tag: string | null | undefined): string | null {
  if (!tag) return null;
  const exact = LOCALES.find((l) => l.code.toLowerCase() === tag.toLowerCase());
  if (exact) return exact.code;
  const lang = tag.split('-')[0]!.toLowerCase();
  return LOCALES.find((l) => l.code.split('-')[0]!.toLowerCase() === lang)?.code ?? null;
}

export function detectLocale(userLocale?: string | null): string {
  let stored: string | null = null;
  try { stored = localStorage.getItem(STORE_KEY); } catch { /* storage blocked */ }
  for (const c of [userLocale, stored, ...(navigator.languages ?? [navigator.language])]) {
    const m = matchLocale(c);
    if (m) return m;
  }
  return 'en';
}

export async function setLocale(code: string): Promise<void> {
  const next = matchLocale(code) ?? 'en';
  const load = loaders[`./locales/${next}.json`];
  catalog = next === 'en' || !load ? {} : (await load()).default;
  locale = next;
  plural = new Intl.PluralRules(next);
  try { localStorage.setItem(STORE_KEY, next); } catch { /* storage blocked */ }
  const info = LOCALES.find((l) => l.code === next)!;
  document.documentElement.lang = next;
  document.documentElement.dir = info.dir;
  version++;
  for (const l of listeners) l();
}

/** The account's language as this browser last saw it (accounts default to 'en', so 'en' alone isn't a choice). */
const ACCOUNT_KEY = 'oc.locale.account';
const readAccount = () => { try { return localStorage.getItem(ACCOUNT_KEY); } catch { return null; } };
const writeAccount = (code: string) => { try { localStorage.setItem(ACCOUNT_KEY, code); } catch { /* storage blocked */ } };

/**
 * Apply the signed-in account's language. A language the account changed to since this browser last saw it
 * always wins (also back to English, from another device); otherwise the account's default 'en' doesn't
 * override the browser's language.
 */
export async function applyAccountLocale(tag: string | null | undefined): Promise<void> {
  const want = matchLocale(tag);
  if (!want) return;
  const seen = readAccount();
  writeAccount(want);
  const apply = seen ? want !== seen : want !== 'en';
  if (apply && want !== locale) await setLocale(want);
}

/** The user picked a language (Settings): switch now and remember it as the account's. */
export async function chooseLocale(code: string): Promise<void> {
  const next = matchLocale(code) ?? 'en';
  writeAccount(next);
  await setLocale(next);
}

export const getLocale = () => locale;
export const isRtl = () => LOCALES.find((l) => l.code === locale)?.dir === 'rtl';

const fill = (s: string, vars?: Record<string, string | number>) =>
  vars ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? (typeof vars[k] === 'number' ? fmtNumber(vars[k] as number) : String(vars[k])) : m)) : s;

/** Translate a UI string. */
export function t(s: string, vars?: Record<string, string | number>): string {
  const v = catalog[s];
  return fill(typeof v === 'string' && v ? v : s, vars);
}

/** Mark a string for extraction without translating it (module-level constants); translate at use with t(). */
export const tk = <S extends string>(s: S): S => s;

/** Translate a counted string; the catalog key is `one|other` (the two English forms joined by `|`). */
export function tn(n: number, one: string, other: string, vars?: Record<string, string | number>): string {
  const v = catalog[`${one}|${other}`];
  const cat = plural.select(n);
  const form = v && typeof v === 'object' ? (v[cat] ?? v.other) : undefined;
  return fill(form ?? (n === 1 ? one : other), { n, ...vars });
}

/** Re-render on locale change. Components that call t() inside render should use this (App does, at the root). */
export function useLocale(): string {
  useSyncExternalStore((cb) => { listeners.add(cb); return () => listeners.delete(cb); }, () => version);
  return locale;
}

export const fmtNumber = (n: number, o?: Intl.NumberFormatOptions) => new Intl.NumberFormat(locale, o).format(n);
export const fmtDate = (d: Date | string | number, o: Intl.DateTimeFormatOptions = { dateStyle: 'medium' }) => new Intl.DateTimeFormat(locale, o).format(new Date(d));
export const fmtList = (items: string[]) => new Intl.ListFormat(locale, { type: 'conjunction' }).format(items);

export function fmtRelative(iso: string | Date): string {
  const s = (new Date(iso).getTime() - Date.now()) / 1000;
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const a = Math.abs(s);
  if (a < 45) return rtf.format(0, 'second');
  if (a < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (a < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  if (a < 86400 * 30) return rtf.format(Math.round(s / 86400), 'day');
  if (a < 86400 * 365) return rtf.format(Math.round(s / (86400 * 30)), 'month');
  return rtf.format(Math.round(s / (86400 * 365)), 'year');
}
