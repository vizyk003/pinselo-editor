#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
/**
 * Build @oc/engine: crates/oc-wasm → wasm32 (size-optimised) → wasm-bindgen --target web → wasm-opt → pkg/,
 * then type-check the facade against the generated .d.ts.
 *
 *   pnpm --filter @oc/engine build            release build (default features: panic hook, complex scripts)
 *   pnpm --filter @oc/engine build -- --dev   unoptimised wasm, fast to build (tests still pass)
 *   … --lean        without complex-script line-break dictionaries (Thai/Lao/Khmer/Myanmar/CJK): much smaller
 *   … --no-render   without the oc-render module (display lists, painter)
 *   … --no-hot      opt-level "z" for the renderer crates too (default: 3 for the painter's hot crates)
 *   … --no-simd     without WebAssembly SIMD (128-bit; every current browser has it — the painter's pipelines use it)
 *   … --no-opt      skip wasm-opt;  --no-check  skip the TS type-check;  --opt=s|z|3  rustc opt-level (default z)
 *
 * Toolchain (see README): a cargo with the wasm32-unknown-unknown target and a wasm-bindgen CLI whose version equals
 * the `wasm-bindgen` crate in Cargo.lock. By default the isolated rustup home in ~/.rustup is used when present
 * (RUSTUP_HOME=~/.rustup CARGO_HOME=~/.rustup/cargo); override with OC_WASM_CARGO / OC_WASM_BINDGEN.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, constants as zlib, gzipSync } from 'node:zlib';

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(here, '..');
const repo = join(pkgDir, '../..');
const out = join(pkgDir, 'pkg');
const args = new Set(process.argv.slice(2));
const dev = args.has('--dev');
const lean = args.has('--lean');

const env = { ...process.env };
let cargo = process.env.OC_WASM_CARGO;
const isolated = join(homedir(), '.rustup');
if (!cargo && existsSync(join(isolated, 'cargo/bin/cargo'))) {
  cargo = join(isolated, 'cargo/bin/cargo');
  env.RUSTUP_HOME = isolated;
  env.CARGO_HOME = join(isolated, 'cargo');
}
cargo ??= 'cargo';
env.CARGO_TARGET_DIR ??= join(repo, 'target/engine-wasm');
// Size profile (profiles can only be declared in the workspace root; the services keep their own).
const opt = [...args].find((a) => a.startsWith('--opt='))?.slice(6) ?? 'z';
Object.assign(env, {
  CARGO_PROFILE_RELEASE_OPT_LEVEL: opt,
  CARGO_PROFILE_RELEASE_LTO: 'fat',
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS: '1',
  CARGO_PROFILE_RELEASE_PANIC: 'abort',
  CARGO_PROFILE_RELEASE_DEBUG: 'false',
});

const run = (cmd, argv, opts = {}) => {
  console.log(`$ ${cmd} ${argv.join(' ')}`);
  execFileSync(cmd, argv, { stdio: 'inherit', env, cwd: repo, ...opts });
};

// The CLI must match the crate exactly (the schema of the custom section changes between versions).
const lock = readFileSync(join(repo, 'Cargo.lock'), 'utf8');
const want = /name = "wasm-bindgen"\nversion = "([^"]+)"/.exec(lock)?.[1];
const bindgen = process.env.OC_WASM_BINDGEN ?? (env.CARGO_HOME && existsSync(join(env.CARGO_HOME, 'bin/wasm-bindgen')) ? join(env.CARGO_HOME, 'bin/wasm-bindgen') : 'wasm-bindgen');
let have = '';
try {
  have = execFileSync(bindgen, ['--version'], { env, encoding: 'utf8' }).trim().split(' ').pop() ?? '';
} catch {
  /* reported below */
}
if (!want || have !== want) {
  const how = env.RUSTUP_HOME
    ? `RUSTUP_HOME=${env.RUSTUP_HOME} CARGO_HOME=${env.CARGO_HOME} ${cargo} install wasm-bindgen-cli --version ${want} --locked`
    : `cargo install wasm-bindgen-cli --version ${want} --locked`;
  console.error(`wasm-bindgen CLI ${want} required (found: ${have || 'none'}). Install it with:\n  ${how}`);
  process.exit(1);
}

