# Running Pinselo on Supabase

Pinselo can use Supabase in two independent ways:

1. **Database.** Pinselo stores everything in a Supabase Postgres database instead of its own. Users notice nothing.
2. **Auth (optional).** Supabase Auth handles sign-up, sign-in, OAuth providers, password resets and two-factor authentication. It is on only when `OC_SUPABASE_URL` **and** `OC_SUPABASE_PUBLISHABLE_KEY` are set. Leave them empty and Pinselo keeps its built-in sign-in (passwords, magic links, passkeys, Google/Apple/SSO). Self-hosters never need Supabase.

You can use either one without the other. Supabase Storage can also hold uploads (section 4).

In Supabase Auth mode, users sign in through Supabase in the browser. The app then exchanges that sign-in once for its normal Pinselo session cookie. Sharing, collaboration, the public API and personal access tokens all work exactly as before.

Use a **dedicated Supabase project** for Pinselo. Its migrations create tables in the `public` schema and lock that schema down for the Data API (section 2), which would also affect other apps' tables there.

Placeholders below: `<ref>` is your project reference, `<region>` its region (for example `eu-central-1`), and `<app host>` the host Pinselo is served from.

## 1. Before you start

- A Supabase project (any plan).
- The database password (Dashboard → Database Settings).
- Node 22+ and pnpm, or the Docker setup in `deploy/`.

## 2. Database

### Choose a connection

| Connection | URL | Use it when |
|---|---|---|
| Direct | `postgres://postgres:<password>@db.<ref>.supabase.co:5432/postgres` | Your servers have IPv6, or you bought the IPv4 add-on. Supabase recommends this for long-running servers. |
| Shared pooler, **session** mode | `postgres://postgres.<ref>:<password>@aws-0-<region>.pooler.supabase.com:5432/postgres` | IPv4-only hosts. Note the user `postgres.<ref>`. |
| Transaction pooler (port **6543**) | — | **Never.** Pinselo refuses to start with it: the job queue and real-time collaboration use `LISTEN/NOTIFY`, and transactions span several statements. |

Copy the URL from Dashboard → Connect, and set it as `OC_DATABASE_URL`. Percent-encode special characters in the password.

### TLS: the Supabase CA certificate

Supabase's database certificates are signed by Supabase's own root CA, which is not in the operating system's trust store.

1. Download the certificate: Dashboard → Database Settings → SSL Configuration → Download certificate (`prod-ca-2021.crt`).
2. Point `OC_DATABASE_SSL_CA` at it. The value can be an absolute path (Docker: mount it, see `deploy/docker-compose.yml`), a path relative to the repository root, or the PEM text itself (literal `\n` sequences are accepted).
3. Optional: turn on **Enforce SSL** on the same page.

With the CA set, Pinselo verifies the certificate chain **and** the host name (verify-full), whatever `sslmode` the URL has, `uselibpqcompat=true` included. Without it, a Supabase URL is refused at startup with a message that points here. Every Supabase project's certificate is signed by the same CA, so anything weaker than verify-full can be intercepted: for a Supabase host, Pinselo refuses `sslmode=no-verify`, `sslmode=disable`, `ssl=0`, and `uselibpqcompat=true` with `sslmode=prefer`, `require` or `verify-ca`, in production, and warns about them in development. The same rules apply when the host comes from `PGHOST` because the URL leaves it empty.

### Pool size

In session mode, every client connection holds a server connection, up to the project's **Pool Size** (Dashboard → Database → Connection pooling; small on the Free and Nano tiers). Each Pinselo service opens up to `OC_DATABASE_POOL_MAX` connections (default 10) plus one listening connection:

```
api (pool + 1) + collab (pool + 1) + worker (pool + 1) + render (pool + 1) + migrate
```

On small projects set `OC_DATABASE_POOL_MAX=4`, or raise the Pool Size, or use a direct connection.

### Migrate

```sh
OC_DATABASE_URL='postgres://…' OC_DATABASE_SSL_CA=/path/to/prod-ca-2021.crt pnpm db:migrate
```

With Docker, the `migrate` service runs this on every `docker compose up`.

> **The migrator locks Supabase down.** `oc-migrate` (`crates/oc-platform/oc-db`; `pnpm db:migrate` builds and runs it) runs the Data API lockdown (`crates/oc-platform/oc-db/lockdown.sql`, verbatim) at three points, prints the exposure report, and exits 1 when a migration failed or, in production, when anything is still exposed. A Supabase database migrated by an `oc-migrate` built before the lockdown was ported (before 2026-10-23) is exposed: run the migrator once, since every run re-applies the lockdown.

