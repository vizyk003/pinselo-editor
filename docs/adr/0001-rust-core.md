# ADR-0001 — Rust core, TypeScript shell

- **Status:** accepted · 2026-10-10; implemented · 2026-10-27 (the TypeScript core and backend are deleted)
- **Supersedes:** the earlier decision to use CanvasKit everywhere.

## Decision

Everything except the browser UI chrome moves to Rust. The shell stays in TypeScript on purpose: the browser's
input stack (IME, pointer/touch, clipboard, drag and drop, contenteditable via TipTap) and its accessibility tree
are the parts a pure-Rust UI cannot fully replace.

| Layer | Before (TS) | After |
|---|---|---|
| Document model | `@oc/schema` (zod) | `crates/oc-schema` (serde), JSON shape unchanged |
| Geometry | `@oc/geometry` | `crates/oc-geometry` |
| CRDT | `@oc/crdt` (yjs) | `crates/oc-crdt` (yrs, same Y.Doc layout, wire-compatible) |
| Editor logic | `@oc/editor-core` | `crates/oc-editor-core` (DocOps in, DocOps out) |
| Text | `@oc/text` (SkParagraph) | `crates/oc-text` (parley + swash) |
| Rendering | `@oc/render-core` + `@oc/render-canvaskit` | `crates/oc-render` (display list + tiny-skia raster + PDF) |
| Sheets | `@oc/sheet` (HyperFormula) | `crates/oc-sheet` |
| Platform libs | config, db, auth, storage, jobs, mail, webhooks, audit, gfonts | `crates/oc-platform/*` |
| Services | api, collab, render, worker (Fastify, Hocuspocus) | `crates/oc-api`, `oc-collab`, `oc-render-svc`, `oc-worker` (axum, tokio, sqlx) |
| Browser bridge | — | `crates/oc-wasm` (wasm-bindgen) behind thin TS facades with the old signatures |
| **Stays in TS** | `apps/web` React components, routing, i18n, TipTap, pointer/keyboard handling, ARIA, `@hocuspocus/provider` + `yjs` as the browser sync transport | unchanged |

### Choices

1. **Renderer: pure Rust** (tiny-skia raster, parley/swash text, own PDF writer). One engine in the browser (WASM)
   and on the server (native), so the PS-1 "export looks exactly like the editor" guarantee holds. PS-1 pixel
   baselines are regenerated once at cutover.
2. **CRDT: yrs on the server, yjs in the browser.** yrs is wire-compatible with yjs. The browser keeps a yjs
   `Y.Doc` because TipTap and the provider are built on it; Rust WASM reads snapshots and returns DocOps, which the
   TS shell applies to the yjs doc. The Y.Doc layout (map names, key names) is frozen and shared.
3. **Strangler migration.** Each TS package is replaced by a Rust crate behind the same JSON/HTTP/wire contract.
   A TS package is deleted only when its Rust replacement passes the golden tests generated from the TS
   implementation (`tests/golden/`) and the existing smoke tests.

## Parity method

For each package, a TS script (`tools/golden/<pkg>.ts`) runs the TS implementation over fixture inputs
(`fixtures/templates-seed/*.json` plus hand-written cases) and writes `tests/golden/<pkg>/*.json`. The Rust crate's
tests load the same files and must match: exact for structural output, within 1e-6 for floats. The goldens are
committed, so the TS can be deleted afterwards and the contract still holds.

**Generator convention.** A golden generator is a `tools/golden/<pkg>.ts` with a line that is exactly
`// golden-generator` in its first 5 lines (after the SPDX line if there is one). `pnpm golden` runs every marked
file (`pnpm golden schema crdt` runs a subset); unmarked files in `tools/golden/` (drivers such as
`collab-client.ts`, probes) are skipped. Every new generator must carry the marker. A generator must be
deterministic: running it twice writes byte-identical files (pin `Date.now`/`new Date()`, seed every RNG, replace
`randomUUID`, sort unordered output). It must not need the network, and fixtures that need Postgres are written
only when `OC_GOLDEN_DATABASE_URL` points at an empty throwaway database (otherwise they are left untouched).

**Drift check.** While a TS package is still the reference, a TS change can silently break parity with its Rust
port. `tools/golden/check.sh` (`pnpm golden:check`, and the `golden` CI job) regenerates every golden, fails if
`git status --porcelain tests/golden` is non-empty, then runs `cargo test --workspace`. With `OC_DATABASE_URL` set
it creates and drops a throwaway database for the DB-backed fixtures. Node/V8 versions compute some `Math` results
(`Math.pow`/`**`, so luminance and easing curves) to a different last digit (Node 22 and the CI's Node 24 from
`.nvmrc` disagree on `10 ** -4`), so before failing, `tools/golden/compare.mjs` restores changed files whose only
differences are numbers within 1e-12 (relative; absolute below 1); any other change (strings, structure, key order,
new or deleted files) is drift. Generate goldens with the Node version in `.nvmrc` where possible, and keep outputs
that are engine-dependent beyond float noise (e.g. a threshold decided on a `**` result) out of the goldens.
Numbers inside strings are invisible to compare.mjs, so a generator that writes them must make them
engine-independent: `tools/golden/render.ts` rounds SVG path coordinates to 1e-6 (`canonPath`), and the oc-render
tests compare path strings number by number within 1e-6. Unicode properties come from Node's ICU, so `pnpm golden`
refuses to run on a Node whose Unicode version differs from the one the goldens were made with (17: Node 24.x,
Node >= 22.23; `GOLDEN_UNICODE` in `tools/golden/run.sh`). The API transcripts record Node 24's passkey algorithms
(ML-DSA-44 offered first when `SubtleCrypto.supports('verify', 'ML-DSA-44')`, Node >= 24.7), so `tools/golden/api.ts`
refuses to run on a Node without it. To fix
a failure, port the TS change to the Rust crate, extend the generator to cover the new behaviour, and commit the
regenerated goldens with the port.

## Phases

1. Workspace, `oc-schema`, `oc-geometry`, platform crates.
2. `oc-crdt` (yrs, cross-decoding yjs updates), `oc-editor-core`.
3. `oc-text`, `oc-render` (raster + PDF + SVG), `oc-sheet`.
4. `oc-wasm` + TS facades in `apps/web`; remove CanvasKit from the browser. *(Done on branch `engine-web`: apps/web
   depends on `@oc/engine` only — editor, schema, geometry, sheet, text and renderer in one WebAssembly module; the
   editor paints with oc-render in scene-anchored tiles, see `packages/engine/README.md`. The TS packages remain in the
   repo for the services and the goldens until phase 6.)*
5. Services: collab → render → worker → api, each cut over behind the same ports and routes.
6. Delete the replaced TS packages, regenerate PS-1 baselines, update the docs.
