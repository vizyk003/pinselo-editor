#!/usr/bin/env node
/**
 * Make sure @oc/engine (the Rust core compiled to WebAssembly, packages/engine/pkg) is built before the web app is
 * type-checked, served or bundled.
 *
 *   node scripts/engine.mjs           build the engine when pkg/ is missing or older than the Rust sources
 *   node scripts/engine.mjs --check   only check: fail with instructions when pkg/ is missing (typecheck)
 *
 * OC_ENGINE_SKIP=1 skips the freshness check (a prebuilt or cached pkg/ is used as it is).
 * The engine build itself (packages/engine/scripts/build.mjs) checks the wasm toolchain and prints how to install it.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '../../..');
const pkg = join(repo, 'packages/engine/pkg');
const stamp = join(pkg, 'build-info.json');
const built = existsSync(stamp) && existsSync(join(pkg, 'oc_wasm_bg.wasm')) && existsSync(join(pkg, 'oc_wasm.js'));
const check = process.argv.includes('--check');

const help = `@oc/web needs @oc/engine, the Rust core compiled to WebAssembly (packages/engine/pkg).
Build it with:  pnpm --filter @oc/engine build
That needs a cargo with the wasm32-unknown-unknown target and the wasm-bindgen CLI matching Cargo.lock
(see packages/engine/README.md, "Build and test").`;

if (check || process.env.OC_ENGINE_SKIP === '1') {
  if (!built) {
    console.error(`\n${help}\n`);
    process.exit(1);
  }
  process.exit(0);
}

/** Newest mtime under the inputs of the wasm build (crate sources and manifests, the lockfile). */
function newest(dir, now = 0) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name === 'target' || e.name.startsWith('.')) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) now = newest(p, now);
    else if (/\.(rs|toml)$/.test(e.name)) now = Math.max(now, statSync(p).mtimeMs);
  }
  return now;
}

if (built) {
  const inputs = Math.max(newest(join(repo, 'crates')), statSync(join(repo, 'Cargo.lock')).mtimeMs, statSync(join(repo, 'Cargo.toml')).mtimeMs);
  if (statSync(stamp).mtimeMs >= inputs) process.exit(0);
  console.log('@oc/engine: Rust sources changed since the last engine build, rebuilding');
} else {
  console.log('@oc/engine: not built yet, building the WebAssembly core first');
}

const r = spawnSync('pnpm', ['--filter', '@oc/engine', 'build'], { cwd: repo, stdio: 'inherit' });
if (r.error || r.status !== 0) {
  console.error(`\n@oc/engine build failed${r.error ? ` (${r.error.message})` : ''}.\n${help}\n`);
  process.exit(1);
}