After every run, also a failed one, `pnpm db:migrate` prints the Data API exposure report (below). It exits non-zero when a migration failed and, in production (`NODE_ENV=production`), when the API roles can still reach anything.

### The Data API (PostgREST)

Supabase serves the `public` schema over REST and GraphQL to anyone holding the publishable (anon) key, and by default grants the `anon` and `authenticated` roles every new table. Pinselo never uses the Data API: the app connects to Postgres directly, as the owner of its tables.

- **Recommended:** turn the Data API off (Dashboard → Integrations → Data API → Enable Data API).
- **Either way**, the migrator locks the `public` schema down. It runs `crates/oc-platform/oc-db/lockdown.sql` in the same transaction that creates `schema_migrations`, inside every migration's transaction before it commits, and once more at the end of every run, also a failed one. So no state where the Data API can read Pinselo data is ever committed, not even half-way through a first migration. For every object in `public` that the migrating role owns, the lockdown:
  - revokes all privileges of `PUBLIC`, `anon`, `authenticated` and `service_role` on tables, views, materialized views, foreign tables and sequences, column grants included;
  - revokes `EXECUTE` on functions and procedures, `SECURITY DEFINER` ones included (Postgres lets `PUBLIC` execute every new function by default);
  - turns on row-level security without policies, so the API roles see nothing;
  - stops the default grants on objects the migrating role creates later. For functions, this is a role-wide default: functions it creates in other schemas are no longer executable by `PUBLIC` either, one more reason to use a dedicated project.

  The table owner is not affected by RLS, so the app works unchanged. Functions that belong to extensions are left alone. Objects in `public` owned by another role are left as they are and named in a warning; the exposure report shows whether they are reachable. On plain Postgres the lockdown does nothing.
- Databases migrated before the lockdown moved to `lockdown.sql` keep the `oc_supabase_lockdown()` function that the first version of `0028_supabase_lockdown.sql` created. Nothing calls it any more, and only its owner may execute it.

Verify:

```sh
curl -s -o /dev/null -w '%{http_code}\n' "https://<ref>.supabase.co/rest/v1/users?select=id" -H "apikey: <publishable key>"
```

This must print 401, 403 or 404, never 200. `pnpm db:migrate` and the API (at startup) both check and log `Supabase exposure check: OK`. The check covers table, column, sequence and function privileges of `anon` and `authenticated`, including those they get through `PUBLIC`. If the API roles can still reach Pinselo objects, the API warns in development and refuses to start in production. It always refuses to start when RLS would hide rows from the app itself, which happens only when it connects as a role that does not own the tables.

## 3. Auth (optional)

Set these, and restart every service:

```sh
OC_SUPABASE_URL=https://<ref>.supabase.co
OC_SUPABASE_PUBLISHABLE_KEY=sb_publishable_…
OC_SUPABASE_SECRET_KEY=sb_secret_…
```

### API keys

- **Publishable key** (`sb_publishable_…`, or the legacy `anon` JWT): goes to `OC_SUPABASE_PUBLISHABLE_KEY`. It is public; browsers receive it. Pinselo refuses to start if this variable holds a secret or `service_role` key.
- **Secret key** (`sb_secret_…`, or the legacy `service_role` JWT): server-only, used solely to delete the Supabase user when someone deletes their account. Required in production.

### JWT signing keys

Migrate the project to asymmetric signing keys (ES256) under Dashboard → Project Settings → JWT Keys. Pinselo then verifies tokens against the project's public JWKS, and `OC_SUPABASE_JWT_SECRET` stays empty. Set `OC_SUPABASE_JWT_SECRET` only while the project still signs with the legacy HS256 secret. Every token is also checked online against Supabase when it is exchanged, so signed-out sessions are refused either way.

`OC_SUPABASE_JWT_ISSUER` is needed only with a custom auth domain (the default is `https://<ref>.supabase.co/auth/v1`).

### URL configuration

Dashboard → Authentication → URL Configuration:

- **Site URL:** the value of `OC_WEB_ORIGIN`, for example `https://<app host>`.
- **Redirect URLs:**
  - `https://<app host>/auth/callback**`
  - `http://localhost:3000/auth/callback**` (local development)

The `**` matters: the page to return to travels in the query string (`?next=…`). If a redirect does not match, Supabase falls back to the Site URL and the user lands on the home page instead of where they started.

### Email

