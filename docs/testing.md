# Testing

This document covers every test layer, how to run each one and what CI runs. Set-up is in [getting-started.md](getting-started.md), and the code being tested is described in [architecture.md](architecture.md).

## Layers at a glance

| Layer | Command | Needs |
|---|---|---|
| Root unit tests (vitest) | `pnpm test` | Built engine, for some tests |
| Type check | `pnpm typecheck` | Built engine |
| Engine tests (vitest) | `pnpm --filter @oc/engine test` | Built engine |
| Rust tests | `cargo test --workspace` | Fixtures; Postgres for the DB tests |
| Golden check | `bash tools/golden/check.sh` | The same, plus a committed `tests/golden` |
| API smoke test | `bash tests/smoke/api.sh [api-url]` | A running stack |
| Stack smoke test | `pnpm exec tsx tests/smoke/stack.ts [api-url] [collab-ws-url]` | A running stack |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | Rust toolchain |

`pnpm verify` runs `pnpm typecheck && pnpm test`. It does not run the engine tests, the Rust tests or the smoke tests.

## Before you run anything

Install dependencies and sync the fixtures. CI does the same, and the sync scripts download fonts and print profiles:

```sh
pnpm install
pnpm fonts:sync && pnpm emoji:sync && pnpm graphics:sync && pnpm icc:sync
```

Node comes from `.nvmrc`. Build the engine before the TypeScript tests:

```sh
pnpm --filter @oc/engine build
```

That needs the `wasm32-unknown-unknown` Rust target and a wasm-bindgen CLI of exactly the version in `Cargo.lock`. The build checks this and prints the install command. `-- --dev` gives a faster, unoptimised build on which the tests still pass (benchmarks are meaningless on it). See `packages/engine/README.md`.

Some Rust and engine tests use optional tools:

| Tool | Used by | Without it |
|---|---|---|
| ffmpeg | `oc-render-svc` video tests and routes, media steps in the `oc-api` parity test | Those parts are skipped |
| `pdftotext` (poppler-utils) | One text check in `oc-render-svc`'s parity tests | Only the PDF header is checked |
| `initdb`, `pg_ctl` | `oc-db` lockdown test | The test is skipped |

## Root unit tests

```sh
pnpm test                                  # vitest run
pnpm test:watch
pnpm exec vitest run tests/web-share.test.ts
```

The root `vitest.config.ts` includes `tests/**/*.test.ts` and `packages/*/src/**/*.test.ts`. These test the web app's logic without a browser (`tests/web-*.test.ts`), plus translations, templates and content rules, and `packages/config/src/config.test.ts`. Some of them import `@oc/engine`, directly or through `tests/engine-doc.ts`, so build the engine first.

## Engine tests

```sh
pnpm --filter @oc/engine test                                   # everything, including the benchmark
pnpm --filter @oc/engine exec vitest run --config vitest.config.ts --exclude test/bench.test.ts   # what CI runs
pnpm --filter @oc/engine exec vitest run --config vitest.config.ts test/sync.test.ts              # one file
pnpm --filter @oc/engine bench                                  # the benchmark only
```

These live in `packages/engine/test/` and test the WebAssembly build through the TypeScript facade. They have their own vitest config and are not part of `pnpm test`. CI excludes `test/bench.test.ts` because it asserts frame rates that shared runners do not reliably give.

| File | Checks |
|---|---|
| `editor-goldens.test.ts` | Editor sessions in `tests/golden/editor` replayed through the facade |
| `helpers-goldens.test.ts`, `drop-helpers.test.ts` | Schema, geometry and helper functions against goldens |
| `render.test.ts` | Display lists against `tests/golden/render`; tile painting equals painting a page at once |
| `ps1.test.ts` | WebAssembly pixels and text layout against the native baselines |
| `sync.test.ts` | `yjs` and the mirror converge, in both directions |
| `text.test.ts`, `arrange.test.ts`, `robustness.test.ts` | Text, arrangement, robustness |
| `bench.test.ts` | Benchmark |

Environment variables that narrow a run:

