/**
 * First-party analytics client (PL-040): consent-gated, batched, sent to our own API only.
 * Nothing is recorded until the visitor allows analytics in the consent banner.
 */
type Consent = { analytics: boolean; decidedAt: string; version: string };

const KEY = 'oc:consent';
const ANON = 'oc:anon';
export const CONSENT_VERSION = '2026-10-01';

function store<T>(k: string, v?: T): T | null {
  try {
    if (v !== undefined) localStorage.setItem(k, JSON.stringify(v));
    const raw = localStorage.getItem(k);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

export function anonId(): string {
  let id = store<string>(ANON);
  if (!id) id = store(ANON, crypto.randomUUID().replace(/-/g, '')) ?? 'unknown';
  return id;
}

export function consent(): Consent | null {
  const c = store<Consent>(KEY);
  return c && c.version === CONSENT_VERSION ? c : null;
}

export async function setConsent(analytics: boolean): Promise<void> {
  store<Consent>(KEY, { analytics, decidedAt: new Date().toISOString(), version: CONSENT_VERSION });
  if (!analytics) queue.length = 0;
  window.dispatchEvent(new Event('oc:consent'));
  await fetch('/api/consent', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ anonId: anonId(), analytics, marketing: false }) }).catch(() => {});
}

const queue: Array<{ name: string; props?: Record<string, unknown>; path: string }> = [];
let timer: ReturnType<typeof setTimeout> | null = null;

function flush() {
  timer = null;
  if (!queue.length || !consent()?.analytics) return;
  const body = JSON.stringify({ anonId: anonId(), events: queue.splice(0, 50) });
  const blob = new Blob([body], { type: 'application/json' });
  if (!navigator.sendBeacon?.('/api/events', blob)) void fetch('/api/events', { method: 'POST', headers: { 'content-type': 'application/json' }, body, keepalive: true }).catch(() => {});
}

export function track(name: string, props?: Record<string, unknown>): void {
  if (!consent()?.analytics) return;
  queue.push({ name, props, path: location.pathname });
  timer ??= setTimeout(flush, 4000);
}

addEventListener('pagehide', flush);