Dashboard → Authentication → Providers → Email:

- **Confirm email: required.** Pinselo links existing accounts by email address, and only Supabase's confirmation email proves that someone owns an address. With it off, Supabase marks every address confirmed without sending anything. Pinselo reads this setting from the project and then refuses to link any existing account by email (`link_conflict` / `email_unverifiable`), creates new accounts as unverified and ignores email changes. If it can't read the setting, linking waits (`supabase_unavailable`) instead of guessing.
- **Secure password change: required.** Changing the password then needs a recent sign-in or a reauthentication code, so someone holding a stolen session can't lock the owner out.
- **Secure email change:** keep it on (both the old and the new address confirm the change).
- Set the minimum password length to 10, and turn on the leaked-password check if your plan has it.
- **Custom SMTP** (Authentication → Emails → SMTP Settings): Supabase's built-in mailer is heavily rate-limited and only delivers to your team's addresses. Configure your own provider before inviting users.
- **Links that open in another browser.** By default Supabase emails PKCE links, which only work in the browser that asked for them. To make confirmation, magic-link and recovery emails work anywhere, change the templates to link to `{{ .SiteURL }}/auth/callback?token_hash={{ .TokenHash }}&type=<type>&next={{ .RedirectTo }}`, with `<type>` = `signup`, `magiclink`, `recovery` or `email_change` for the respective template. Such a link isn't tied to the browser that asked for it, so Pinselo first asks "Continue as <email>?" before it signs that browser in.

### OAuth providers

In each provider's developer console, the redirect URI is `https://<ref>.supabase.co/auth/v1/callback`. Turn the provider on under Authentication → Providers. It appears on the Pinselo sign-in page automatically within 5 minutes.

#### Google (Gmail) sign-in

