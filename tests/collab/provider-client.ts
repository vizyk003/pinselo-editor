// SPDX-License-Identifier: Apache-2.0
/**
 * Protocol conformance driver for the collab server (crates/oc-collab).
 * It drives REAL browser-side clients — `@hocuspocus/provider` 3.4.4 + yjs, exactly as apps/web uses them —
 * against a server process it starts, kills and restarts itself.
 *
 *   SERVER_CMD="<shell command that starts the server>" \
 *   OC_DATABASE_URL=postgres://…/<throwaway, migrated db> OC_COLLAB_PORT=<port> OC_SECRET=<secret> \
 *   pnpm tsx tests/collab/provider-client.ts
 *
 * The server inherits this process's environment. Normally run by `cargo test -p oc-collab --test provider`,
 * which creates and drops the throwaway database (see crates/oc-collab/tests/provider.rs, including how to
 * point the same test at another server command). Exit code 0 = every scenario passed; the last stdout line is
 * a JSON summary.
 *
 * Scenarios: banner/health, auth rejection (bad / wrong-design / expired ticket, bad room), two clients editing
 * concurrently converge (+ awareness, + two documents multiplexed on one socket), read-only roles (view and
 * comment) cannot write, ack-after-persist (ack ⇒ row committed; DB failure ⇒ no ack, CLOSE "Reset Connection",
 * resend on reconnect), malformed updates refused before the op log, offline edits resync on reconnect, the
 * debounced store (1.5 s after the last change, at most 8 s while editing; op log folded, one automatic version per
 * 10 minutes, thumbnail refresh with the internal token), the last client leaving flushes the pending store,
 * load repairs a damaged document once (CO-004), persistence across SIGKILL (op-log replay) and SIGTERM (compacted
 * snapshot, designs row, version), live revocation (oc_revoke), server-side updates (oc_doc_update),
 * BroadcastStateless, messages without a server hook (Stateless, QueryAwareness, token sync, unknown types) are
 * ignored, a provider CLOSE message ends only that document connection, brand locks (CLOSE "brand lock").
 *
 * The driver also stands in for the render service: it sets OC_RENDER_URL to a local listener that records the
 * thumbnail refresh requests and answers 503 (the server must ignore render failures).
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { createHmac, randomBytes, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '../..');
// The exact client stack of apps/web (CJS builds, one yjs instance shared with the provider).
const webRequire = createRequire(join(ROOT, 'apps/web/package.json'));
const Y: typeof import('yjs') = webRequire('yjs');
const { HocuspocusProvider, HocuspocusProviderWebsocket } = webRequire('@hocuspocus/provider') as typeof import('@hocuspocus/provider');
const pg = createRequire(join(ROOT, 'tests/package.json'))('pg') as typeof import('pg');

const env = (k: string) => {
  const v = process.env[k];
  if (!v) throw new Error(`${k} is required`);
  return v;
};
const SERVER_CMD = env('SERVER_CMD');
const PORT = Number(env('OC_COLLAB_PORT'));
const SECRET = env('OC_SECRET');
const DB_URL = env('OC_DATABASE_URL');
const WS_URL = `ws://127.0.0.1:${PORT}`;
const HTTP_URL = `http://127.0.0.1:${PORT}/`;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function waitFor(cond: () => boolean | Promise<boolean>, what: string, timeoutMs = 10_000): Promise<void> {
  const end = Date.now() + timeoutMs;
  for (;;) {
    if (await cond()) return;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(25);
  }
}
function assert(c: unknown, msg: string): asserts c {
  if (!c) throw new Error(msg);
}

// ---------------------------------------------------------------- render service stand-in

interface RenderCall { path: string; internal: boolean; body: unknown }
const renderCalls: RenderCall[] = [];
const INTERNAL = createHmac('sha256', SECRET).update('internal').digest('base64url');
const render = createServer((req, res) => {
  const chunks: Buffer[] = [];
  req.on('data', (c: Buffer) => chunks.push(c));
  req.on('end', () => {
    let body: unknown = Buffer.concat(chunks).toString('utf8');
    try {
      body = JSON.parse(body as string);
    } catch {
      /* keep text */
    }
    renderCalls.push({ path: `${req.method} ${req.url}`, internal: req.headers['x-oc-internal'] === INTERNAL, body });
    // Render failures must not matter to the collab server.
    res.writeHead(503, { 'content-type': 'text/plain' });
    res.end('render busy');
  });
});
await new Promise<void>((r) => render.listen(0, '127.0.0.1', r));
process.env.OC_RENDER_URL = `http://127.0.0.1:${(render.address() as { port: number }).port}`;
const thumbnails = (design: string) => renderCalls.filter((c) => c.path === 'POST /internal/thumbnail' && (c.body as { designId?: string })?.designId === design);

