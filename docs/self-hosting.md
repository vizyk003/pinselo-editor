# Self-hosting

This guide covers running Pinselo in production with the files in `deploy/`. For a local development setup, see [getting-started.md](getting-started.md). For every setting, see [configuration.md](configuration.md).

Pinselo is licensed under the AGPL-3.0. If you run a modified version as a network service, you must offer its source code to that service's users.

## What you run

`deploy/docker-compose.yml` defines these services. You build the images yourself from `deploy/Dockerfile`.

| Service | Image target | Command | Role |
|---|---|---|---|
| `db` | `postgres:17` | Default | Database. Volume `pg` |
| `migrate` | `runtime` | `oc-migrate` | Applies pending migrations, then exits |
| `api` | `runtime` | `oc-api` | HTTP API on port 4000 |
| `collab` | `runtime` | `oc-collab` | Real-time sync on port 4100 |
| `render` | `runtime` | `oc-render-svc` | Exports and thumbnails on port 4200 |
| `worker` | `runtime` | `oc-worker` | Email, webhooks, account export and erasure, social publishing, cleanup |
| `gateway` | `gateway` | Caddy | TLS, security headers, the web app and routing. The only service with published ports: 80 and 443. Volume `caddy` |

`migrate`, `api`, `collab`, `render` and `worker` share the volume `data`, mounted at `/data`. The `runtime` image sets `NODE_ENV=production`.

## Requirements

- A server with Docker, the Compose plugin and BuildKit. The Dockerfile uses BuildKit cache mounts.
- Network access during the build. The image build downloads fonts, emoji, graphics and colour profiles, and compiles the Rust services and the WebAssembly engine. The build is long: the CI job that builds these images allows up to 75 minutes.
- Ports 80 and 443 reachable from the internet.
- Two hostnames on **different registrable domains**, for example `design.example.com` and `example-usercontent.net`. The first serves the app and its cookies. The second serves user uploads only, so an uploaded file can never run with the app's cookies.
- An SMTP account for sign-in links, invites and password resets.

## Install

1. Point DNS for both hostnames at the server.

2. Create the settings file:

   ```bash
   cd deploy
   cp .env.example .env
   chmod 600 .env
   ```

3. Fill in `.env`. These are the values you need for the bundled database and local file storage:

   ```bash
   OC_APP_HOST=design.example.com
   OC_USERCONTENT_HOST=example-usercontent.net
   OC_ACME_EMAIL=ops@example.com
   OC_SECRET=<output of: openssl rand -base64 48>
   POSTGRES_PASSWORD=<output of: openssl rand -hex 32>
   OC_STAFF_EMAILS=you@example.com
   OC_SMTP_URL=smtps://user:pass@smtp.example.com:465
   OC_MAIL_FROM=Pinselo <no-reply@design.example.com>
   ```

   `POSTGRES_PASSWORD` is placed into a database URL without encoding, so use a value with no URL-special characters. Hex is the simplest choice.

