# Configuration

Pinselo is configured with environment variables. This page lists every variable that the code reads. Defaults come from the code, mainly `crates/oc-platform/oc-config/src/lib.rs`. See [getting-started.md](getting-started.md) for local use and [self-hosting.md](self-hosting.md) for production.

**Service names:** `api` (`oc-api`), `collab` (`oc-collab`), `render` (`oc-render-svc`), `worker` (`oc-worker`), `migrate` (`oc-migrate`), `gateway` (Caddy). "Services" means api, collab, render and worker.

The Rust services and the tooling do not load `.env` files. In production, Compose passes `deploy/.env` to the containers through `env_file`. Locally, set variables in your shell.

Required means required for a working production deployment unless the table says otherwise. Nothing is required locally.

## Empty and unset values

An empty value is not always the same as an unset one. `VAR=` in a Compose `env_file` sets the variable to an empty string.

| Behaviour | Variables |
|---|---|
| Empty means the **default** | `OC_DATABASE_POOL_MAX`, `OC_API_PORT`, `OC_COLLAB_PORT`, `OC_RENDER_PORT`, `OC_UPLOAD_QUOTA`, `OC_MAX_UPLOAD_BYTES`, `OC_MAX_MEDIA_BYTES`, `OC_SIGNUP` |
| Empty means **off** | Credentials and keys for sign-in, stock media and speech-to-text, `OC_SUPPORT_EMAIL`, `OC_COLLAB_URL`, `OC_ICC_DIR` for api and render |
| Empty is **taken literally** | All other variables on this page, unless noted. Delete the line rather than leaving it empty |

The literal case causes real problems with these variables:

- `OC_S3_BUCKET=` selects S3 with an empty bucket name instead of local storage.
- `OC_S3_ENDPOINT=` and `OC_S3_REGION=` pass empty values to the S3 client.
- `OC_SMTP_URL=` makes the worker fail on delivery instead of keeping the message in the outbox. The api still treats it as no SMTP.
- `OC_MAIL_FROM=` is an invalid sender address.

Non-numeric values for the integer variables stop the service with `invalid configuration`.

## Core

| Name | Required | Default | Service | Purpose |
|---|---|---|---|---|
| `NODE_ENV` | Yes | `development` | All Rust binaries | `production` turns on the startup checks and turns off development features. The `runtime` image sets it |
| `OC_SECRET` | Yes, 32+ characters | `dev-only-secret-change-me` | Services | Signs sessions and service tickets. Also the source of the key that encrypts connected social accounts' tokens. The default is refused in production |
| `OC_WEB_ORIGIN` | Yes, `https://` | `http://localhost:3000` | api, render, worker | Public origin of the app |
| `OC_API_URL` | Yes, with sign-in providers | `http://localhost:4000` | api | Base of the sign-in provider callback URL (`/api/auth/oauth/<id>/callback`) |
| `OC_USERCONTENT_ORIGIN` | Recommended | Empty (same origin) | api, render, worker | Origin that serves uploads. In production, its hostname must not end with the app's hostname. Use a different registrable domain |
| `OC_API_PORT` | No | `4000` | api, Vite proxy | API port |
| `OC_COLLAB_PORT` | No | `4100` | collab, Vite proxy; api for the `OC_COLLAB_URL` default | Collaboration port |
| `OC_RENDER_PORT` | No | `4200` | render | Render port |
| `OC_RENDER_HOST` | No | `127.0.0.1` | render | Bind address. Compose sets `0.0.0.0` |
| `OC_RENDER_URL` | Between containers | `http://localhost:4200` | api, collab, worker | Where to reach render. Does not follow `OC_RENDER_PORT` |
| `OC_COLLAB_URL` | Between containers | `ws://localhost:<OC_COLLAB_PORT>` | api | Where the AI connector reaches collab |
| `OC_DATA_DIR` | No | `<repository>/.data` | api, render, worker, tooling | Data directory. Holds `objects`, `thumbs` and `s3-cache`. Relative paths resolve against the working directory. Compose sets `/data` |
| `OC_ICC_DIR` | No | `<repository>/fixtures/icc` | api, render, `icc:sync` | Directory of print colour profiles |
| `OC_MIGRATIONS_DIR` | No | `<repository>/crates/oc-platform/oc-db/migrations` | migrate | Directory of migration files |

