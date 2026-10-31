/**
 * Pinselo service worker (PWA-001..004).
 * - App shell: navigations are network-first with the cached shell as the offline fallback, so the
 *   editor opens without a connection (designs themselves are kept offline by y-indexeddb).
 * - Hashed build assets (including the WebAssembly engine) and font files are immutable: cache-first.
 * - Everything else under /api, /v1, /collab and /site goes straight to the network (never cached:
 *   it is private, and the server is the source of truth).
 * A new version waits until the page asks it to take over (the app shows "Update available").
 */
const VERSION = 'oc-v1';
const SHELL = `${VERSION}-shell`;
const STATIC = `${VERSION}-static`;
const FONTS = 'oc-fonts';
const SHELL_URLS = ['/', '/manifest.webmanifest', '/favicon.svg', '/icons/icon-192.png'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(SHELL).then((c) => c.addAll(SHELL_URLS)));
});

self.addEventListener('activate', (e) => {
  e.waitUntil((async () => {
    for (const k of await caches.keys()) if (k.startsWith('oc-v') && !k.startsWith(VERSION)) await caches.delete(k);
    await self.clients.claim();
  })());
});

self.addEventListener('message', (e) => {
  if (e.data === 'skip-waiting') self.skipWaiting();
});

async function cacheFirst(req, name, maxEntries) {
  const cache = await caches.open(name);
  const hit = await cache.match(req);
  if (hit) return hit;
  const res = await fetch(req);
  if (res.ok && res.type === 'basic') {
    await cache.put(req, res.clone());
    if (maxEntries) {
      const keys = await cache.keys();
      for (const k of keys.slice(0, Math.max(0, keys.length - maxEntries))) await cache.delete(k);
    }
  }
  return res;
}

async function shell(req) {
  try {
    const res = await fetch(req);
    if (res.ok) (await caches.open(SHELL)).put('/', res.clone());
    return res;
  } catch {
    return (await caches.match('/')) ?? new Response('<h1>You’re offline</h1><p>Reconnect to open Pinselo.</p>', { headers: { 'content-type': 'text/html; charset=utf-8' } });
  }
}

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  const p = url.pathname;
  if (p.startsWith('/api/fonts/files/')) return e.respondWith(cacheFirst(req, FONTS, 300));
  if (/^\/(api|v1|collab|site|dev)\b/.test(p)) return;
  if (req.mode === 'navigate') return e.respondWith(shell(req));
  if (p.startsWith('/assets/') || p.startsWith('/icons/')) return e.respondWith(cacheFirst(req, STATIC, 400));
});