- `OC_GOLDEN_CASE=<name>` replays one editor session.
- `OC_PS1_FILTER=<substring>` limits the PS-1 pages.

## Rust tests

```sh
cargo test --workspace
cargo test -p oc-crdt
cargo test -p oc-render --test ps1
```

### Database tests

Tests that need Postgres use `oc_db::testing::TestDb`. Set one variable:

```sh
OC_DATABASE_URL=postgres://localhost:5432/opencanvas cargo test -p oc-collab
```

| Variable | Purpose |
|---|---|
| `OC_DATABASE_URL` | Required for DB tests. Identifies the server to use. |
| `RUST_TEST_THREADS` | Optional. CI uses `2`, see below. |
| `OC_TEST_PG_BIN` | Optional. A directory with `initdb` and `pg_ctl` for the lockdown test. Otherwise the test looks in the usual install locations and on `PATH`. |

`TestDb` connects to the server's `postgres` maintenance database, creates a database named `oc_rust_test_<uuid>`, applies all migrations and drops it afterwards. **The database named in the URL is never touched**, but the role must be able to connect to `postgres` and create databases.

**Without `OC_DATABASE_URL`, DB tests skip and pass.** A green run without it says nothing about the API parity transcripts, the collab conformance suite or the worker. Run with Postgres before you rely on the result.

Each DB test holds a throwaway database and a connection pool. CI sets `RUST_TEST_THREADS=2` to stay under the 100 connections that `postgres:17` allows by default. If you see connection errors locally, do the same.

The lockdown test (`crates/oc-platform/oc-db/tests/lockdown.rs`) does not use `OC_DATABASE_URL`. It starts its own temporary Postgres cluster, because it creates cluster-wide roles.

### WebAssembly target

`cargo test -p oc-wasm` runs the bridge's native tests. `crates/oc-wasm/tests/web.rs` only compiles for `wasm32` and runs in Node, so `cargo test --workspace` does not exercise it and neither does CI:

```sh
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test -p oc-wasm --target wasm32-unknown-unknown --test web
```

`wasm-bindgen-test-runner` has to be on your `PATH`, or be given as a full path; it is installed with the wasm-bindgen CLI. `packages/engine/README.md` shows the same command with an isolated rustup home.

## Goldens

`tests/golden/` holds committed expected outputs, and the Rust and engine tests compare against them. Most were recorded from an earlier TypeScript implementation, which has been removed. **Those files can no longer be regenerated by a tool, because the generators are gone.** They are frozen records of behaviour. Code comments and [ADR-0001](adr/0001-rust-core.md) mention `tools/golden/*.ts` and `pnpm golden`; neither exists any more. `tools/golden/` now contains only `check.sh` and `compare.mjs`.

| Directory | Checked by |
|---|---|
| `api` | `oc-api` parity transcripts |
| `crdt` | `oc-crdt` |
| `editor` | `oc-editor-core`, engine tests |
| `engine` | Engine tests (`drop-helpers`, `print-colors`) |
| `geometry`, `schema` | `oc-geometry`, `oc-schema`, engine tests |
| `import` | `oc-import` |
| `platform` | The `oc-platform` crates |
| `ps1` | `oc-render` (native), engine tests (WebAssembly) |
| `render` | `oc-render`, engine tests |
| `render-svc`, `render-video` | `oc-render-svc` |
| `social` | `oc-social` |
| `text` | `oc-text`, engine tests |
| `worker` | `oc-worker` |

### The golden check

```sh
OC_DATABASE_URL=postgres://localhost:5432/opencanvas bash tools/golden/check.sh
# or: pnpm golden:check
```

`tools/golden/check.sh` does the following:

1. It fails if `tests/golden` already has uncommitted changes. **Commit or stash golden changes before you run it.**
2. It fails if `fixtures/fonts/google-catalog.json` differs from the committed copy, because the catalogue is an input to some goldens. `pnpm fonts:catalog` fetches a live copy, which will trip this check. Restore the file, or commit it along with updated goldens.
3. It runs `cargo test --workspace --no-fail-fast`. Without `OC_DATABASE_URL`, it prints a notice and the DB tests skip.
4. If a test changed anything under `tests/golden`, it runs `tools/golden/compare.mjs`.

