// SPDX-License-Identifier: Apache-2.0
/**
 * `@oc/engine`: the Pinselo Rust core in the browser, behind the `@oc/editor-core`, `@oc/geometry` and
 * `@oc/schema` signatures. The module initialises the WebAssembly with top-level await, so every export (constants
 * included) is usable as soon as the import resolves — like the TS packages it replaces. Needs an ES2022 target
 * (apps/web builds with `target: 'es2022'`). Use `@oc/engine/lazy` to control initialisation yourself.
 */
import { initEngine } from './wasm';

export * from './api';

await initEngine();
