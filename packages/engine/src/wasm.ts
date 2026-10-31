// SPDX-License-Identifier: Apache-2.0
/**
 * Loading the WebAssembly module (crates/oc-wasm, built into ../pkg by `pnpm --filter @oc/engine build`).
 *
 * Browser/Vite: the .wasm is referenced with `new URL(…, import.meta.url)`, which Vite turns into an emitted
 * asset (dev and build) and the browser streams and compiles. Node (vitest, scripts): the file is read with
 * `node:fs` through `process.getBuiltinModule`, so no Node import reaches browser bundles.
 */
import initWasm from '../pkg/oc_wasm.js';

export * as raw from '../pkg/oc_wasm.js';

/** What `initEngine` accepts: wasm bytes, a URL/Response to fetch, or a compiled module. */
export type EngineInput = BufferSource | URL | string | Response | WebAssembly.Module;

let ready: Promise<void> | null = null;
let initialized = false;
const hooks: Array<() => void> = [];

/** Run `f` once the module is instantiated (immediately if it already is). */
export function onEngineReady(f: () => void): void {
  if (initialized) f();
  else hooks.push(f);
}

function wasmUrl(): URL {
  return new URL('../pkg/oc_wasm_bg.wasm', import.meta.url);
}

type NodeProcess = { versions?: { node?: string }; getBuiltinModule?: (id: string) => unknown };

async function defaultInput(): Promise<EngineInput> {
  const url = wasmUrl();
  const proc = (globalThis as { process?: NodeProcess }).process;
  if (url.protocol === 'file:' && proc?.versions?.node && proc.getBuiltinModule) {
    const fs = proc.getBuiltinModule('node:fs') as { readFileSync(p: URL): Uint8Array };
    return fs.readFileSync(url);
  }
  return url;
}

/**
 * Instantiate the engine (idempotent). `@oc/engine` calls it with top-level await, so importers can use every
 * export synchronously; `@oc/engine/lazy` leaves it to the host (e.g. to pass bytes or a precompiled module).
 */
export function initEngine(input?: EngineInput): Promise<void> {
  ready ??= (async () => {
    const override = (globalThis as { __OC_ENGINE_WASM__?: EngineInput }).__OC_ENGINE_WASM__;
    await initWasm({ module_or_path: input ?? override ?? (await defaultInput()) });
    initialized = true;
    for (const f of hooks.splice(0)) f();
  })();
  return ready;
}

/** Whether `initEngine` has completed. */
export function engineReady(): boolean {
  return initialized;
}
