/**
 * End-to-end smoke test of a running stack (plan 13 stage 3.3): api + collab + render (+ worker), the way the
 * editor uses them. Signs in, creates a design, joins it through the collab server with the editor's client stack
 * (@hocuspocus/provider + yjs from apps/web), paints the first page's background, waits until the API reads the
 * edit back from Postgres, then exports the page as PNG through the render service and checks its first pixel.
 *
 *   pnpm exec tsx tests/smoke/stack.ts [api-url] [collab-ws-url]
 *     (defaults: OC_API_URL or http://localhost:4000, ws://localhost:<OC_COLLAB_PORT or 4100>)
 *   OC_SMOKE_EMAIL / OC_SMOKE_PASSWORD   sign in as an existing account instead of signing up a new one
 */
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';
import { inflateSync } from 'node:zlib';

const ROOT = resolve(import.meta.dirname, '../..');
const webRequire = createRequire(join(ROOT, 'apps/web/package.json'));
const Y: typeof import('yjs') = webRequire('yjs');
const { HocuspocusProvider } = webRequire('@hocuspocus/provider') as typeof import('@hocuspocus/provider');

const API = (process.argv[2] ?? process.env.OC_API_URL ?? 'http://localhost:4000').replace(/\/$/, '');
const COLLAB = process.argv[3] ?? `ws://localhost:${process.env.OC_COLLAB_PORT ?? 4100}`;
const COLOR = { r: 228, g: 87, b: 46 };

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
function fail(msg: string): never {
  console.error(`FAIL: ${msg}`);
  process.exit(1);
}
async function until<T>(what: string, ms: number, f: () => Promise<T | undefined> | T | undefined): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = await f();
    if (v !== undefined) return v;
    if (Date.now() > end) fail(`timed out waiting for ${what}`);
    await sleep(200);
  }
}

let cookie = '';
async function call(method: string, path: string, body?: unknown): Promise<Response> {
  const r = await fetch(API + path, {
    method,
    headers: { ...(body === undefined ? {} : { 'content-type': 'application/json' }), ...(cookie ? { cookie } : {}) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const set = r.headers.getSetCookie().map((c) => c.split(';')[0]).filter((c) => !c.endsWith('='));
  if (set.length) cookie = [...new Map([...cookie.split('; ').filter(Boolean), ...set].map((c) => [c.split('=')[0], c])).values()].join('; ');
  return r;
}
async function json(method: string, path: string, body?: unknown): Promise<any> {
  const r = await call(method, path, body);
  const text = await r.text();
  if (!r.ok) fail(`${method} ${path}: ${r.status} ${text}`);
  return JSON.parse(text);
}

/** RGB of pixel (0, 0) of an 8-bit RGB/RGBA PNG: whatever the row filter, the first pixel is stored as is. */
function firstPixel(png: Buffer): { r: number; g: number; b: number } {
  if (png.subarray(1, 4).toString() !== 'PNG') fail('export is not a PNG');
  const idat: Buffer[] = [];
  let depth = 0;
  for (let o = 8; o < png.length; ) {
    const len = png.readUInt32BE(o);
    const type = png.subarray(o + 4, o + 8).toString();
    if (type === 'IHDR') depth = png[o + 8 + 8];
    if (type === 'IDAT') idat.push(png.subarray(o + 8, o + 8 + len));
    o += 12 + len;
  }
  if (depth !== 8) fail(`unexpected PNG bit depth ${depth}`);
  const raw = inflateSync(Buffer.concat(idat));
  return { r: raw[1], g: raw[2], b: raw[3] };
}

// 1. Account.
if (process.env.OC_SMOKE_EMAIL) {
  await json('POST', '/api/auth/login', { email: process.env.OC_SMOKE_EMAIL, password: process.env.OC_SMOKE_PASSWORD });
} else {
  const email = `stack${Date.now()}${Math.floor(Math.random() * 1e6)}@example.com`;
  await json('POST', '/api/auth/signup', { email, password: 'correct horse battery', name: 'Stack Smoke', birthYear: 1990, acceptTerms: true });
}
const me = await json('GET', '/api/auth/me');
if (!me.user?.id) fail(`not signed in: ${JSON.stringify(me)}`);
console.log(`signed in as ${me.user.email}`);

// 2. A design, joined through collab like the editor does.
const { id } = await json('POST', '/api/designs', { w: 400, h: 300, unit: 'px', title: 'Stack smoke' });
const ydoc = new Y.Doc();
const provider = new HocuspocusProvider({
  url: COLLAB,
  name: `design:${id}`,
  document: ydoc,
  token: async () => (await json('GET', `/api/designs/${id}/ticket`)).token,
});
await until('the collab sync', 15_000, () => (provider.isSynced ? true : undefined));
const pages = ydoc.getMap<InstanceType<typeof Y.Map>>('pages');
if (pages.size < 1) fail('the synced document has no page');
const [pageId, page] = [...pages.entries()].sort(([, a], [, b]) => String(a.get('order')).localeCompare(String(b.get('order'))))[0];
console.log(`collab synced design ${id} (${pages.size} page)`);

// 3. Edit: paint the page; the server acknowledges once the update is in its op log.
const color = { space: 'srgb', r: COLOR.r / 255, g: COLOR.g / 255, b: COLOR.b / 255, a: 1 };
ydoc.transact(() => page.set('background', [{ k: 'solid', color }]));
await until('the collab acknowledgement', 10_000, () => (provider.unsyncedChanges === 0 ? true : undefined));
provider.destroy();
await until('the edit read back by the API', 20_000, async () => {
  const { doc } = await json('GET', `/api/designs/${id}/doc`);
  const bg = doc?.pages?.[pageId]?.background?.[0]?.color;
  return bg && Math.abs(bg.r - color.r) < 1e-6 && Math.abs(bg.g - color.g) < 1e-6 ? true : undefined;
});
console.log('edit persisted');

// 4. Export through the render service.
const r = await call('POST', `/api/designs/${id}/export`, { format: 'png', pages: [0] });
if (!r.ok) fail(`export: ${r.status} ${await r.text()}`);
const px = firstPixel(Buffer.from(await r.arrayBuffer()));
if (Math.abs(px.r - COLOR.r) > 2 || Math.abs(px.g - COLOR.g) > 2 || Math.abs(px.b - COLOR.b) > 2) fail(`exported pixel ${JSON.stringify(px)}, want ${JSON.stringify(COLOR)}`);
console.log(`PNG export has the edited background ${JSON.stringify(px)}`);
console.log('Stack smoke test passed');
process.exit(0);
