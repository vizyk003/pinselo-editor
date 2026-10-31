/**
 * `pnpm dev`: the web app (Vite, apps/web) and the four Rust services — oc-api, oc-collab, oc-worker and
 * oc-render-svc — built in release by tools/scripts/rust.ts (target/dev, `OC_DEV_TARGET_DIR` overrides) and run with
 * the repository environment (`OC_*`, ports included). Output is prefixed per process; stopping one stops all.
 *
 *   OC_DEV_WATCH=1 pnpm dev            rebuild and restart the services when crates/ change
 *   OC_DEV_SERVICES=api,collab pnpm dev   only those services (plus the web app)
 *   OC_DEV_NO_WEB=1 pnpm dev           the services only
 *
 * Run `pnpm db:migrate` first on a new database (`pnpm setup` does).
 */
import { type ChildProcess, spawn } from 'node:child_process';
import { existsSync, watch } from 'node:fs';
import { join } from 'node:path';
import { ROOT, binPath, buildRust } from './rust';

const SERVICES = { api: 'oc-api', collab: 'oc-collab', worker: 'oc-worker', render: 'oc-render-svc' } as const;
type Service = keyof typeof SERVICES;

const wanted = (process.env.OC_DEV_SERVICES ?? '').split(',').map((s) => s.trim()).filter(Boolean);
const services = (Object.keys(SERVICES) as Service[]).filter((s) => !wanted.length || wanted.includes(s));
const children = new Map<string, ChildProcess>();
let stopping = false;

/** Spawn with every output line prefixed by `[name]`. */
function start(name: string, cmd: string, args: string[], cwd = ROOT) {
  const c = spawn(cmd, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? '1', FORCE_COLOR: '1' } });
  for (const [from, to] of [[c.stdout, process.stdout], [c.stderr, process.stderr]] as const) {
    let rest = '';
    from.setEncoding('utf8');
    from.on('data', (d: string) => {
      const lines = (rest + d).split('\n');
      rest = lines.pop() ?? '';
      // The Rust services tag their own lines (`[api] listening …`).
      for (const l of lines) to.write(l.startsWith(`[${name}]`) ? `${l}\n` : `[${name}] ${l}\n`);
    });
  }
  children.set(name, c);
  c.on('exit', (code, signal) => {
    if (children.get(name) !== c) return; // replaced by a restart
    children.delete(name);
    if (!stopping) {
      console.error(`[${name}] exited (${signal ?? code}); stopping the others`);
      stop(code ?? 1);
    }
  });
}

function stop(code = 0) {
  stopping = true;
  for (const c of children.values()) c.kill('SIGTERM');
  setTimeout(() => process.exit(code), 3000).unref();
  if (!children.size) process.exit(code);
}

for (const sig of ['SIGINT', 'SIGTERM'] as const) process.on(sig, () => stop(0));

const startServices = () => {
  for (const s of services) start(s, binPath(SERVICES[s]), []);
};

if (services.length) {
  const ok = buildRust('[dev]');
  const missing = services.filter((s) => !existsSync(binPath(SERVICES[s])));
  if (missing.length) {
    console.error(`[dev] the Rust build failed and there is no previous binary for ${missing.join(', ')}`);
    process.exit(1);
  }
  if (!ok) console.warn('[dev] the Rust build failed — running the previous binaries');
  startServices();
}
if (process.env.OC_DEV_NO_WEB !== '1') start('web', 'pnpm', ['--filter', '@oc/web', 'dev']);

if (process.env.OC_DEV_WATCH === '1' && services.length) {
  let timer: NodeJS.Timeout | null = null;
  watch(join(ROOT, 'crates'), { recursive: true }, (_e, file) => {
    if (!file || !/\.(rs|toml)$/.test(String(file)) || String(file).includes('target')) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      if (!buildRust('[dev]')) return console.warn('[dev] build failed — keeping the running services');
      for (const s of services) {
        const old = children.get(s);
        children.delete(s);
        if (old) old.once('exit', () => start(s, binPath(SERVICES[s]), []));
        old?.kill('SIGTERM');
      }
    }, 800);
  });
}