1. In [Google Cloud Console](https://console.cloud.google.com/) → APIs & Services → **OAuth consent screen**: user type *External*, app name and support email, scopes `openid`, `.../auth/userinfo.email` and `.../auth/userinfo.profile` (no sensitive scopes, so no Google review). While it is in *Testing*, add the accounts that may sign in as test users. Publish it for everyone else.
2. **Credentials** → Create credentials → **OAuth client ID** → *Web application*:
   - Authorised JavaScript origins: your app origin (e.g. `http://localhost:3900`, `https://<app host>`).
   - Authorised redirect URIs: `https://<ref>.supabase.co/auth/v1/callback`. Google redirects to Supabase, not to Pinselo.
3. Supabase Dashboard → Authentication → Providers → **Google**: turn it on and paste the client ID and secret. Leave "Skip nonce checks" off.
4. Authentication → URL Configuration: Site URL = your app origin; add `http://localhost:3900/auth/callback**` and `https://<app host>/auth/callback**` to the redirect allow-list.

"Continue with Google" then comes first on the sign-in and sign-up pages (within 5 minutes, or restart the API). It always opens Google's account chooser, so someone with several Gmail accounts picks one. A new Google user gets the usual name, birth year and terms step. Google addresses arrive confirmed, so a Google sign-in links to an existing Pinselo account with the same verified address. An unverified account with that address is set aside as described below.

### Two-factor authentication

TOTP is available by default. Users enrol under Settings → Security in Pinselo. Supabase has no recovery codes, so Pinselo recommends adding a second authenticator.

An account that already had Pinselo's built-in two-factor before the switch keeps it. Pinselo asks for that code on every sign-in, even after a Supabase code: Supabase lets a session that has only the password add its first factor, so a Supabase code alone doesn't prove it's the account's owner. Users turn the built-in code off under Settings → Security once they have signed in.

### Browser sessions and published sites

supabase-js keeps the Supabase session, including its long-lived refresh token, in the browser's `localStorage` for the app origin. Any script that runs on that origin can read it. Published sites (`/site/<slug>`) are served from the app origin, and their Content-Security-Policy allows inline scripts (`script-src 'unsafe-inline'`). Pinselo generates those pages itself, so this matters only if user-made content ever slipped into them as script, but it is the weak spot:

- Set `OC_USERCONTENT_ORIGIN` to a different registrable domain (see `deploy/`), so uploads never share the app's origin.
- Prefer custom domains for published sites: on their own host they can't reach the app's storage.

## 4. Storage (optional)

Supabase Storage works through its S3-compatible endpoint:

1. Create a **private** bucket (Dashboard → Storage), for example `opencanvas`. Never make it public: Pinselo serves files itself, from its user-content origin.
2. Storage → S3 Configuration: note the region and create an access key pair. These keys bypass RLS and can reach every bucket; keep them server-side.
3. Set:

```sh
OC_S3_BUCKET=opencanvas
OC_S3_REGION=<region>
OC_S3_ENDPOINT=https://<ref>.storage.supabase.co/storage/v1/s3
AWS_ACCESS_KEY_ID=…
AWS_SECRET_ACCESS_KEY=…
OC_S3_CHECKSUMS=off
```

`OC_S3_CHECKSUMS=off` stops Pinselo from sending SHA-256 upload checksums, which Supabase's S3 endpoint does not list as supported. Objects are named by their SHA-256, but Pinselo does not hash them again when it reads them, so with checksums off nothing checks end to end that the stored bytes match. TLS still protects the transfer.

Supabase's global upload limit (Storage settings; 50 MB on Free) must be at least `OC_MAX_UPLOAD_BYTES`, and at least `OC_MAX_MEDIA_BYTES` if you allow video uploads. Raise it, or lower the Pinselo limits.

## 5. Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `OC_DATABASE_URL` | `postgres://localhost:5432/opencanvas` | Session pooler or direct connection. Port 6543 and `pgbouncer=true` are refused. |
| `OC_DATABASE_SSL_CA` | (empty) | The Supabase CA: PEM text, an absolute path, or a path relative to the repository root. Enables verify-full. |
| `OC_DATABASE_POOL_MAX` | `10` | Connections per service. |
| `OC_SUPABASE_URL` | (empty) | `https://<ref>.supabase.co`. Empty = built-in sign-in. |
| `OC_SUPABASE_PUBLISHABLE_KEY` | (empty) | Publishable (anon) key. Sent to browsers. |
| `OC_SUPABASE_SECRET_KEY` | (empty) | Secret key, server-only. Required in production with Supabase Auth. |
| `OC_SUPABASE_JWT_SECRET` | (empty) | Legacy HS256 secret, only while the project still uses it. |
| `OC_SUPABASE_JWT_ISSUER` | `${OC_SUPABASE_URL}/auth/v1` | Custom auth domains only. |
| `OC_S3_CHECKSUMS` | `on` | `off` for Supabase Storage. |

In production, Pinselo refuses to start when only one of `OC_SUPABASE_URL` and `OC_SUPABASE_PUBLISHABLE_KEY` is set, when `OC_SUPABASE_URL` is not `https://`, or when Supabase Auth is on without `OC_SUPABASE_SECRET_KEY`. Error messages name the variables, never their values.

## 6. Moving an existing deployment to Supabase Auth

- **Linking.** The first time someone signs in through Supabase with the same email as an existing, **verified** Pinselo account, the two are linked, but only when Supabase reports the email confirmed and the project has **Confirm email** on (section 3). Until then the sign-in is refused with an explanation (`link_conflict` / `unconfirmed_supabase`, or `email_unverifiable` when the project auto-confirms), so nobody can take over an account by signing up with its address. The link is made only after every code: the Supabase one when the user has two-step verification there, and the built-in one when the Pinselo account has it.
- **Unverified accounts are never reused.** Whoever created an account whose address was never verified didn't prove they own the inbox, so when a Supabase user confirms that address, Pinselo sets the old account aside in one step: it is disabled, its address is replaced by a non-routable placeholder, and its sessions, personal access tokens, webhooks, passkeys, Google/Apple/SSO links, pending email links, team memberships, shares, access requests, published sites, social-account credentials and scheduled posts are revoked or switched off. The confirmed person then gets a new, empty account through the usual name, birth year and terms step. The old account's designs and other data stay in the database for support (audited as `auth.supabase_squat_tombstoned`, with both ids); if it turns out to be the same person's, support can move them over. An unverified account that staff had disabled is set aside the same way, so it can't block the address's owner. A set-aside account can't be enabled again from the admin page. Requests it had in flight (a site publish, a social-account connection, a team invite) are refused once it is set aside, and a disabled or deleted owner's sites are offline. With two-step verification on in Supabase, nothing happens before the code.
- **Turning Confirm email on later** doesn't re-prove addresses that Supabase marked confirmed while it was off: those users keep an unproven `email_confirmed_at`. Accounts created then are unverified here, and email-based linking was refused for them, so nothing was linked on that basis.
- **What stops working:** built-in passwords, magic links, passkeys and built-in Google/Apple/SSO buttons. Users sign in with Supabase instead (set up the same OAuth providers there). Built-in two-factor codes keep working for linked accounts, and are asked on every sign-in, until the user turns them off.
- **Deleting an account** removes its link to the Supabase user in either mode. With Supabase Auth off, Pinselo can't delete the Supabase user itself: delete it in the dashboard, or it can sign up again later as a new account.
- **Switching back** (unset `OC_SUPABASE_URL`): linked users use "Forgot password" to set a built-in password.

## 7. For contributors

- New migrations (`crates/oc-platform/oc-db/migrations`) need nothing extra for Supabase, whatever they create (tables, views, sequences, functions). The migrator (`oc_db::migrate`, `pnpm db:migrate`) runs `crates/oc-platform/oc-db/lockdown.sql` verbatim right after creating `schema_migrations`, inside every migration's transaction before COMMIT, and at the end of every run, also a failed one. That covers later migrations and lower-numbered ones merged after others ran (migrations are applied by file name). Change the lockdown only in that file, keep it idempotent, and remember that databases that already ran every migration get only the new file text. `crates/oc-platform/oc-db/tests/lockdown.rs` checks it on a throwaway cluster with a non-superuser migrating role.
- If you apply migrations by hand (for example with `supabase db query --linked -f`), mirror the migrator: `BEGIN`, then for each file its SQL, its `INSERT INTO schema_migrations(name)` and the text of `lockdown.sql`, then `COMMIT`.
- Every Postgres connection goes through `oc_db::connect_options()` (the pool, the listener, the migrator and the tests). It applies `OC_DATABASE_SSL_CA` and the TLS mode; never build connect options from the URL alone.
- Long-lived `LISTEN` connections use `oc_db::create_listener()`. It reconnects and listens again after a dropped connection, gives up on a connect, `LISTEN` or ping that hangs, and calls `onReconnect` once it listens again: notifications sent in between are lost, so catch up there. The job queue wakes every worker loop; collaboration closes every client socket with code 4403, so editors fetch a fresh ticket (their role is checked again) and resync. A revoked or downgraded collaborator's socket is closed the same way.

## 8. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `self-signed certificate in certificate chain` | `OC_DATABASE_SSL_CA` is not set (or the URL's `sslrootcert` points at the wrong file). Download the Supabase CA (section 2). |
| `OC_DATABASE_URL points at a transaction-mode pooler` | You used port 6543. Use the session pooler on port 5432 or a direct connection. |
| `Max client connections reached` / `too many clients` | The project's Pool Size is smaller than all services' pools together. Lower `OC_DATABASE_POOL_MAX` or raise the Pool Size. |
| "Open the link in the same browser…" after clicking an email link | PKCE links only work in the browser that requested them. Switch the email templates to `token_hash` links (section 3). |
| Users land on the home page after OAuth or an email link | The redirect URL is not on Supabase's allow-list, so Supabase used the Site URL. Add `https://<app host>/auth/callback**`. |
| `link_conflict` with `unconfirmed_supabase` | The user has not confirmed their Supabase email yet. They follow the confirmation email, then sign in again. |
| `link_conflict` with `linked_elsewhere` | The Pinselo account is already linked to a different Supabase user. |
| `link_conflict` with `changed_during_signin` | The account changed while the user was signing in (for example its email changed, or it was linked or deleted elsewhere). Signing in again resolves it. |
| `link_conflict` with `email_unverifiable` | The project auto-confirms emails, so Pinselo won't link existing accounts by email. Turn **Confirm email** on (section 3); the change applies within 5 minutes. |
| `supabase_unavailable` when signing in with an existing account's address | Pinselo couldn't read the project's Auth settings to check **Confirm email**. It retries after 30 seconds; check that `OC_SUPABASE_URL` and the publishable key are right. |
| API logs `Supabase Data API roles can reach Pinselo objects` or `Row-level security is off on` | Run `pnpm db:migrate` as the role that owns the tables: every run re-applies the lockdown. Objects you created by hand in `public` are covered by the next run too. |
| `pnpm db:migrate` warns `Supabase lockdown: … object(s) in schema public belong to roles that … cannot act for` | Those objects belong to another role, so the lockdown left them alone. Migrate as their owner, or revoke their grants (`REVOKE ALL … FROM PUBLIC, anon, authenticated, service_role`) and enable RLS by hand. In production `db:migrate` exits non-zero while the exposure report still lists them. |
| API refuses to start: `Row-level security would hide rows from Pinselo` | The app connects as a role that does not own the tables. Use the role that ran the migrations (`postgres` on Supabase). |
