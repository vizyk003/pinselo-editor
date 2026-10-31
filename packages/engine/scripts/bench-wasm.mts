// SPDX-License-Identifier: Apache-2.0
/** Lets `OC_ENGINE_WASM` choose the wasm the benchmark loads (imported before @oc/engine initialises). */
import { readFileSync } from 'node:fs';

if (process.env.OC_ENGINE_WASM) (globalThis as { __OC_ENGINE_WASM__?: unknown }).__OC_ENGINE_WASM__ = readFileSync(process.env.OC_ENGINE_WASM);