Tests should only read goldens. `compare.mjs` restores files that differ only by floating-point noise: numbers within `1e-12`, plus wider tolerances for a few values that come from external tools such as ffmpeg. Any other change fails the check: a different string, structure or key order, or a new or deleted file.

This is the only CI job that runs the whole Rust workspace's tests.

### PS-1: pixel and layout baselines

`tests/golden/ps1/baselines.json` is the one golden that is written by a current test. It holds fingerprints of `oc-render`'s output for every page of the hand-made scenes and the seed templates. `crates/oc-render/tests/ps1.rs` documents the corpus. Each page stores:

- `sha256`: a hash of the RGBA pixels. Equal means pixel-identical.
- `cells`: mean CIELAB values for a 16 × 16 grid. If the hash differs, the page still passes when no cell's CIEDE2000 exceeds 3.0.
- `layout`: a hash of the caret geometry of every text box. This must match, so line breaks and glyph positions cannot move.

`packages/engine/test/ps1.test.ts` checks the WebAssembly build against the same file.

### Changing a golden legitimately

Change a golden only when behaviour changed **on purpose**. Do not update one to make a failing test pass before you understand why it failed; an unexpected failure is usually a bug. Do it in the same commit as the code change, and say why in the commit message.

For rendering, there is a built-in update switch:

```sh
# 1. See what changed; the failure lists the affected pages. Review them.
cargo test -p oc-render --test ps1

# 2. Rewrite the baselines.
OC_PS1_UPDATE=1 cargo test -p oc-render --test ps1

# 3. Check the WebAssembly build against the new baselines.
pnpm --filter @oc/engine build
pnpm --filter @oc/engine exec vitest run --config vitest.config.ts test/ps1.test.ts

# 4. Commit, then run the full check (it refuses uncommitted goldens).
bash tools/golden/check.sh
```

`OC_PS1_UPDATE` is the only update switch in the test suites. For every other golden, edit the affected values in the file directly, keep the change as small as the behaviour change, and review the diff line by line. Because no generator exists to cross-check the result, the diff is the review.

For new behaviour, prefer a new Rust test over extending a recorded file. `crates/oc-api/tests/signup.rs` is an example: it reuses the parity harness's fakes without a transcript.

## Parity transcripts

Parity tests replay recorded behaviour against the Rust code.

| Test | What it replays | Run |
|---|---|---|
| `oc-api`, `tests/parity.rs` | HTTP transcripts in `tests/golden/api/<group>.json`, against the router | `cargo test -p oc-api --test parity` |
| `oc-worker`, `tests/parity.rs` | Worker phases in `tests/golden/worker`, including the real `oc-worker` binary draining seeded jobs | `cargo test -p oc-worker --test parity` |
| `oc-render-svc`, `tests/parity.rs`, `tests/video.rs` | Exporters against `tests/golden/render-svc` and `render-video` | `cargo test -p oc-render-svc` |
| `oc-render-svc`, `tests/service.rs` | Every route and the export job on a throwaway database | `cargo test -p oc-render-svc --test service` |

The `oc-api`, `oc-worker` and `service.rs` tests need `OC_DATABASE_URL`. To run one API group, filter by its name in `crates/oc-api/tests/parity.rs`:

```sh
OC_DATABASE_URL=postgres://localhost:5432/opencanvas cargo test -p oc-api --test parity designs
```

For each group, the API harness (`crates/oc-api/tests/harness/mod.rs`) creates its own database, object store directory, pinned clock and in-process fake of external services. Groups run one at a time. Volatile values are normalised before comparing, and mismatches are collected and reported together.

### Collab conformance

`crates/oc-collab/tests/provider.rs` runs the real browser client stack (`@hocuspocus/provider` and `yjs`) against the `oc-collab` binary, driven by `tests/collab/provider-client.ts`. The driver starts and kills the server itself. Its scenarios cover authentication, convergence, read-only roles, acknowledgement after persistence, malformed updates, offline resync and the debounced store; the driver's header has the full list.