// ---------------------------------------------------------------- server process

let server: ChildProcess | null = null;
let exited: Promise<void> = Promise.resolve();

async function startServer() {
  // `exec`: the shell becomes the server, so the SIGTERM in stopServer reaches it. Whether `sh -c` execs or forks a
  // single command depends on the shell and its build; when it forks, SIGTERM only ends the shell, `exited`
  // resolves at once and the server stores later (CI: "doc_state written on shutdown" failed).
  server = spawn(`exec ${SERVER_CMD}`, { shell: true, detached: true, stdio: ['ignore', 'pipe', 'pipe'], env: process.env });
  const tag = (s: Buffer) => s.toString().split('\n').filter(Boolean).forEach((l) => process.stderr.write(`  [server] ${l}\n`));
  server.stdout!.on('data', tag);
  server.stderr!.on('data', tag);
  const p = server;
  exited = new Promise((r) => p.once('exit', () => r()));
  await waitFor(async () => {
    try {
      const r = await fetch(HTTP_URL);
      return r.status === 200 && (await r.text()) === 'Welcome to Hocuspocus!';
    } catch {
      return false;
    }
  }, 'the server to listen', 90_000);
}

async function stopServer(signal: 'SIGKILL' | 'SIGTERM') {
  if (!server) return;
  try {
    // SIGTERM goes to the server process itself (wrappers like tsx relay it); SIGKILL takes the whole group.
    if (signal === 'SIGKILL') process.kill(-server.pid!, signal);
    else process.kill(server.pid!, signal);
  } catch {
    /* already gone */
  }
  await Promise.race([exited, sleep(40_000)]);
  server = null;
}

// ---------------------------------------------------------------- database

const db = new pg.Client({ connectionString: DB_URL });
const userA = randomUUID();
const userB = randomUUID();
const userV = randomUUID();
const ws = randomUUID();

async function seed() {
  for (const [id, n] of [[userA, 'Ann'], [userB, 'Bob'], [userV, 'Vera']] as const) {
    await db.query('INSERT INTO users(id, email, name, color) VALUES ($1,$2,$3,$4)', [id, `${n.toLowerCase()}-${id.slice(0, 8)}@example.test`, n, '#336699']);
  }
  await db.query('INSERT INTO workspaces(id, name, personal) VALUES ($1, $2, false)', [ws, 'Collab test']);
}

async function newDesign(): Promise<string> {
  const id = randomBytes(8).toString('hex');
  await db.query('INSERT INTO designs(id, workspace_id, owner_id, title, width, height) VALUES ($1,$2,$3,$4,100,100)', [id, ws, userA, 'Collab test']);
  return id;
}

/** What a fresh load would see: snapshot + op-log tail, decoded with yjs. */
async function persisted(design: string): Promise<Y.Doc> {
  const doc = new Y.Doc();
  const s = await db.query('SELECT state, seq FROM doc_state WHERE design_id = $1', [design]);
  if (s.rows[0]) Y.applyUpdate(doc, new Uint8Array(s.rows[0].state));
  const tail = await db.query('SELECT update FROM doc_updates WHERE design_id = $1 AND seq > $2 ORDER BY seq', [design, s.rows[0]?.seq ?? 0]);
  for (const r of tail.rows) Y.applyUpdate(doc, new Uint8Array(r.update));
  return doc;
}

// ---------------------------------------------------------------- clients

type Role = 'owner' | 'edit' | 'comment' | 'view';
function ticket(design: string, sub: string, role: Role, o: { iat?: number; ttl?: number; brandLocked?: boolean; aud?: string } = {}) {
  const now = o.iat ?? Date.now();
  const t = { sub, name: sub.slice(0, 4), color: '#ff0000', aud: o.aud ?? 'collab', design, role, ...(o.brandLocked ? { brandLocked: true } : {}) };
  const body = Buffer.from(JSON.stringify({ ...t, iat: now, exp: Math.floor(now / 1000) + (o.ttl ?? 300) })).toString('base64url');
  return `${body}.${createHmac('sha256', SECRET).update(body).digest('base64url')}`;
}

interface Client {
  doc: Y.Doc;
  p: InstanceType<typeof HocuspocusProvider>;
  closes: string[];
  denied: string[];
  scopes: string[];
  stateless: string[];
  acked: () => boolean;
  destroy: () => void;
}

