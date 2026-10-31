# Getting started

This guide takes you from a fresh clone to a running local stack: the web app and four Rust services against a local Postgres. To run Pinselo in production, see [self-hosting.md](self-hosting.md). Every environment variable is listed in [configuration.md](configuration.md).

## Prerequisites

| Tool | Version | Where the version comes from |
|---|---|---|
| Node.js | 24 recommended, 22 or newer accepted | `.nvmrc` says `24`; `engines` in `package.json` says `>=22` |
| pnpm | 10.15.1 | `packageManager` in `package.json` |
| Rust (`cargo`, `rustc`) | Stable | There is no `rust-toolchain` file. CI uses current stable; the Docker build and the CI lint job pin 1.99.0 |
| `wasm32-unknown-unknown` target | Same toolchain | Required by `packages/engine/scripts/build.mjs` |
| `wasm-bindgen` CLI | Exactly the `wasm-bindgen` version in `Cargo.lock` (0.2.129 at the time of writing) | Checked by the engine build, which stops on a mismatch |
| PostgreSQL | 17 | Used by CI and `deploy/docker-compose.yml` |
| `npm`, `tar`, `unzip` | Any | Called by the asset sync scripts in `tools/scripts` |
| `ffmpeg` | Any | Needed for video and audio features; see [What is optional](#what-is-optional) |

Install the toolchain pieces that are specific to this repository:

```bash
corepack enable        # gives you the pinned pnpm, or install pnpm 10 another way
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # use the version in your Cargo.lock
```

## Install and set up

Run these from the repository root:

```bash
pnpm install
createdb opencanvas                # the default database name
pnpm --filter @oc/engine build     # compiles the Rust core to WebAssembly
pnpm run setup                     # syncs assets, migrates, seeds
```

Two details matter here:

- **Use `pnpm run setup`, not `pnpm setup`.** `pnpm setup` is a built-in pnpm command that configures pnpm itself, so it does not run this repository's script.
- **Build the engine first.** The `seed` step of `setup` builds the template catalogue on `@oc/engine`, which needs the output in `packages/engine/pkg`. Nothing in `setup` builds it.

`pnpm run setup` runs these steps in order. You can also run any of them on its own.

| Command | What it does | Network |
|---|---|---|
| `pnpm fonts:sync` | Downloads the bundled fonts to `fixtures/fonts/files` | npm registry |
| `pnpm emoji:sync` | Writes the emoji stickers to `fixtures/emoji` | npm registry |
| `pnpm graphics:sync` | Writes the icons and graphics to `fixtures/graphics`; also compiles a Rust helper | npm registry |
| `pnpm icc:sync` | Downloads the print colour profiles to `fixtures/icc` and checks their hashes | Profile download URLs |
| `pnpm db:migrate` | Builds and runs `oc-migrate` | None |
| `pnpm seed` | Builds the templates, then runs `oc-templates seed` to store them and their thumbnails | May fetch template photos; they are cached in `fixtures/template-photos` |

The first Rust build takes a few minutes. Later builds only recompile what changed.

## Run

```bash
pnpm dev
```

This builds the services in release mode into `target/dev`, then starts them next to the Vite dev server. Output lines are prefixed with the process name. If one process exits, the others stop too.

| Process | Port | Binary or command | Listens on |
|---|---|---|---|
| Web | 3000 | `pnpm --filter @oc/web dev` (Vite) | Vite default |
| API | 4000 | `oc-api` | All interfaces |
| Collaboration | 4100 | `oc-collab` | All interfaces |
| Render | 4200 | `oc-render-svc` | `127.0.0.1` |
| Worker | None | `oc-worker` | Not applicable |

Open <http://localhost:3000>. Vite proxies `/api`, `/site`, `/v1`, `/mcp`, `/.well-known/oauth-*` and `/collab` to the services, so the browser only talks to port 3000.

You can check the services directly:

```bash
curl localhost:4000/healthz
curl localhost:4200/healthz
curl localhost:4100/
```

### Useful switches

The dev tooling does not load a `.env` file. Set variables in your shell or in front of the command.

| Command | Effect |
|---|---|
| `OC_DEV_WATCH=1 pnpm dev` | Rebuilds and restarts the services when a `.rs` or `.toml` file under `crates/` changes |
| `OC_DEV_SERVICES=api,collab pnpm dev` | Starts only these services, plus the web app. Names: `api`, `collab`, `worker`, `render` |
| `OC_DEV_NO_WEB=1 pnpm dev` | Starts the services without the web app |
| `OC_DEV_TARGET_DIR=/some/dir pnpm dev` | Builds the services into another directory |

The web app rebuilds `@oc/engine` on start when a Rust source or `Cargo.lock` is newer than the last engine build. `OC_ENGINE_SKIP=1` uses the existing build as it is.

### Running a second copy

To run a second checkout next to the first, give it its own ports and database. Set `OC_RENDER_URL` and `OC_WEB_ORIGIN` as well, because their defaults do not follow the port variables:

```bash
createdb opencanvas2
export OC_DATABASE_URL=postgres://localhost:5432/opencanvas2
export OC_WEB_PORT=3001 OC_API_PORT=4001 OC_COLLAB_PORT=4101 OC_RENDER_PORT=4201
export OC_RENDER_URL=http://localhost:4201 OC_WEB_ORIGIN=http://localhost:3001
pnpm db:migrate && pnpm dev
```

## Signing in and reading email

Outside production you can sign in without a password. On the sign-in page, choose **Development sign-in without a password** and enter an email address. If there is no account for that address, one is created. To turn this off, set `OC_DEV_LOGIN=0`.

Without `OC_SMTP_URL`, email is not sent. Sign-in links, invites and resets stay in the database, and you can read them in the dev mailbox at <http://localhost:3000/dev/mail>. The mailbox is unavailable in production and when `OC_SMTP_URL` is set.

To get the staff console at `/admin`, start the API with your address in `OC_STAFF_EMAILS`:

```bash
OC_STAFF_EMAILS=you@example.com pnpm dev
```

## What is optional

| Item | Without it |
|---|---|
| `pnpm seed` | The template catalogue is empty |
| `ffmpeg` | Video and audio exports and media ingest fail. The API, render service and worker all call it |
| SMTP (`OC_SMTP_URL`) | Email stays in the dev mailbox |
| Object storage (`OC_S3_BUCKET`) | Files are stored in `.data/objects` |
| Stock media keys | Openverse still works without a key. Unsplash, Pexels, Pixabay and GIPHY stay off |
| Sign-in providers | Passwords, magic links, passkeys and the development sign-in remain |
| Social network credentials | Those networks stay unavailable in the Content Planner. Bluesky needs no server credentials |
| Stripe | A local stand-in handles payments with no charge, outside production only |
| Supabase | Not used. See [supabase.md](supabase.md) if you want it |
| `OC_WHISPER_CMD`, `OC_WHISPER_MODEL` | Auto captions report that they are unavailable |

Local state lives in the database and in `.data/`. Both can be recreated with `pnpm db:migrate` and `pnpm seed`, but anything you made locally is lost.

## Common failures

These messages come from the scripts in `tools/scripts`, `apps/web/scripts` and `packages/engine/scripts`, and from the service binaries.

| Message | Cause | Fix |
|---|---|---|
| No assets synced and no database migrated after `pnpm setup` | You ran the pnpm built-in, which configures pnpm in your shell profile | Run `pnpm run setup` |
| `wasm-bindgen CLI <version> required (found: …)` | The CLI is missing or does not match `Cargo.lock` | Run the install command the message prints |
| `cargo not found` | The engine build cannot find Rust | Install Rust, or set `OC_WASM_CARGO` to a `cargo` binary |
| `The Rust toolchain at … has no wasm32-unknown-unknown target` | The target is not installed | `rustup target add wasm32-unknown-unknown` |
| `@oc/web needs @oc/engine, the Rust core compiled to WebAssembly` | `packages/engine/pkg` is missing or its build failed | `pnpm --filter @oc/engine build`, and read its error |
| An import error for `packages/engine/pkg/oc_wasm.js` during `pnpm seed` | The engine has not been built | `pnpm --filter @oc/engine build` |
| `fonts:sync needs npm on PATH` | `npm` is not available | Install npm or fix your `PATH` |
| `oc-sanitize-svg failed` during `pnpm graphics:sync` | The Rust helper failed to build or run | Read the `cargo` output above the message |
| `<profile>: GET <url> → <status>` or `sha256 …, expected …` during `pnpm icc:sync` | The profile download failed or changed | Check your network and rerun. A hash mismatch stops the sync on purpose |
| `[dev] the Rust build failed and there is no previous binary for …` | The first build failed | Read the `cargo` errors above the message |
| `[dev] the Rust build failed — running the previous binaries` | A later build failed | Your code is not what is running; fix the build error |
| `[oc-…] …/release/oc-… is missing` | `OC_DEV_SKIP_BUILD=1` with nothing built | Unset the variable |
| `[api] exited (…); stopping the others`, or the same for another process | One process stopped, so `pnpm dev` stops them all | Read that process's lines above the message |
| `listen on 0.0.0.0:4000`, `cannot listen on :4100`, `cannot listen on 127.0.0.1:4200` | The port is in use | Stop the other process or change the port. See [Running a second copy](#running-a-second-copy) |
| Vite reports that port 3000 is in use | Vite runs with `strictPort`, so it will not pick another port | Stop the other process or set `OC_WEB_PORT` |
| `OC_DATABASE_URL points at a transaction-mode pooler` | The URL uses port 6543 or `pgbouncer=true` | Use a session-mode or direct connection |
| `OC_DATABASE_URL points at Supabase without TLS settings` | Supabase host with no CA set | Set `OC_DATABASE_SSL_CA`; see [supabase.md](supabase.md) |
| `invalid configuration: OC_… is not an integer` | A numeric variable has a non-numeric value | Fix the value |
| `invalid configuration: OC_SIGNUP="…" must be open or invite` | Unknown sign-up mode | Use `open` or `invite` |

If your machine has an isolated Rust install at `~/.rustup/cargo/bin/cargo`, the engine build uses it instead of the `cargo` on your `PATH`. It sets `RUSTUP_HOME` and `CARGO_HOME` to match, so install the target and `wasm-bindgen` there. The error messages print the exact commands.

## Next steps

- [architecture.md](architecture.md): how the crates and the web app fit together
- [testing.md](testing.md): running the test suites
- [api.md](api.md): the HTTP API
- [mcp-connector.md](mcp-connector.md): connecting an AI assistant
- [user-guide.md](user-guide.md): using the editor