```sh
OC_DATABASE_URL=postgres://localhost:5432/opencanvas cargo test -p oc-collab --test provider
```

It needs `pnpm` and the workspace's node modules. `OC_COLLAB_SERVER_CMD` points the same test at another server command.

`crates/oc-collab/tests/listener.rs` separately checks that the `LISTEN` connection survives a killed backend.

### Debugging editor sessions

`crates/oc-editor-core/tests/golden_sessions.rs` replays recorded editor sessions and has three debugging switches: `OC_GOLDEN_CASE=<name>` replays one session, `OC_GOLDEN_TRACE=<step>[,<step>…]` prints state before those steps, and `OC_GOLDEN_EL=<id>` adds that element to the trace.

## Smoke tests

Both smoke tests run against a live stack on a migrated database. Use a throwaway database, because they create accounts and data.

```sh
pnpm db:migrate
pnpm dev                        # or start the four binaries yourself

bash tests/smoke/api.sh http://localhost:4000
pnpm exec tsx tests/smoke/stack.ts http://localhost:4000 ws://localhost:4100
```

- **`tests/smoke/api.sh`** uses curl to check sign-up, the age gate, login, teams and invites, report validation, admin gating, the privacy export and security headers. The default URL is `http://localhost:4000`.
- **`tests/smoke/stack.ts`** signs in, creates a design and joins it through `oc-collab` with the editor's client stack. It paints the first page's background, waits for the API to read the edit back from Postgres, exports the page as PNG through `oc-render-svc`, and checks the pixel. Defaults: `OC_API_URL` or `http://localhost:4000`, and `ws://localhost:<OC_COLLAB_PORT or 4100>`. Set `OC_SMOKE_EMAIL` and `OC_SMOKE_PASSWORD` to sign in with an existing account instead of signing up.

`api.sh` always signs up a new account, so sign-up has to be open on the server (`OC_SIGNUP`, open by default).

## Other checks

- `pnpm typecheck` runs `tsc` through Turborepo. For `@oc/web`, it first requires a built engine. The engine package has no `typecheck` script, because its `build` does that.
- `pnpm i18n:check` runs the translation extraction check. CI does not run it.
- `pnpm audit --audit-level high` checks dependency vulnerabilities.

## What CI runs

`.github/workflows/ci.yml` runs on pushes to `main`, on pull requests and on manual dispatch. A newer run on the same ref cancels the older one. There are six jobs:

| Job | Runs | Services and tools |
|---|---|---|
| `verify` | Fixture sync; `pnpm --filter @oc/engine build`; `pnpm typecheck`; `pnpm test`; the engine tests without the benchmark; `pnpm --filter @oc/web build` | wasm32 target, wasm-bindgen CLI at the `Cargo.lock` version |
| `rust-smoke` | Debug build of the services and `oc-db`; `target/debug/oc-migrate`; starts the four services and waits for health; `tests/smoke/api.sh`; `tests/smoke/stack.ts` | Postgres 17, ffmpeg. Sets `OC_DATABASE_URL`, `OC_SECRET`, `OC_STAFF_EMAILS` |
| `golden` | Fixture sync; `tools/golden/check.sh` | Postgres 17, ffmpeg, poppler-utils. Sets `OC_DATABASE_URL`, `RUST_TEST_THREADS=2` |
| `docker` | Builds the `runtime` and `gateway` targets of `deploy/Dockerfile` and checks their contents. Nothing is pushed | Buildx |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | Rust 1.99.0, pinned so a new stable release cannot fail CI unannounced |
| `audit` | `pnpm audit --audit-level high` | |

To reproduce the `rust-smoke` database, use `postgres://oc:oc@localhost:5432/opencanvas`, which is the URL CI sets. Local health checks are the same as CI's: `localhost:4000/healthz`, `localhost:4200/healthz` and `localhost:4100/`.

Not covered by CI: the wasm-target `oc-wasm` tests, the benchmarks and `pnpm i18n:check`.