function client(design: string, o: { sub?: string; role?: Role; token?: string | (() => string); room?: string; brandLocked?: boolean; socket?: InstanceType<typeof HocuspocusProviderWebsocket> } = {}): Client {
  const doc = new Y.Doc();
  const c = { doc, closes: [] as string[], denied: [] as string[], scopes: [] as string[], stateless: [] as string[] } as Client;
  const sub = o.sub ?? userA;
  const base = {
    name: o.room ?? `design:${design}`,
    document: doc,
    // Fetched on every (re)connect, like apps/web.
    token: o.token ?? (() => ticket(design, sub, o.role ?? 'edit', { brandLocked: o.brandLocked })),
    onClose: ({ event }: { event: { reason?: string } }) => c.closes.push(String(event?.reason ?? '')),
    onAuthenticationFailed: ({ reason }: { reason: string }) => c.denied.push(reason),
    onAuthenticated: ({ scope }: { scope: string }) => c.scopes.push(scope),
    onStateless: ({ payload }: { payload: string }) => c.stateless.push(payload),
  };
  c.p = o.socket ? new HocuspocusProvider({ ...base, websocketProvider: o.socket }) : new HocuspocusProvider({ ...base, url: WS_URL });
  if (o.socket) c.p.attach();
  c.acked = () => c.p.isSynced && c.p.unsyncedChanges === 0;
  c.destroy = () => c.p.destroy();
  return c;
}
/** Drop the socket and connect again (the provider ignores connect() until the old socket has closed). */
async function reconnect(c: Client) {
  c.p.disconnect();
  await waitFor(() => c.p.configuration.websocketProvider.status === 'disconnected', 'socket closed');
  await c.p.connect();
}
const synced = (c: Client, what = 'sync') => waitFor(() => c.p.isSynced, what);
const acked = (c: Client, what = 'ack') => waitFor(() => c.acked(), what);
const text = (c: Client | Y.Doc) => ('doc' in c ? c.doc : c).getText('body').toString();
/** Compare the root types the scenarios use (Y.Doc#toJSON only lists roots that were instantiated locally). */
const view = (d: Y.Doc) => JSON.stringify({ body: d.getText('body').toString(), items: d.getArray('items').toJSON(), meta: d.getMap('meta').toJSON(), pages: d.getMap('pages').toJSON(), elements: d.getMap('elements').toJSON() });
const same = (a: Y.Doc, b: Y.Doc) => view(a) === view(b);

/** Send raw bytes on a provider's socket (messages the provider API does not expose). */
function raw(c: Client, parts: (number | string)[]) {
  const out: number[] = [];
  const varUint = (n: number) => {
    while (n > 0x7f) {
      out.push(0x80 | (n & 0x7f));
      n = Math.floor(n / 128);
    }
    out.push(n);
  };
  for (const p of parts) {
    if (typeof p === 'number') varUint(p);
    else {
      const b = Buffer.from(p, 'utf8');
      varUint(b.length);
      out.push(...b);
    }
  }
  c.p.configuration.websocketProvider.send(new Uint8Array(out));
}

// ---------------------------------------------------------------- scenarios

const results: { name: string; ok: boolean; error?: string; ms: number }[] = [];
async function scenario(name: string, fn: () => Promise<void>) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true, ms: Date.now() - t0 });
    console.log(`PASS ${name} (${Date.now() - t0} ms)`);
  } catch (e) {
    results.push({ name, ok: false, error: (e as Error).message, ms: Date.now() - t0 });
    console.log(`FAIL ${name}: ${(e as Error).stack ?? e}`);
  }
}

