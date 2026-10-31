/**
 * The Rust binaries the dev tooling runs (`pnpm dev`, `pnpm db:migrate`, `pnpm seed`, `pnpm tpl:preview`), built in
 * release into target/dev (`OC_DEV_TARGET_DIR` overrides). Every caller builds the same package set, so they share
 * one feature set and one set of artifacts: the first build takes a few minutes, later ones only relink what changed.
 *
 *   tsx tools/scripts/rust.ts <binary> [args…]   build, then run one binary (e.g. `oc-migrate`)
 */
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const ROOT = resolve(import.meta.dirname, '../..');
export const TARGET = resolve(ROOT, process.env.OC_DEV_TARGET_DIR || 'target/dev');
/** Crates whose binaries the tooling runs: oc-api, oc-collab, oc-worker, oc-render-svc (+ oc-templates), oc-migrate. */
const PACKAGES = ['oc-api', 'oc-collab', 'oc-worker', 'oc-render-svc', 'oc-db'].flatMap((p) => ['-p', p]);

export const binPath = (name: string): string => join(TARGET, 'release', name);

/** `cargo build --release` of the tooling's crates; false when the build failed. */
export function buildRust(tag = '[rust]'): boolean {
  console.log(`${tag} cargo build --release ${PACKAGES.join(' ')} (target ${TARGET})`);
  const r = spawnSync('cargo', ['build', '--release', ...PACKAGES], { cwd: ROOT, stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: TARGET } });
  return r.status === 0;
}

/** Build (unless `OC_DEV_SKIP_BUILD=1`), then run `name` to completion with inherited stdio; returns its exit code. */
export function runRust(name: string, args: string[]): number {
  if (process.env.OC_DEV_SKIP_BUILD !== '1' && !buildRust(`[${name}]`) && !existsSync(binPath(name))) return 1;
  if (!existsSync(binPath(name))) {
    console.error(`[${name}] ${binPath(name)} is missing`);
    return 1;
  }
  const r = spawnSync(binPath(name), args, { cwd: ROOT, stdio: 'inherit', env: { ...process.env, RUST_BACKTRACE: process.env.RUST_BACKTRACE ?? '1' } });
  return r.status ?? 1;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [name, ...args] = process.argv.slice(2);
  if (!name) {
    console.error('usage: tsx tools/scripts/rust.ts <binary> [args…]');
    process.exit(2);
  }
  process.exit(runRust(name, args));
}