`deploy/docker-compose.yml` sets `OC_RENDER_URL`, `OC_COLLAB_URL`, `OC_WEB_ORIGIN`, `OC_API_URL`, `OC_USERCONTENT_ORIGIN`, `OC_DATA_DIR` and `OC_RENDER_HOST` itself. Those values override `.env`.

## Accounts and access

| Name | Required | Default | Service | Purpose |
|---|---|---|---|---|
| `OC_SIGNUP` | No | `open` | api | `open` or `invite`. With `invite`, only `OC_SIGNUP_ALLOW`, `OC_STAFF_EMAILS` and people with a pending team invite can create accounts. Existing accounts can still sign in. Other values stop the service |
| `OC_SIGNUP_ALLOW` | No | Empty | api | With `invite`: comma-separated emails and `@domain` entries |
| `OC_STAFF_EMAILS` | No | Empty | api | Comma-separated emails that get the staff console at `/admin`. Applied at startup |
| `OC_DEV_LOGIN` | No | On outside production | api | `0` turns off the passwordless development sign-in. It is always off in production |
| `OC_SUPPORT_EMAIL` | No | Empty | api | Receives a copy of support requests |

## Database

| Name | Required | Default | Service | Purpose |
|---|---|---|---|---|
| `OC_DATABASE_URL` | Yes | `postgres://localhost:5432/opencanvas` | Services, migrate, tooling | Session-mode or direct Postgres URL. Port 6543 and `pgbouncer=true` are refused. Compose defaults to `postgres://oc:<POSTGRES_PASSWORD>@db:5432/opencanvas` |
| `OC_DATABASE_SSL_CA` | With Supabase | Empty | Services, migrate | CA certificate: inline PEM (literal `\n` accepted), an absolute path, or a path relative to the repository root. When set, the certificate and hostname are verified |
| `OC_DATABASE_POOL_MAX` | No | `10` | Services, migrate | Connections per process. Each service also opens one listener |
| `PGHOST`, `PGPORT` | No | Not set | Services, migrate | Used when `OC_DATABASE_URL` has no host or port |
| `POSTGRES_PASSWORD` | With the bundled `db` | None | Compose | Password for the `db` service, and used in the default database URL |

## File storage