async function main() {
  await db.connect();
  await seed();
  await startServer();

  await scenario('http banner (health)', async () => {
    const r = await fetch(`${HTTP_URL}health`);
    assert(r.status === 200, `status ${r.status}`);
    assert((await r.text()) === 'Welcome to Hocuspocus!', 'banner');
  });

  await scenario('auth rejection', async () => {
    const d = await newDesign();
    const other = await newDesign();
    const bad = client(d, { token: 'garbage.token' });
    const wrong = client(d, { token: ticket(other, userA, 'edit') });
    const expired = client(d, { token: ticket(d, userA, 'edit', { iat: Date.now() - 3_600_000 }) });
    const render = client(d, { token: ticket(d, userA, 'edit', { aud: 'render' }) });
    const room = client(d, { room: 'design:not-a-valid-room' });
    const all = [bad, wrong, expired, render, room];
    await waitFor(() => all.every((c) => c.denied.length > 0), 'permission denied');
    for (const c of all) assert(c.denied[0] === 'permission-denied', `reason ${c.denied[0]}`);
    for (const c of all) assert(!c.p.isSynced && c.scopes.length === 0, 'not authenticated');
    // Nothing reaches the document.
    bad.doc.getText('body').insert(0, 'nope');
    await sleep(300);
    const rows = await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d]);
    assert(rows.rows[0].n === 0, 'no op-log rows');
    all.forEach((c) => c.destroy());
  });

  await scenario('two clients converge (concurrent edits, awareness, multiplexed socket)', async () => {
    const d = await newDesign();
    const d2 = await newDesign();
    const a = client(d, { sub: userA });
    // B shares one socket between two documents, like a provider websocket reused across rooms.
    const sock = new HocuspocusProviderWebsocket({ url: WS_URL });
    const b = client(d, { sub: userB, socket: sock });
    const b2 = client(d2, { sub: userB, socket: sock });
    await Promise.all([synced(a), synced(b), synced(b2)]);
    assert(a.scopes[0] === 'read-write', `scope ${a.scopes[0]}`);
    a.p.setAwarenessField('user', { id: userA, name: 'Ann' });
    const ta = a.doc.getArray<string>('items');
    const tb = b.doc.getArray<string>('items');
    for (let i = 0; i < 40; i++) {
      ta.insert(Math.floor(Math.random() * (ta.length + 1)), [`a${i}`]);
      tb.insert(Math.floor(Math.random() * (tb.length + 1)), [`b${i}`]);
      if (i % 7 === 0) {
        a.doc.getMap('meta').set('title', `A${i}`);
        b.doc.getMap('meta').set('title', `B${i}`);
        a.doc.getText('body').insert(0, `a${i} `);
        b.doc.getText('body').insert(0, `b${i} `);
      }
      if (i % 10 === 0) await sleep(5);
    }
    b2.doc.getText('body').insert(0, 'second document');
    await Promise.all([acked(a), acked(b), acked(b2)]);
    await waitFor(() => same(a.doc, b.doc), 'convergence');
    const items = new Set(ta.toArray());
    assert(ta.length === 80 && tb.length === 80 && items.size === 80, `all inserts present (${ta.length})`);
    for (let i = 0; i < 40; i++) assert(items.has(`a${i}`) && items.has(`b${i}`), `insert ${i}`);
    assert(text(a) === text(b) && text(a).length === 44, `text converged (${text(a).length})`);
    // Awareness relayed to the other client.
    await waitFor(() => [...b.p.awareness!.getStates().values()].some((s) => (s as { user?: { name?: string } }).user?.name === 'Ann'), 'awareness');
    // A third client loads the same state; the multiplexed document stayed separate.
    const c = client(d, { sub: userB });
    await synced(c);
    await waitFor(() => same(c.doc, a.doc), 'late joiner state');
    assert(!text(c).includes('second document'), 'documents are isolated');
    const c2 = client(d2, { sub: userA });
    await synced(c2);
    await waitFor(() => text(c2) === 'second document', 'second document synced');
    // Leaving removes the awareness state on the others.
    a.destroy();
    await waitFor(() => ![...b.p.awareness!.getStates().values()].some((s) => (s as { user?: { name?: string } }).user?.name === 'Ann'), 'awareness removal');
    // Everything acknowledged is on disk.
    assert(same(await persisted(d), b.doc), 'persisted state matches');
    [b, b2, c, c2].forEach((x) => x.destroy());
    sock.destroy();
  });

  await scenario('read-only roles cannot write', async () => {
    const d = await newDesign();
    const ed = client(d, { sub: userA, role: 'owner' });
    await synced(ed);
    ed.doc.getText('body').insert(0, 'from editor');
    await acked(ed);
    for (const role of ['view', 'comment'] as const) {
      const v = client(d, { sub: userV, role });
      await synced(v);
      assert(v.scopes[0] === 'readonly', `scope for ${role}: ${v.scopes[0]}`);
      await waitFor(() => text(v) === 'from editor', 'viewer receives edits');
      v.doc.getText('body').insert(0, `${role} was here `);
      await sleep(800);
      assert(v.p.unsyncedChanges > 0, 'read-only write is never acknowledged');
      assert(text(ed) === 'from editor', 'editor never sees the read-only write');
      const rows = (await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1 AND actor_id = $2', [d, userV])).rows[0].n;
      assert(rows === 0, 'no op-log row for read-only writes');
      assert(!text(await persisted(d)).includes('was here'), 'nothing persisted');
      // Reconnecting (SyncStep2 with the unsent change) is refused too.
      await reconnect(v);
      await synced(v);
      await sleep(500);
      assert(text(ed) === 'from editor', 'read-only resync does not leak');
      v.destroy();
    }
    assert(text(await persisted(d)) === 'from editor', 'persisted state unchanged');
    ed.destroy();
  });

  await scenario('ack only after commit; a failed write is not acknowledged and is resent on reconnect', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    // (1) Ack ⇒ committed: query right after the ack.
    for (let i = 0; i < 5; i++) {
      a.doc.getText('body').insert(0, `x${i}`);
      await acked(a);
      assert(text(await persisted(d)).startsWith(`x${i}`), `edit ${i} on disk at ack time`);
    }
    // (2) The op log refuses writes: no ack, the server closes the document connection.
    await db.query('ALTER TABLE doc_updates ADD CONSTRAINT oc_collab_test_block CHECK (false) NOT VALID');
    try {
      a.doc.getText('body').insert(0, 'LOST? ');
      await waitFor(() => a.closes.includes('Reset Connection'), 'CLOSE "Reset Connection"');
      await sleep(500);
      assert(a.p.unsyncedChanges > 0, 'no ack for the failed write');
      assert(!text(b).includes('LOST?'), 'an unpersisted update is never applied or broadcast');
    } finally {
      await db.query('ALTER TABLE doc_updates DROP CONSTRAINT oc_collab_test_block');
    }
    // (3) Reconnect: the client still holds the update and resends it.
    await reconnect(a);
    await synced(a);
    await acked(a);
    assert(text(await persisted(d)).startsWith('LOST? '), 'resent update persisted');
    await waitFor(() => text(b).startsWith('LOST? '), 'resent update broadcast');
    a.destroy();
    b.destroy();
  });

  await scenario('malformed updates are refused before they reach the op log', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    a.doc.getText('body').insert(0, 'good ');
    await acked(a);
    const before = (await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d])).rows[0].n;
    // Sync / Update carrying bytes that are not a Yjs update (one state update announcing five structs, then nothing).
    raw(a, [`design:${d}`, 0, 2, '\x01\x05']);
    await waitFor(() => a.closes.includes('Reset Connection'), 'CLOSE "Reset Connection"');
    await sleep(300);
    const after = (await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d])).rows[0].n;
    assert(after === before, `no op-log row for the malformed update (${before} → ${after})`);
    assert(b.closes.length === 0, 'other connections unaffected');
    b.doc.getText('body').insert(0, 'still fine ');
    await acked(b);
    await reconnect(a);
    await synced(a);
    await waitFor(() => text(a) === 'still fine good ', `resynced after the reset (${text(a)})`);
    a.destroy();
    b.destroy();
  });

  await scenario('debounced store: 1.5 s after the last change, at most every 8 s while editing; versions, thumbnails', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    await synced(a);
    const snapshot = async () => (await db.query('SELECT state FROM doc_state WHERE design_id = $1', [d])).rows[0]?.state as Buffer | undefined;
    a.doc.transact(() => {
      a.doc.getMap('meta').set('dims', { w: 640, h: 480, unit: 'mm' });
      a.doc.getText('body').insert(0, 'first');
    });
    await acked(a);
    const t0 = Date.now();
    assert(!(await snapshot()), 'no store right after the edit (debounced)');
    await waitFor(async () => Boolean(await snapshot()), 'debounced store', 6000);
    const waited = Date.now() - t0;
    assert(waited >= 1000, `store waited for the debounce (${waited} ms)`);
    const s1 = await snapshot();
    const y1 = new Y.Doc();
    Y.applyUpdate(y1, new Uint8Array(s1!));
    assert(text(y1) === 'first', 'snapshot holds the edit');
    // The store's statements are separate (snapshot, truncate the log, designs row): wait for the last of them.
    const row = async () => (await db.query('SELECT width, height, unit, page_count FROM designs WHERE id = $1', [d])).rows[0];
    await waitFor(async () => (await row()).unit === 'mm', 'designs row mirrored', 3000).catch(async (e) => {
      throw new Error(`${e.message}: ${JSON.stringify(await row())}`);
    });
    const r = await row();
    assert(r.width === 640 && r.height === 480 && r.page_count === 0, `designs row ${JSON.stringify(r)}`);
    const log = (await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d])).rows[0].n;
    assert(log === 0, `op log folded into the snapshot (${log} left)`);
    await waitFor(() => thumbnails(d).length >= 1, 'thumbnail refresh', 3000);
    assert(thumbnails(d).every((c) => c.internal), 'thumbnail refresh carries the internal token');
    // Continuous editing (every 250 ms for 9.5 s) never lets the 1.5 s debounce fire; maxDebounce (8 s) forces a store.
    let storedWhileEditing = false;
    const end = Date.now() + 9500;
    for (let i = 0; Date.now() < end; i++) {
      a.doc.getText('body').insert(0, String(i % 10));
      await sleep(250);
      if (!storedWhileEditing) {
        const s = await snapshot();
        storedWhileEditing = Boolean(s && Buffer.compare(s, s1!) !== 0);
      }
    }
    assert(storedWhileEditing, 'maxDebounce stored during continuous editing');
    await acked(a);
    await waitFor(async () => {
      const s = await snapshot();
      if (!s) return false;
      const y = new Y.Doc();
      Y.applyUpdate(y, new Uint8Array(s));
      return text(y) === text(a);
    }, 'final store after the last edit', 6000);
    const v = (await db.query('SELECT count(*)::int AS n FROM design_versions WHERE design_id = $1', [d])).rows[0].n;
    assert(v === 1, `one automatic version per 10 minutes (${v})`);
    // The refresh is fire-and-forget after each store: wait for the third one (first, maxDebounce, final store).
    await waitFor(() => thumbnails(d).length >= 3, `a thumbnail refresh after every store (${thumbnails(d).length})`, 3000);
    assert(thumbnails(d).every((c) => c.internal), 'every thumbnail refresh carries the internal token');
    a.destroy();
  });

  await scenario('the last client leaving flushes the pending store immediately', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    b.doc.getText('body').insert(0, 'flush me');
    await acked(b);
    b.destroy();
    await sleep(100);
    assert((await db.query('SELECT 1 FROM doc_state WHERE design_id = $1', [d])).rows.length === 0, 'still debounced while a client remains');
    const t0 = Date.now();
    a.destroy();
    await waitFor(async () => (await db.query('SELECT 1 FROM doc_state WHERE design_id = $1', [d])).rows.length === 1, 'store on last disconnect', 1200);
    assert(Date.now() - t0 < 1200, 'flushed before the debounce would have fired');
    const c = client(d, { sub: userA });
    await synced(c);
    await waitFor(() => text(c) === 'flush me', 'reloaded');
    c.destroy();
  });

  await scenario('load repairs a damaged document once (CO-004)', async () => {
    const d = await newDesign();
    const src = new Y.Doc();
    src.transact(() => {
      src.getMap('meta').set('dims', { w: 100, h: 100, unit: 'px' });
      const page = new Y.Map();
      page.set('id', 'page000000000001');
      page.set('order', 'a0');
      src.getMap('pages').set('page000000000001', page);
      const el = (id: string, parentId: string, xf: Record<string, unknown>) => {
        const m = new Y.Map();
        m.set('id', id);
        m.set('type', 'shape');
        m.set('parentId', parentId);
        m.set('z', 'a0');
        m.set('xf', xf);
        src.getMap('elements').set(id, m);
      };
      el('elemok0000000001', 'page000000000001', { x: 1, y: 2, w: 3, h: 4, rot: 0 });
      el('elemorphan000001', 'elemmissing00001', { x: 1, y: 2, w: 3, h: 4, rot: 0 });
      el('elembadxf0000001', 'page000000000001', { x: NaN, y: 5, w: Infinity, h: -0, rot: 0 });
      el('elemcyclea000001', 'elemcycleb000001', { x: 0, y: 0, w: 1, h: 1, rot: 0 });
      el('elemcycleb000001', 'elemcyclea000001', { x: 0, y: 0, w: 1, h: 1, rot: 0 });
    });
    await db.query('INSERT INTO doc_state(design_id, state) VALUES ($1, $2)', [d, Buffer.from(Y.encodeStateAsUpdate(src))]);
    const a = client(d, { sub: userA });
    await synced(a);
    const els = a.doc.getMap('elements');
    const get = (id: string, k: string) => (els.get(id) as Y.Map<unknown>).get(k) as Record<string, unknown> | string | undefined;
    await waitFor(() => els.size === 5, 'elements synced');
    assert(get('elemok0000000001', 'deleted') === undefined, 'healthy element untouched');
    assert((get('elemorphan000001', 'deleted') as { by?: string })?.by === 'repair', `orphan deleted by repair: ${JSON.stringify(get('elemorphan000001', 'deleted'))}`);
    const xf = get('elembadxf0000001', 'xf') as Record<string, number>;
    assert(xf.x === 0 && xf.y === 5 && xf.w === 100 && xf.h === 1 && xf.rot === 0, `transform repaired: ${JSON.stringify(xf)}`);
    const cyc = [get('elemcyclea000001', 'parentId'), get('elemcycleb000001', 'parentId')];
    assert(cyc.includes('page000000000001'), `cycle broken onto the first page: ${JSON.stringify(cyc)}`);
    const rows = (await db.query('SELECT actor_id FROM doc_updates WHERE design_id = $1', [d])).rows;
    assert(rows.length === 1 && rows[0].actor_id === null, `the repair is persisted through the op log once (${JSON.stringify(rows)})`);
    // Nothing changed after the load, so the last client leaving unloads without a store; the next load replays the
    // logged repair and finds nothing left to repair.
    a.destroy();
    await sleep(400);
    const b = client(d, { sub: userB });
    await synced(b);
    await waitFor(() => b.doc.getMap('elements').size === 5, 'reloaded');
    await sleep(200);
    const again = (await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d])).rows[0].n;
    assert(again === 1, `no second repair (${again} op-log rows)`);
    assert(((b.doc.getMap('elements').get('elemorphan000001') as Y.Map<unknown>).get('deleted') as { by?: string })?.by === 'repair', 'repair survives the reload');
    b.destroy();
  });

  await scenario('offline edits resync on reconnect', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    a.doc.getText('body').insert(0, 'base ');
    await acked(a);
    await waitFor(() => text(b) === 'base ', 'base');
    a.p.disconnect();
    await waitFor(() => a.p.configuration.websocketProvider.status === 'disconnected', 'offline');
    a.doc.getText('body').insert(5, 'offline-a ');
    a.doc.getMap('meta').set('k', 'from a');
    b.doc.getText('body').insert(0, 'online-b ');
    await acked(b);
    await a.p.connect();
    await synced(a);
    await acked(a);
    await waitFor(() => same(a.doc, b.doc), 'convergence after reconnect', 10_000).catch((e) => {
      throw new Error(`${e.message}: a=${view(a.doc)} b=${view(b.doc)}`);
    });
    assert(text(a).includes('offline-a') && text(a).includes('online-b'), 'both edits');
    assert(same(await persisted(d), a.doc), 'persisted');
    a.destroy();
    b.destroy();
  });

  await scenario('persistence survives SIGKILL (op-log replay) and SIGTERM (compacted snapshot)', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    await synced(a);
    const meta = a.doc.getMap('meta');
    a.doc.transact(() => {
      meta.set('dims', { w: 1080, h: 1920, unit: 'px' });
      const page = new Y.Map();
      page.set('id', 'page000000000001');
      page.set('order', 'a0');
      a.doc.getMap('pages').set('page000000000001', page);
      a.doc.getText('body').insert(0, 'acked before kill');
    });
    await acked(a);
    // Hard kill right after the ack, while the client is still connected: no store ran (debounce), so the
    // op log alone must carry the edit.
    await stopServer('SIGKILL');
    a.destroy();
    const snap0 = await db.query('SELECT 1 FROM doc_state WHERE design_id = $1', [d]);
    assert(snap0.rows.length === 0, 'no snapshot was written before the kill');
    await startServer();
    const b = client(d, { sub: userB });
    await synced(b);
    await waitFor(() => text(b) === 'acked before kill', 'edit survives SIGKILL');
    b.doc.getText('body').insert(0, 'then SIGTERM: ');
    await acked(b);
    // Graceful stop flushes the pending store (Hocuspocus destroy()).
    await stopServer('SIGTERM');
    b.destroy();
    const st = await db.query('SELECT state FROM doc_state WHERE design_id = $1', [d]);
    assert(st.rows.length === 1, 'doc_state written on shutdown');
    const snap = new Y.Doc();
    Y.applyUpdate(snap, new Uint8Array(st.rows[0].state));
    assert(text(snap) === 'then SIGTERM: acked before kill', `snapshot holds every edit: ${text(snap)}`);
    const log = await db.query('SELECT count(*)::int AS n FROM doc_updates WHERE design_id = $1', [d]);
    assert(log.rows[0].n === 0, `op log folded into the snapshot (${log.rows[0].n} left)`);
    const row = await db.query('SELECT width, height, unit, page_count FROM designs WHERE id = $1', [d]);
    assert(row.rows[0].width === 1080 && row.rows[0].height === 1920 && row.rows[0].unit === 'px' && row.rows[0].page_count === 1, `designs row ${JSON.stringify(row.rows[0])}`);
    const v = await db.query('SELECT count(*)::int AS n, bool_and(created_by IS NOT NULL) AS by FROM design_versions WHERE design_id = $1', [d]);
    assert(v.rows[0].n >= 1 && v.rows[0].by, 'automatic version with author');
    await startServer();
    const c = client(d, { sub: userA });
    await synced(c);
    await waitFor(() => text(c) === 'then SIGTERM: acked before kill', 'reload after restart');
    c.destroy();
  });

  await scenario('live revocation (oc_revoke) closes sockets and refuses older tickets', async () => {
    const d = await newDesign();
    const issued = Date.now() - 1000;
    const stale = ticket(d, userB, 'edit', { iat: issued });
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    await db.query('SELECT pg_notify($1, $2)', ['oc_revoke', JSON.stringify({ designId: d, userId: userB, at: Date.now() })]);
    await waitFor(() => b.closes.includes('access changed'), 'CLOSE "access changed"');
    await sleep(200);
    assert(!a.closes.includes('access changed'), 'other users stay connected');
    const old = client(d, { token: stale });
    await waitFor(() => old.denied.length > 0, 'stale ticket refused');
    // A ticket fetched after the revocation passes.
    await sleep(5);
    const fresh = client(d, { sub: userB });
    await synced(fresh, 'fresh ticket accepted');
    [a, b, old, fresh].forEach((x) => x.destroy());
  });

  await scenario('server-side update (oc_doc_update) reaches clients and the op log', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    await synced(a);
    const srv = new Y.Doc();
    Y.applyUpdate(srv, Y.encodeStateAsUpdate(a.doc));
    srv.getMap('meta').set('brand', 'updated by the API');
    const update = Y.encodeStateAsUpdate(srv, Y.encodeStateVector(a.doc));
    await db.query('SELECT pg_notify($1, $2)', ['oc_doc_update', JSON.stringify({ designId: d, seq: '1', update: Buffer.from(update).toString('base64') })]);
    await waitFor(() => a.doc.getMap('meta').get('brand') === 'updated by the API', 'merged update');
    await waitFor(async () => (await persisted(d)).getMap('meta').get('brand') === 'updated by the API', 'logged by onChange');
    a.destroy();
  });

  await scenario('BroadcastStateless reaches every connection', async () => {
    const d = await newDesign();
    const a = client(d, { sub: userA });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(b)]);
    raw(a, [`design:${d}`, 6, 'ping-all']);
    await waitFor(() => a.stateless.includes('ping-all') && b.stateless.includes('ping-all'), 'stateless broadcast');
    // Plain Stateless has no server hook: nothing comes back.
    a.p.sendStateless('just me');
    await sleep(300);
    assert(!b.stateless.includes('just me') && !a.stateless.includes('just me'), 'stateless not echoed');
    a.destroy();
    b.destroy();
  });

  await scenario('messages without a server hook are ignored; a provider CLOSE ends only that document', async () => {
    const d = await newDesign();
    const d2 = await newDesign();
    const sock = new HocuspocusProviderWebsocket({ url: WS_URL });
    const a = client(d, { sub: userA, socket: sock });
    const a2 = client(d2, { sub: userA, socket: sock });
    const b = client(d, { sub: userB });
    await Promise.all([synced(a), synced(a2), synced(b)]);
    raw(a, [`design:${d}`, 3]); // QueryAwareness: Hocuspocus answers only through a reply callback, never on a connection
    raw(a, [`design:${d}`, 2, 0, 'a-new-token']); // token sync on an established connection: no onTokenSync hook
    raw(a, [`design:${d}`, 42]); // unknown message type: logged, not fatal
    a.doc.getText('body').insert(0, 'still working');
    await acked(a);
    await waitFor(() => text(b) === 'still working', 'edits flow after ignored messages');
    assert(a.closes.length === 0 && b.closes.length === 0, `no closes (${a.closes} / ${b.closes})`);
    // CLOSE from the provider: the server closes that document connection and says so.
    raw(a, [`design:${d}`, 7]);
    await waitFor(() => a.closes.includes('provider_initiated'), 'CLOSE "provider_initiated"');
    await sleep(200);
    assert(b.closes.length === 0, 'other clients stay connected');
    a2.doc.getText('body').insert(0, 'same socket, other document');
    await acked(a2, 'the other document on the same socket still syncs');
    [a, a2, b].forEach((x) => x.destroy());
    sock.destroy();
  });

  await scenario('brand-locked editors cannot change locked elements', async () => {
    const d = await newDesign();
    const owner = client(d, { sub: userA, role: 'owner' });
    await synced(owner);
    owner.doc.transact(() => {
      const el = new Y.Map();
      el.set('id', 'elem000000000001');
      el.set('type', 'shape');
      el.set('parentId', 'page000000000001');
      el.set('z', 'a0');
      el.set('xf', { x: 10, y: 10, w: 100, h: 100, rot: 0 });
      el.set('lock', { by: 'brand', move: true, delete: true });
      owner.doc.getMap('elements').set('elem000000000001', el);
    });
    await acked(owner);
    const m = client(d, { sub: userB, role: 'edit', brandLocked: true });
    await synced(m);
    await waitFor(() => m.doc.getMap('elements').has('elem000000000001'), 'locked element synced');
    // Allowed: unrelated changes.
    m.doc.getMap('meta').set('note', 'fine');
    await acked(m);
    (m.doc.getMap('elements').get('elem000000000001') as Y.Map<unknown>).set('xf', { x: 999, y: 10, w: 100, h: 100, rot: 0 });
    await waitFor(() => m.closes.includes('brand lock'), 'CLOSE "brand lock"');
    await sleep(300);
    const p = await persisted(d);
    assert(((p.getMap('elements').get('elem000000000001') as Y.Map<unknown>).get('xf') as { x: number }).x === 10, 'violating update not persisted');
    assert(p.getMap('meta').get('note') === 'fine', 'allowed change persisted');
    assert(((owner.doc.getMap('elements').get('elem000000000001') as Y.Map<unknown>).get('xf') as { x: number }).x === 10, 'owner unaffected');
    owner.destroy();
    m.destroy();
  });

  await stopServer('SIGTERM');
  await db.end();
  render.close();
  const failed = results.filter((r) => !r.ok);
  console.log(JSON.stringify({ passed: results.length - failed.length, failed: failed.length, results }));
  process.exit(failed.length ? 1 : 0);
}

main().catch(async (e) => {
  console.error(e);
  await stopServer('SIGKILL');
  render.close();
  process.exit(2);
});