4. **Delete the lines you are not using.** In particular, for local file storage, delete `OC_S3_BUCKET`, `OC_S3_REGION` and `OC_S3_ENDPOINT` from `.env`. The comment in `.env.example` says to leave `OC_S3_BUCKET` empty, but the services treat a variable that is set to an empty value as set. An empty `OC_S3_BUCKET` selects S3 with an empty bucket name. See [Empty and unset values](configuration.md#empty-and-unset-values).

5. Build, migrate and start:

   ```bash
   docker compose build
   docker compose run --rm migrate
   docker compose up -d
   ```

   Run `migrate` on its own first. In this Compose file, `migrate` also runs on every `up`, but `api` and the other services do not wait for it to finish.

6. Check the result:

   ```bash
   docker compose ps
   docker compose logs -f api
   curl https://design.example.com/api/status
   ```

   A healthy site reports `"overall":"operational"`. Then open `https://design.example.com` and sign up with an address from `OC_STAFF_EMAILS`. The staff console is at `/admin`. The API applies `OC_STAFF_EMAILS` each time it starts.

### Why a service refuses to start

With `NODE_ENV=production`, `api`, `collab`, `render` and `worker` exit with an error when:

- `OC_SECRET` is the development value or shorter than 32 characters
- `OC_WEB_ORIGIN` is not `https://`
- `OC_USERCONTENT_ORIGIN` is not a valid URL, or its hostname ends with the app's hostname
- only one of `OC_SUPABASE_URL` and `OC_SUPABASE_PUBLISHABLE_KEY` is set
- Supabase Auth is on and `OC_SUPABASE_URL` is not `https://` or `OC_SUPABASE_SECRET_KEY` is empty

In every environment, the services refuse a secret key in `OC_SUPABASE_PUBLISHABLE_KEY`. The database connection also refuses a transaction-mode pooler (port 6543 or `pgbouncer=true`).

## Templates

The migrations do not load the template catalogue, and the `runtime` image does not include the `oc-templates` binary. Until you seed, the catalogue is empty.

Seeding runs from a source checkout with the toolchain from [getting-started.md](getting-started.md). The checkout needs the same `OC_DATABASE_URL` and the same object store settings as your services:

```bash
pnpm --filter @oc/engine build
pnpm fonts:sync && pnpm emoji:sync
OC_DATABASE_URL=postgres://… OC_S3_BUCKET=… OC_S3_ENDPOINT=… AWS_ACCESS_KEY_ID=… AWS_SECRET_ACCESS_KEY=… pnpm seed
```

This works directly with an external database and an S3-compatible store. With the bundled stack it does not: `db` publishes no port, and local files live inside the `data` volume. This repository ships no seeding procedure for that setup. You would have to give the checkout access to both, for example with a temporary port mapping and `OC_DATA_DIR` pointing at the volume's contents.

## Database and migrations

Migrations are SQL files in `crates/oc-platform/oc-db/migrations`, copied into the `runtime` image. `oc-migrate` applies the ones that have not been recorded yet, by name, and it is safe to run repeatedly. It exits with status 1 if a migration fails. There are no down migrations, so to go back you restore a backup.

To use an external database instead of `db`, set `OC_DATABASE_URL` in `.env` and start the services without the bundled database:

```bash
docker compose up -d --no-deps migrate api collab render worker gateway
```

Pinselo needs a session-mode or direct connection, because the job queue and collaboration use `LISTEN/NOTIFY`. Each service opens up to `OC_DATABASE_POOL_MAX` connections (default 10), plus one listener. Check that your server's connection limit can take this.

For Supabase, follow [supabase.md](supabase.md). It covers the CA certificate, pool sizing and the lockdown that the migrator applies.

## File storage

| `OC_S3_BUCKET` | Objects stored in | Notes |
|---|---|---|
| Not set | `/data/objects` in the `data` volume | Back up this volume |
| Set | The bucket | A local cache is kept in `/data/s3-cache` |

For an S3-compatible store (AWS S3, R2, MinIO), set these in `.env`:

```bash
OC_S3_BUCKET=pinselo
OC_S3_REGION=auto
OC_S3_ENDPOINT=https://s3.example.com
AWS_ACCESS_KEY_ID=…
AWS_SECRET_ACCESS_KEY=…
```

Setting `OC_S3_ENDPOINT` also switches to path-style addressing. Omit it entirely for AWS S3. Keep the bucket private, since Pinselo serves files itself. If your store rejects SHA-256 upload checksums, set `OC_S3_CHECKSUMS=off`. Supabase Storage needs this.

Upload limits are `OC_MAX_UPLOAD_BYTES` (50 MiB) and `OC_MAX_MEDIA_BYTES` (1 GiB). If your store has its own upload limit, it must be at least as large.

Once a day, `oc-worker` removes unreferenced objects older than the grace period `OC_GC_GRACE_HOURS` (48). Set `OC_GC_DRY_RUN=1` to run this without deleting anything.

## Email

The worker delivers email through `OC_SMTP_URL`:

| Form | Default port | TLS |
|---|---|---|
| `smtp://user:pass@host:port` | 587 | STARTTLS when offered |
| `smtps://user:pass@host:port` | 465 | Implicit TLS |

Percent-encode special characters in the username and password. The sender is `OC_MAIL_FROM`, or `Pinselo <no-reply@<app host>>` when that is unset.

Do not run production without SMTP:

- If `OC_SMTP_URL` is **not set**, the worker marks each message as sent without sending it, and the dev mailbox is not available in production. Sign-in links and resets disappear without any error.
- If `OC_SMTP_URL` is **set but empty**, as in an unedited `.env.example`, delivery fails with an error and the job is retried.

## TLS and routing

Caddy obtains and renews certificates automatically, and uses `OC_ACME_EMAIL` for the ACME account. Certificates are stored in the `caddy` volume. Keep that volume between restarts so Caddy does not have to request them again. Both hostnames must resolve to the server and ports 80 and 443 must be open before the first request.

`deploy/Caddyfile` routes the app host as follows:

| Path | Goes to |
|---|---|
| `/api/*`, `/v1/*`, `/mcp`, `/.well-known/oauth-*`, `/site/*` | `api:4000` |
| `/collab*` | `collab:4100`, with the prefix removed |
| Anything else | The web app in `/srv/web` |

The uploads host serves only `/api/assets/<id>`, with `/poster` or `/waveform` variants. It removes cookies from the request and answers 404 to everything else.

The third site block serves customers' own domains for published websites. Caddy issues a certificate on demand, only for domains that `GET /api/sites/domain-check` reports as verified.

`/healthz` is not routed through the gateway, so use `/api/status` for external monitoring.

The API takes the client address from `X-Forwarded-For`, so keep port 4000 private. The Compose file does not publish it.

If you add stock media sources or embeds, note that the `Content-Security-Policy` in `deploy/Caddyfile` allows a fixed list of origins. When you use Supabase Auth, `OC_SUPABASE_URL` must also reach the `gateway` service, which it does through `env_file: .env`.

## Upgrades

```bash
git pull
cd deploy
docker compose build
docker compose run --rm migrate
docker compose up -d
```

Take a backup first. Migrations cannot be reversed. Fonts, graphics, emoji and colour profiles are part of the `runtime` image, so a rebuild picks up changes to them. Rerun `pnpm seed` if the template sources changed.

Old images stay on disk until you remove them, for example with `docker image prune -f`.

## Backups

Back up all of these:

| What | Why |
|---|---|
| The database | Accounts, designs, edit history and job queue |
| Object storage: the `data` volume or the bucket | Uploads, exports and thumbnails |
| `deploy/.env` | Contains `OC_SECRET`, which signs sessions and is the source of the key that encrypts connected social accounts' tokens. Without the same secret, those tokens cannot be decrypted |

With the bundled stack, from `deploy/`:

```bash
mkdir -p backups
docker compose exec -T db pg_dump -U oc --format=custom opencanvas > backups/db-$(date -u +%F).dump
docker compose cp api:/data backups/data-$(date -u +%F)
```

To check a dump, restore it into a throwaway container:

```bash
docker run -d --name restore-test -e POSTGRES_PASSWORD=test postgres:17
docker exec -i restore-test sh -c 'until pg_isready -q; do sleep 1; done; pg_restore -U postgres -d postgres --no-owner' < backups/db-<date>.dump
docker rm -f restore-test
```

Keep a copy off the server. `deploy/oracle/backup.sh` is an example of a complete script, described below. It is written for Supabase, so adapt it to your setup.

## Worked example: one small server with Supabase

`deploy/oracle/` and [deploy-oracle.md](deploy-oracle.md) show one concrete deployment: a single Oracle Cloud ARM server, with Supabase for the database and files. It is an example, not a requirement, and the same approach works on other hosts. Read deploy-oracle.md for the step-by-step version.

| Topic | `deploy/` | `deploy/oracle/` |
|---|---|---|
| Compose file | `docker-compose.yml` | `compose.yml` |
| Database | Bundled `db` | Supabase, with a mounted `supabase-ca.crt` |
| Files | Volume or any S3 store | Supabase Storage |
| Hostnames | App and separate uploads host | App host only; uploads share it |
| Images | Built by `docker compose build` | Pulled from a registry or built on the server |
| Migrations | Run by you first, then on every `up` | Services wait for `migrate` to complete |
| Connections | Default pool | `OC_DATABASE_POOL_MAX=2` to fit a small pool |

The scripts:

- **`update.sh`** pulls the images, stops the services so the migrator can get a connection, then starts everything. `./update.sh web` replaces only the gateway.
- **`build-on-server.sh`** sends the committed `HEAD` to the server over SSH and builds there. `OC_SERVER=user@host` is required. With `--deploy`, it updates `OC_RUNTIME_IMAGE` and `OC_GATEWAY_IMAGE` by replacing existing lines in `.env`, so add those lines first.
- **`backup.sh`** dumps the `public` schema with `pg_dump`, keeps the last seven dumps, and syncs the file bucket to `backups/files`. If `OC_BACKUP_S3_BUCKET` is set, it also copies everything to another S3-compatible bucket. It reads its settings from `.env` and expects `supabase-ca.crt` next to it.

Schedule the backup with cron, for example:

```bash
17 3 * * * cd /path/to/deployment && ./backup.sh >> backups/backup.log 2>&1
```

To reuse this example:

- `compose.yml` falls back to image names from the original repository. Set `OC_RUNTIME_IMAGE` and `OC_GATEWAY_IMAGE` in `.env` to your own images. `.github/workflows/images.yml` publishes ARM64 images to `ghcr.io/<owner>/<repo>-runtime` and `-gateway` for your fork.
- Keep Supabase Auth off without a separate uploads domain. It keeps tokens in browser storage on the host that would also serve uploads.
- `.github/workflows/uptime.yml` checks the original project's site and runs only for its owner. Write your own check against `/api/status`.

## Troubleshooting

Start with `docker compose ps` and `docker compose logs <service>`.

| Symptom | Likely cause |
|---|---|
| A service keeps restarting | A refused setting. The log names the variable; see [Why a service refuses to start](#why-a-service-refuses-to-start) |
| API errors about missing tables just after first start | `api` started before `migrate` finished. Run `docker compose run --rm migrate`, then `docker compose restart api collab render worker` |
| Upload or thumbnail failures mentioning S3 | `OC_S3_BUCKET` is set but empty, or the credentials or endpoint are wrong |
| No sign-in emails | `OC_SMTP_URL` is missing or wrong. Check `docker compose logs worker` |
| Certificate errors | DNS does not point at the server yet, or ports 80 or 443 are blocked. Check `docker compose logs gateway` |
| `Invalid URL: https://` in a service log | `OC_USERCONTENT_HOST` is empty, so `OC_USERCONTENT_ORIGIN` became `https://` |
| The template list is empty | The catalogue has not been seeded; see [Templates](#templates) |