| Name | Required | Default | Service | Purpose |
|---|---|---|---|---|
| `OC_S3_BUCKET` | No | Not set (local storage in `<OC_DATA_DIR>/objects`) | api, render, worker, `oc-templates` | Set to use an S3-compatible store. See [Empty and unset values](#empty-and-unset-values) |
| `OC_S3_REGION` | No | `auto` | Same | Region |
| `OC_S3_ENDPOINT` | For non-AWS stores | Not set | Same | Custom endpoint. Also switches to path-style addressing |
| `OC_S3_CHECKSUMS` | No | On | Same | `off` stops SHA-256 upload checksums. Needed for Supabase Storage |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | With S3 | None | Same | Read by the AWS SDK's standard credential chain, which also accepts its other usual sources |
| `OC_UPLOAD_QUOTA` | No | `5368709120` (5 GiB) | api | Fair-use storage quota per user, in bytes. Uploads and restores over it fail with 402 `upgrade_required` |
| `OC_MAX_UPLOAD_BYTES` | No | `52428800` (50 MiB) | api | Largest upload |
| `OC_MAX_MEDIA_BYTES` | No | `1073741824` (1 GiB) | api | Largest video or audio upload |
| `OC_GC_GRACE_HOURS` | No | `48` | worker | Age an unreferenced object must reach before removal. Must be a number |
| `OC_GC_DRY_RUN` | No | Off | worker | `1` reports what would be removed without deleting |

## Email

| Name | Required | Default | Service | Purpose |
|---|---|---|---|---|
| `OC_SMTP_URL` | Yes | Not set | worker, api | `smtp://user:pass@host:port` (default port 587, STARTTLS when offered) or `smtps://…` (default port 465). When not set, messages stay in the outbox and are marked sent. Outside production, you can read them at `/dev/mail`; in production they are not delivered at all |
| `OC_MAIL_FROM` | Recommended | `Pinselo <no-reply@<host of OC_WEB_ORIGIN>>` | worker | Sender address |

## Sign-in providers

All optional. A provider appears only when all its required values are non-empty.

| Name | Default | Service | Purpose |
|---|---|---|---|
| `OC_GOOGLE_CLIENT_ID`, `OC_GOOGLE_CLIENT_SECRET` | Empty | api | Google sign-in |
| `OC_APPLE_CLIENT_ID`, `OC_APPLE_CLIENT_SECRET` | Empty | api | Apple sign-in |
| `OC_OIDC_ISSUER`, `OC_OIDC_CLIENT_ID`, `OC_OIDC_CLIENT_SECRET` | Empty | api | Any OpenID Connect provider |
| `OC_OIDC_LABEL` | `SSO` | api | Button label for that provider |

## Supabase

All optional. See [supabase.md](supabase.md).

| Name | Default | Service | Purpose |
|---|---|---|---|
| `OC_SUPABASE_URL` | Empty | api; gateway for the CSP | `https://<ref>.supabase.co`. Supabase Auth is on only when this and the publishable key are both set |
| `OC_SUPABASE_PUBLISHABLE_KEY` | Empty | api | Publishable key, sent to browsers. A secret key here stops the services in every environment |
| `OC_SUPABASE_SECRET_KEY` | Empty | api | Server-only. Required in production with Supabase Auth; used to delete Supabase users |
| `OC_SUPABASE_JWT_SECRET` | Empty | api | Legacy HS256 secret |
| `OC_SUPABASE_JWT_ISSUER` | `<OC_SUPABASE_URL>/auth/v1` | api | Expected token issuer, for custom auth domains |

## Stock media

All optional, all read by api. The `*_API_URL` variables exist mainly to point at a test server.

| Name | Default | Purpose |
|---|---|---|
| `OC_STOCK_OPENVERSE` | On | `0` turns off Openverse |
| `OC_OPENVERSE_CLIENT_ID`, `OC_OPENVERSE_CLIENT_SECRET` | Empty | Openverse credentials. Openverse also works without them |
| `OC_UNSPLASH_KEY` | Empty | Turns on Unsplash |
| `OC_PEXELS_KEY` | Empty | Turns on Pexels |
| `OC_PIXABAY_KEY` | Empty | Turns on Pixabay |
| `OC_GIPHY_KEY` | Empty | Turns on GIPHY |
| `OC_OPENVERSE_API_URL` | `https://api.openverse.org` | Base URL |
| `OC_UNSPLASH_API_URL` | `https://api.unsplash.com` | Base URL |
| `OC_PEXELS_API_URL` | `https://api.pexels.com` | Base URL |
| `OC_PIXABAY_API_URL` | `https://pixabay.com` | Base URL |
| `OC_GIPHY_API_URL` | `https://api.giphy.com` | Base URL |

The web app's Content Security Policy in `deploy/Caddyfile` lists the image hosts of these providers.

## Social publishing

All optional, read by api and worker. Set a network's credentials to make it available in the Content Planner. Set them on both services; Compose does this through `.env`.

| Name | Default | Purpose |
|---|---|---|
| `OC_X_CLIENT_ID`, `OC_X_CLIENT_SECRET` | Empty | X |
| `OC_LINKEDIN_CLIENT_ID`, `OC_LINKEDIN_CLIENT_SECRET` | Empty | LinkedIn |
| `OC_LINKEDIN_VERSION` | `202507` | `linkedin-version` request header |
| `OC_FACEBOOK_CLIENT_ID`, `OC_FACEBOOK_CLIENT_SECRET` | Empty | Facebook and Instagram |
| `OC_FACEBOOK_GRAPH_VERSION` | `v23.0` | Graph API version |
| `OC_THREADS_CLIENT_ID`, `OC_THREADS_CLIENT_SECRET` | Empty | Threads |
| `OC_PINTEREST_CLIENT_ID`, `OC_PINTEREST_CLIENT_SECRET` | Empty | Pinterest |
| `OC_TIKTOK_CLIENT_KEY`, `OC_TIKTOK_CLIENT_SECRET` | Empty | TikTok |
| `OC_TIKTOK_MODE` | Inbox | `direct` posts directly. Any other value sends to the TikTok inbox |
| `OC_YOUTUBE_CLIENT_ID`, `OC_YOUTUBE_CLIENT_SECRET` | Empty | YouTube |
| `OC_BLUESKY_SERVICE` | `https://bsky.social` | Bluesky service. Bluesky needs no server credentials |
| `OC_BLUESKY_DISABLED` | Off | `1` hides Bluesky |

## Media and content safety

All optional.

| Name | Default | Service | Purpose |
|---|---|---|---|
| `OC_FFMPEG` | `ffmpeg` | worker | The ffmpeg command for social publishing. api and render always run `ffmpeg` from `PATH` |
| `OC_PHOTODNA_KEY` | Empty | api | Turns on PhotoDNA hash matching of uploads. Without it, no check is made |
| `OC_PHOTODNA_URL` | `https://api.microsoftmoderator.com/photodna/v1.0/Match?enhance=false` | api | PhotoDNA endpoint |
| `OC_PHOTODNA_FAIL_CLOSED` | Off | api | `1` reports the check as unavailable when it errors. Otherwise the upload passes |

## Gateway and Compose

These are read by Compose or by Caddy, not by the Rust services.

| Name | Required | Default | Read by | Purpose |
|---|---|---|---|---|
| `OC_APP_HOST` | Yes | None | Compose, Caddy | App hostname. Compose derives `OC_WEB_ORIGIN` and `OC_API_URL` from it |
| `OC_USERCONTENT_HOST` | With `deploy/docker-compose.yml` | None | Compose, Caddy | Uploads hostname. Compose derives `OC_USERCONTENT_ORIGIN` from it. Not used by `deploy/oracle` |
| `OC_ACME_EMAIL` | Yes | None | Caddy | Email for the certificate account |
| `OC_RUNTIME_IMAGE` | No | The original repository's image | `deploy/oracle/compose.yml` | Image for the services. Set your own |
| `OC_GATEWAY_IMAGE` | No | The original repository's image | `deploy/oracle/compose.yml` | Image for the gateway. Set your own |

## Backup script

Read by `deploy/oracle/backup.sh` from `.env`. All optional. Without `OC_BACKUP_S3_BUCKET`, backups stay on the server. The script also reads `OC_DATABASE_URL`, `OC_S3_ENDPOINT`, `OC_S3_REGION`, `OC_S3_BUCKET`, `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`.

| Name | Default | Purpose |
|---|---|---|
| `OC_BACKUP_S3_BUCKET` | Empty | Bucket for the off-server copy |
| `OC_BACKUP_S3_REGION` | Empty | Its region |
| `OC_BACKUP_S3_ENDPOINT` | Empty | Its S3-compatible endpoint |
| `OC_BACKUP_S3_ACCESS_KEY_ID` | Empty | Its access key |
| `OC_BACKUP_S3_SECRET_ACCESS_KEY` | Empty | Its secret key |

`deploy/oracle/build-on-server.sh` reads `OC_SERVER`, the SSH target for the build. It is required.

## Web app dev server and engine build

Only used at development or build time.

| Name | Default | Read by | Purpose |
|---|---|---|---|
| `OC_WEB_PORT` | `3000` | `apps/web/vite.config.ts` | Vite port. Vite will not fall back to another port |
| `OC_ENGINE_SKIP` | Off | `apps/web/scripts/engine.mjs` | `1` skips the engine freshness check and uses the existing build |
| `OC_WASM_CARGO` | `~/.rustup/cargo/bin/cargo` if present, else `cargo` | `packages/engine/scripts/build.mjs` | `cargo` for the engine build |
| `OC_WASM_BINDGEN` | `wasm-bindgen` from `CARGO_HOME/bin` if present, else from `PATH` | Same | `wasm-bindgen` CLI |
| `CARGO_TARGET_DIR` | `<repository>/target/engine-wasm` | Same | Build directory for the engine. `pnpm dev` and the tooling set their own value, from `OC_DEV_TARGET_DIR` |
| `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS` | Empty | Same | Extra flags; the build appends the SIMD flag |

## Development tooling

| Name | Default | Read by | Purpose |
|---|---|---|---|
| `OC_DEV_SERVICES` | All | `tools/scripts/dev.ts` | Comma-separated services to start: `api`, `collab`, `worker`, `render` |
| `OC_DEV_NO_WEB` | Off | Same | `1` does not start the web app |
| `OC_DEV_WATCH` | Off | Same | `1` rebuilds and restarts the services on changes in `crates/` |
| `OC_DEV_TARGET_DIR` | `target/dev` | `rust.ts`, `graphics-sync.ts` | Build directory for the service binaries |
| `OC_DEV_SKIP_BUILD` | Off | `rust.ts` | `1` runs a binary without building it first. Applies to `pnpm db:migrate` and `pnpm seed`, not `pnpm dev` |
| `RUST_BACKTRACE` | `1` | `dev.ts`, `rust.ts` | Passed on to the binaries |
| `OC_OAUTH_FAKE_BASE` | Empty | api | Test server for OAuth client metadata lookups. Ignored in production |
| `OC_SOCIAL_FAKE_BASE` | Empty | api | Test server for social network calls. Ignored in production |

## Tests

See [testing.md](testing.md).

| Name | Read by | Purpose |
|---|---|---|
| `OC_DATABASE_URL` | Rust DB tests, `tools/golden/check.sh`, `tests/collab/provider-client.ts` | Server for throwaway test databases. Without it, the Rust DB tests are skipped |
| `OC_SMOKE_EMAIL`, `OC_SMOKE_PASSWORD` | `tests/smoke/stack.ts` | Sign in with this account instead of creating one |
| `OC_API_URL`, `OC_COLLAB_PORT` | `tests/smoke/stack.ts` | Targets when not given as arguments |
| `SERVER_CMD`, `OC_SECRET`, `OC_COLLAB_PORT` | `tests/collab/provider-client.ts` | Required by that client |
| `OC_COLLAB_SERVER_CMD` | `crates/oc-collab/tests/provider.rs` | Server command to test instead of the built binary |
| `OC_TEST_PG_BIN` | `oc-db` test cluster helper | Directory of Postgres binaries |
| `OC_SKIP_TS_VERIFY` | `oc-crdt`, `oc-editor-core` and `oc-render-svc` tests | When set, skips their TypeScript verification step |
| `OC_GOLDEN_CASE` | `oc-editor-core` golden tests, `packages/engine/test/editor-goldens.test.ts` | Run only this case |
| `OC_GOLDEN_TRACE`, `OC_GOLDEN_EL` | `oc-editor-core` golden tests | Debug tracing for a golden session |
| `OC_PS1_UPDATE` | `crates/oc-render/tests/ps1.rs` | `1` rewrites the pixel baselines |
| `OC_PS1_FILTER` | `packages/engine/test/ps1.test.ts` | Limit that test to matching cases |
| `OC_TEMPLATE_PHOTOS` | `packages/engine/scripts/bench-render.mts` | Directory of template photos for the benchmark |
| `OC_ENGINE_WASM` | `packages/engine/scripts/bench-wasm.mts` | Path to another `.wasm` build for the benchmark |

## Mobile shell

Read by `apps/mobile/capacitor.config.ts`.

| Name | Default | Purpose |
|---|---|---|
| `OC_APP_URL` | Not set | Your deployment's URL, which the native shell loads. When not set, the shell packages `apps/web/dist` |
| `OC_APP_ID` | `app.pinselo` | App identifier |