// A cargo whose rustc has the wasm32 target (checked here so a missing toolchain fails with instructions).
let sysroot = '';
try {
  execFileSync(cargo, ['--version'], { env, stdio: 'ignore' });
  const sibling = join(dirname(cargo), 'rustc');
  const rustc = cargo.includes('/') && existsSync(sibling) ? sibling : 'rustc';
  sysroot = execFileSync(rustc, ['--print', 'sysroot'], { env, encoding: 'utf8', cwd: repo }).trim();
} catch {
  console.error(`cargo not found (tried: ${cargo}). Install Rust (https://rustup.rs) with the wasm32 target:\n  rustup target add wasm32-unknown-unknown\nor point OC_WASM_CARGO at a cargo binary.`);
  process.exit(1);
}
if (sysroot && !existsSync(join(sysroot, 'lib/rustlib/wasm32-unknown-unknown'))) {
  const how = env.RUSTUP_HOME ? `RUSTUP_HOME=${env.RUSTUP_HOME} CARGO_HOME=${env.CARGO_HOME} ${join(env.CARGO_HOME, 'bin/rustup')} target add wasm32-unknown-unknown` : 'rustup target add wasm32-unknown-unknown';
  console.error(`The Rust toolchain at ${sysroot} has no wasm32-unknown-unknown target. Install it with:\n  ${how}`);
  process.exit(1);
}

const noRender = args.has('--no-render');
const simd = !args.has('--no-simd');
// tiny-skia's raster pipelines have a wasm `simd128` path (4–8 lanes per op instead of scalar code).
if (simd) env.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS = [process.env.CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS, '-C target-feature=+simd128'].filter(Boolean).join(' ');
const featureList = ['console_error_panic_hook', ...(lean ? [] : ['complex-scripts']), ...(noRender ? [] : ['render'])];
const features = lean || noRender ? ['--no-default-features', '--features', featureList.join(',')] : [];
// The painter's hot loops (rasterisation, image decoding/resampling, glyph outlines, shaping) are ~2x faster at
// opt-level 3 than at "z"; the rest of the core stays size-optimised. `--opt=…` applies one level to everything.
const HOT = ['tiny-skia', 'tiny-skia-path', 'oc-render', 'oc-text', 'image', 'zune-jpeg', 'zune-core', 'png', 'fdeflate', 'image-webp', 'gif', 'resvg', 'skrifa', 'read-fonts', 'harfrust', 'parley', 'parley_engine'];
const hot = !dev && !args.has('--no-hot') && ![...args].some((a) => a.startsWith('--opt=')) ? HOT.flatMap((c) => ['--config', `profile.release.package.${JSON.stringify(c)}.opt-level=3`]) : [];
run(cargo, ['build', '-p', 'oc-wasm', '--target', 'wasm32-unknown-unknown', ...(dev ? [] : ['--release']), ...features, ...hot]);
const wasm = join(env.CARGO_TARGET_DIR, 'wasm32-unknown-unknown', dev ? 'debug' : 'release', 'oc_wasm.wasm');

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
run(bindgen, ['--target', 'web', '--out-dir', out, '--out-name', 'oc_wasm', ...(dev ? [] : ['--remove-name-section', '--remove-producers-section']), wasm]);

const bg = join(out, 'oc_wasm_bg.wasm');
const beforeOpt = statSync(bg).size;
if (!dev && !args.has('--no-opt')) {
  const wasmOpt = join(pkgDir, 'node_modules/.bin/wasm-opt');
  // The features rustc enables by default for wasm32-unknown-unknown.
  const feats = ['bulk-memory', 'multivalue', 'mutable-globals', 'nontrapping-float-to-int', 'reference-types', 'sign-ext', ...(simd ? ['simd'] : [])].map((f) => `--enable-${f}`);
  run(wasmOpt, [opt === '3' ? '-O3' : opt === 's' ? '-Os' : '-Oz', ...feats, bg, '-o', bg]);
}

const raw = statSync(bg).size;
const bytes = readFileSync(bg);
const gz = gzipSync(bytes, { level: 9 }).length;
const br = brotliCompressSync(bytes, { params: { [zlib.BROTLI_PARAM_QUALITY]: 11, [zlib.BROTLI_PARAM_SIZE_HINT]: raw } }).length;
const info = { profile: dev ? 'dev' : `release (opt-level ${opt}${hot.length ? ', 3 for the renderer crates' : ''})`, features: featureList, simd, wasmBindgen: want, bytes: raw, gzipBytes: gz, brotliBytes: br, beforeWasmOpt: beforeOpt };
writeFileSync(join(out, 'build-info.json'), `${JSON.stringify(info, null, 2)}\n`);
const mb = (n) => `${(n / 1024 / 1024).toFixed(2)} MB`;
console.log(`oc_wasm_bg.wasm: ${mb(raw)} raw, ${mb(gz)} gzip -9, ${mb(br)} brotli -11${raw !== beforeOpt ? ` (before wasm-opt ${mb(beforeOpt)})` : ''}`);

if (!args.has('--no-check')) run(join(pkgDir, 'node_modules/.bin/tsc'), ['-p', join(pkgDir, 'tsconfig.json')], { cwd: pkgDir });
