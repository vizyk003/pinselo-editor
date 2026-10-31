# oc-api route port status

> **The TypeScript API (`services/api`) was deleted in plan 13 stage 4 (2026-10-27; the last commit that has it is `aa18983`).**
> The goldens it wrote stay in `tests/golden` as frozen records of its behaviour, and this file records how the port
> was verified. New behaviour is written and tested in Rust only.

The Rust port of `services/api` (ADR-0001, strangler migration). Every TS route file maps to one Rust module;
`src/router.rs` already merges every module (unported ones are empty stubs that list their endpoints), so a
route group can be ported without touching any shared file.

Status legend:

* **ported + parity** — every endpoint ported; an HTTP transcript of the TS app (`tests/golden/api/<group>.json`)
  replays byte-for-byte (after normalization) against the Rust router in `cargo test -p oc-api --test parity`.
* **ported, no parity yet** — code ported, no transcript yet.
* **helpers only** — the functions other modules import are ported; the endpoints are not.
* **not started** — empty stub.
* **blocked on X** — needs a crate or library that is not ready.

| TS file (`services/api/src/…`) | Rust module (`crates/oc-api/src/…`) | Endpoints | Status | Parity group(s) |
|---|---|---:|---|---|
| `main.ts` (inline `/healthz`, `/api/config`, hooks, error handler) | `routes/health.rs`, `router.rs`, `http/*` | 2 | ported + parity | `framework` |
| `routes/account.ts` (+ `auth.ts`, `login.ts`, `accountLock.ts`) | `routes/account.rs`, `auth.rs`, `login.rs`, `webauthn.rs`, `jwt.rs` | 32 | ported + parity (Supabase mode: built-in routes answer 404 `supabase_auth` before their rate limit; providers, account `auth`, magic `next`, revoke-others and deletion with a Supabase token) | `account`, `oidc`, `supabase`, `supabase-legacy` |
| `routes/designs.ts` (+ `docAccess.ts`) | `routes/designs.rs`, `doc_access.rs` | 19 | ported + parity (sheet surface needs the default `sheet` feature; batch 2: `POST /api/designs/bulk`, folder moves, sort/owner/deep filters, trash holds, copies next to the original; `projects.rs`, `share_access.rs`) | `designs`, `projects` |
| `routes/assets.ts` (+ `media.ts`, `hashMatch.ts`) | `routes/assets.rs`, `media.rs`, `hash_match.rs` | 18 | ported + parity (batch 2 uploads library and trash: `uploads.rs`) | `assets`, `uploads` |
| `routes/media.ts` | `routes/media.rs` | 6 | ported + parity | `assets` |
| `routes/teams.ts` | `routes/teams.rs` | 21 | ported + parity (incl. batch 2 member groups) | `teams`, `sharing` |
| `routes/brand.ts` | `routes/brand.rs` | 7 | ported + parity | `teams` |
| `routes/docs.ts` | `routes/docs.rs` (Word → HTML: `oc-import::docx`, a port of mammoth 1.12) | 2 | ported + parity (page PNGs through the render service with the 200-entry version-keyed cache; Word import incl. picture ingest) | `docs` |
| `routes/safety.ts` | `routes/safety.rs` | 9 | ported + parity (the `OC_STAFF_EMAILS` start-up step is `bootstrap_staff`, called from `main.rs`, not covered by a transcript) | `safety` |
| `routes/billing.ts` | `routes/billing.rs` (+ `billing.rs`: plans, entitlements, local + Stripe providers, `stripe()`, webhook signature, `applyStripeEvent`, `register_stripe_event_hook`) | 8 | ported + parity (the Stripe REST provider needs api.stripe.com and is not covered by a transcript) | `billing` |
| `routes/brandhub.ts` | `routes/brandhub.rs` (+ `brandhub.rs`, incl. `templateUpdateOps`/`applyServerOps`) | 17 | ported + parity | `brandhub` |
| `routes/content.ts` (+ `stock.ts`) | `routes/content.rs`, `stock.rs` | 18 | ported + parity (`families_for` exported for misc, `is_public_host` for embeds/v1) | `content` |
| `routes/dev.ts` | `routes/dev.rs` | 1 | ported + parity | `dev`, `dev-smtp` |
| `routes/embeds.ts` | `routes/embeds.rs` | 3 | ported + parity (oEmbed/thumbnail fetches go to fixed third-party hosts; only providers without them are covered; batch 2 outbound oEmbed and public thumbnail: `share_links.rs`) | `embeds`, `sharing` |
| `routes/forms.ts` | `routes/forms.rs` | 3 | ported + parity | `forms` |
| `routes/graphics.ts` (+ `graphics.ts`) | `routes/graphics.rs`, `graphics.rs` | 4 | ported + parity | `graphics` |
| `routes/growth.ts` | `routes/growth.rs` | 12 | ported + parity | `growth` |
| `routes/imports.ts` (+ `importers/pdf.ts`, `importers/pptx.ts`) | `routes/imports.rs`; importers in `crates/oc-import` (`importers/*` re-export them) | 3 | ported + parity (the `import` worker starts from `main.rs` and, idempotently, on the first import request; importer output parity: `cargo test -p oc-import`, goldens `tests/golden/import/`) | `imports`, `docs` (doc → design) |
| `routes/misc.ts` | `routes/misc.rs` (fonts via `oc-gfonts`) | 22 | ported + parity (fetching unpinned Google faces/previews needs the network and is not covered) | `misc` |
| `routes/openapi.ts` (data for v1) | `routes/openapi.rs` | 0 | ported + parity (`GET /v1/openapi.json`) | `v1` |
| `routes/print.ts` (+ `print/*`) | `routes/print.rs`, `print/*` | 22 | ported + parity (the `print-fulfil` worker: `start_worker` in `main.rs`, the same `oc_jobs::work` loop as TS; without it, drained in-process after each enqueue; Stripe hook registered with billing) | `print` |
| `routes/printProfiles.ts` (+ `print` of the export routes in designs/media/v1) | `routes/print_profiles.rs` (bundled registry via `oc-color`) | 4 | ported + parity (ICC inspection and gamut maths stay in the render service) | `print-profiles` |
| `routes/search.ts` (+ `search.ts`) | `routes/search.rs`, `search.rs` | 2 | ported + parity | `search` |
| `routes/sheets.ts` | `routes/sheets.rs` | 1 | ported + parity (needs the default `sheet` feature; 501 without it) | `sheets` |
| `routes/sites.ts` (+ `siteSeo.ts`, `insights.ts`) | `routes/sites.rs`, `site_seo.rs`, `insights.rs` | 17 | ported + parity (incl. `customDomainHook` in the global middleware and the plugin's urlencoded body parser); site builds call the render service over HTTP | `sites` |
| `routes/social.ts` (+ `social/*`) | `routes/social.rs` (on `oc-social`) | 13 | ported + parity | `social` |
| `routes/v1.ts` | `routes/v1.rs` | 20 | ported + parity (`coverCropFor` inlined; autofill URL imports need the network and are covered only up to the private-host guard) | `v1` |
| `routes/sharing.ts` (+ `shareLinks.ts`, `shareAccess.ts`): batch 2 sharing (links, template links, access) | `routes/sharing.rs`, `share_links.rs`, `share_access.rs` | 12 | ported + parity | `sharing` |
| `routes/supabase.ts` (+ `supabase.ts`): Supabase Auth bridge | `routes/supabase.rs`, `supabase.rs` | 2 | ported + parity (token checks reproduce jose 6.2.12; the fake project serves a JWKS, RS256/EdDSA/HS256 tokens are signed by the scenario, the ES256 one is fixed in `tools/golden/api/supabase-keys.ts`) | `supabase`, `supabase-legacy` |
| `routes/oauth.ts` (+ `oauth.ts`, `bearer.ts`): OAuth for the AI connector | `routes/oauth.rs`, `oauth.rs`, `bearer.rs` | 10 | ported + parity (discovery + CORS preflights, DCR, client ID metadata documents through the `OC_OAUTH_FAKE_BASE` seam, consent API, PKCE code exchange, refresh rotation, revocation, connected apps; v1 refuses `oc_oat_` tokens: audience) | `oauth` |
| `mcp/index.ts`, `mcp/files.ts` (+ `mcp/*`): the MCP endpoint | `mcp/*` (transport + JSON-RPC in `mcp/mod.rs`, tools, edit, view, layout, live, files, guide) | 4 | ported + parity (the MCP SDK's streamable-HTTP wire protocol, stateless JSON mode, implemented directly; `tools.json` is the SDK's `tools/list`; search_media/search_fonts and URL/stock image imports are not covered by a transcript) | `mcp` |
| **Total** | | **303** | **303 ported + parity** | 33 transcripts |

Remaining (recounted 2026-10-22 on the merge of rust-core, the AI connector and integrate/oct15; the per-file
counts equal the `.get/.post/.put/.patch/.delete('/…')` registrations in `services/api/src`):
* The AI connector (OAuth + `/mcp`, 14 endpoints) is ported (`oauth`, `mcp` transcripts); the built-in AI
  (`routes/ai.ts`) was removed, and so are its Rust counterparts (`routes/ai.rs`, `anthropic.rs`, the `ai*` groups).
* The 28 endpoints of plan-11 batch 2 and Supabase are ported (2026-10-23): new groups `sharing`, `projects`,
  `uploads`, `supabase` (JWKS + legacy JWT secret + Auth Admin key) and `supabase-legacy` (no JWT secret, no secret
  key, an auto-confirming project). Every group passes `cargo test -p oc-api --test parity`.
* The `sites` scenario turns recordVisit's random 1 % prune of old visitor hashes into a no-op (a rule in its seed):
  it compares with the database's real date, so with the app clock pinned to 2025 it deleted the scenario's rows
  whenever the draw fired, and TS (seeded `Math.random`) and Rust (OS randomness) could not agree.

## Adding or changing a route

The parity transcripts can no longer be regenerated (their harness, `tools/golden/api`, ran the TS app). A new or
changed endpoint gets Rust tests instead: a scenario in `tests/` against the router and a throwaway database
(`oc_db::testing::TestDb`). A deliberate change to a recorded behaviour edits the affected transcript steps by hand,
with the reason in the commit message. The steps below are how every existing group was ported.

1. **Rust:** fill in `src/routes/<file>.rs` (its `routes()` and `LIMITS` are already merged by `router.rs`). Every
   handler takes `State<AppState>`, then path extractors, then `Req` **last** (it applies the route's rate limit
   and parses the body in Fastify's order), and starts with `require_user(&s, &req).await?` where the TS plugin
   has `addHook('preHandler', requireUser)`. Validate with `crate::zod` (exact zod issues), query with
   `crate::pg` (node-postgres row shapes; bind user strings as text and cast `$n::uuid` where the column is a
   uuid, like the TS SQL relies on), reply with `Reply`. Per-route `config.rateLimit` goes into `LIMITS`;
   module-level TS state (`const cache = new Map()`) into `state.local::<T>()`; `void promise` into
   `state.spawn(…)`; `process.env.X` into `state.env("X")`; `Date.now()` into `state.now_ms()`.
2. **Transcript:** add `tools/golden/api/groups/<group>.ts` (a `Group`: seed SQL, env, fake replies, steps with
   captures/checks; see `tools/golden/api/harness.ts`) and run
   `OC_GOLDEN_DATABASE_URL=postgres://localhost:5432/postgres OC_GOLDEN_API_GROUPS=<group> pnpm golden api`.
3. **Replay:** add one line `<name> => "<group>",` to the `groups!` list in `tests/parity.rs`, run
   `OC_DATABASE_URL=postgres://localhost:5432/postgres cargo test -p oc-api --test parity <name>`.
4. Update this table.

## Deliberate deviations and engine differences

* **SVG rasterising** (uploads, recolours): Rust resvg 0.48 vs `@resvg/resvg-js` 2.6.2 — same dimensions,
  different PNG bytes (sha256/size blanked in `assets`).
* **WebAuthn:** attestation formats `none` and self-`packed` only (browsers send `none` for the API's
  `attestation: 'none'`); certificate-chain formats answer 500 instead of being verified. Credential keys EdDSA, ES256, RS256 and
  ML-DSA-44 (COSE -48, offered first, as `@simplewebauthn/server` does on Node 24 from `.nvmrc`; verified with
  RustCrypto `ml-dsa`, cross-checked against Node in fixtures/webauthn/ml-dsa-44.json).
* **Malformed multipart bodies** answer 400 with multer's message (not covered by a transcript; busboy's may differ).
* **Response headers** `content-length`/`connection` are transport details (hyper sets them); JSON object key
  order of design documents follows yrs (sorted) where yjs keeps insertion order — clients do not depend on it,
  the transcripts compare JSON with sorted keys.
* **Body limits and IPs** follow Fastify (`bodyLimit` 10 MiB, `trustProxy: true` → left-most
  `X-Forwarded-For`); a body sent to an unknown route is not parsed (Fastify would parse it and could answer
  400 for malformed JSON instead of 404).
* Database sessions run in UTC (sqlx); node-postgres uses the server's default time zone, so DST-sensitive
  `now() + interval '30 days'` can differ by an hour on servers not running in UTC.
* **Template push** (`POST /api/templates/:id/push`, `templateUpdateOps`): values are compared by content. TS
  compares `JSON.stringify` output, so a template value (jsonb key order) and the same value in the design
  (Yjs insertion order) can count as different and be rewritten unchanged; yrs does not keep key order, so
  that cannot be reproduced. The `brandhub` scenario writes its documents in jsonb key order, where both agree.
  The op-log update bytes differ (Yjs client ids), so transcripts check update counts, not bytes.
* **Insights top lists** sort ties with an ASCII approximation of ICU `localeCompare` (punctuation < digits <
  letters, case-insensitive, lowercase first); non-ASCII keys fall back to code-point order.
* **Site password form / beacons:** the plugin-scoped `application/x-www-form-urlencoded` parser is a route
  layer (`sites::form_body`) that hands the body to `Req` as the equivalent JSON object; a too-large form body
  answers 413 before the route's rate limit is counted.
* **Custom domains:** `customDomainHook` runs in the global middleware without the TCP peer address, so a
  tracked visit without `X-Forwarded-For` hashes the IP as `127.0.0.1`. `POST .../domains/:domain/verify`
  resolves TXT records with a small UDP client against the `/etc/resolv.conf` name servers (no c-ares search
  domains, no TCP fallback for truncated answers); the transcript only covers a `.invalid` name.
* **Social posts `timezone`:** `validTz` accepts UTC offsets and names from the system tz database
  (`/usr/share/zoneinfo`, case-insensitive) instead of ICU's list.
* **Social test seam:** `OC_SOCIAL_FAKE_BASE` (ignored in production) rewrites `https://host/path` provider calls
  to `<base>/host/path`; the `social` scenario patches `fetch` the same way on the TS side.
* **oc-social token expiry** (`expiresIn`, Bluesky sessions) uses the system clock, not the app clock, so
  `token_expires_at` is not compared in transcripts.

* **`localeCompare`** in search suggestions (`rankSuggestions` ties) uses an approximation of ICU root collation
  (punctuation < digits < letters, case-insensitive first, lowercase before uppercase).
* **Webhook delivery payloads** (`emit_event` in oc-webhooks) stamp `created` with the system clock, not the app
  clock; the transcripts check delivery `event`/`data` only.
* **v1 rate limits:** the token routes count with their own per-route limiters keyed by the `Authorization`
  header (autofill: IP), after the token check, as Fastify's plugin-level `onRequest` order implies.
* **Font manifest transcripts:** the signed-out manifest is recorded in full; the signed-in ones blank
  `families.*.faces` to keep `misc.json` small (brand-font faces are `familiesFor`, covered by `content`).

* **Third-party API hosts (print, stock):** `OC_PRINTFUL_API_URL`, `OC_GELATO_ORDERS_URL`, `OC_STRIPE_API_URL`
  (print payments/refunds only), `OC_OPENVERSE_API_URL`, `OC_UNSPLASH_API_URL`, `OC_PEXELS_API_URL`,
  `OC_PIXABAY_API_URL`, `OC_GIPHY_API_URL` override the hard-coded base URLs (unset = the real hosts). The TS
  scenarios point the same hosts at the fake with `rewriteFetch` (tools/golden/api/rewrite.ts); the print scenario
  also holds a pool client around `fetch` and object-store writes (`holdPoolWhileWriting`) so the harness's settle
  loop waits for the in-process `print-fulfil` worker on a busy machine (timing only).
* **`print-fulfil` worker:** TS runs `work('print-fulfil')` in the API process (LISTEN + 5 s poll), and so does the
  Rust binary (`start_worker`: `oc_jobs::work`, concurrency 2, woken by the enqueue's NOTIFY). An app state without
  that loop (the parity harness) drains the queue after every enqueue (`print::fulfil::kick` → `oc_jobs::drain`, the
  same claim and bookkeeping, in a tracked background task the harness waits for), so a failed job is never retried
  within a transcript (the print scenario puts its failing render last). Several replicas share both queues through
  `FOR UPDATE SKIP LOCKED`; `crates/oc-platform/oc-jobs/tests/replicas.rs` checks each job runs exactly once.
* **Uploaded fonts** are read with a small sfnt reader instead of opentype.js: `name` records for Mac English and
  Windows en-US (last wins, like opentype.js's `en`), `OS/2` weight/fsSelection; a file without `glyf`+`loca`
  (+`head`/`maxp`) or `CFF ` outlines, or with a truncated table directory, is `corrupt_font`. Other opentype.js
  parse failures (e.g. malformed CFF) are not reproduced.
* **Emoji and graphics assets** are rasterised with Rust resvg (PNG byte sizes blanked in `content`/`graphics`).
* **`formatMoney`** (print confirmation emails) formats common currencies like `Intl` `en` (`$`, `€`, `£`, `¥`,
  `CA$`, …; other codes `XYZ 1.00`); only USD is covered by a transcript.
* **Stock/URL downloads:** DNS failures read `getaddrinfo …` with the resolver's message (Node: `ENOTFOUND`);
  the success path needs public https hosts and is not covered by a transcript (only the guards are).
* The `graphics`, `content` (emoji, built-in font names) transcripts depend on the synced fixtures
  (`pnpm graphics:sync`, `emoji:sync`, `fonts:sync`, as CI runs them).

* **Imports (oc-import):** PDF page pictures are rendered by hayro instead of poppler's `pdftocairo` (same pixel
  size, different PNG bytes; the `imports` transcript compares only sizes), so text colours measured from the
  renders differ by anti-aliasing noise (up to ~8/255 per channel in the goldens; compared within 0.05). PDF text
  follows pdf.js `getTextContent` except: no bidi reordering of right-to-left text, vertical (`-V`) CID fonts are
  read as horizontal, `\p{Mn}` is approximated by a non-zero combining class, and glyph names/standard-14 metrics
  come from hayro's (PDFBox/AGL-derived) tables. The text-free copy of a PDF is written as an incremental update
  instead of a pdf-lib rewrite. PowerPoint parts with whitespace text between elements (pretty-printed XML) import
  in Rust; the TS importer throws on them in theme colour schemes and custom geometry. Word: a style map embedded
  in the file by mammoth's `embedStyleMap` is ignored; zip/XML strictness may differ from JSZip/xmldom on
  borderline files (both answer 422 `unreadable` when a package cannot be read).

* **MCP (`/mcp`):** no MCP SDK on the Rust side; `mcp/mod.rs` implements what `@modelcontextprotocol/sdk` 1.32's
  `StreamableHTTPServerTransport` (stateless, `enableJsonResponse`) and `McpServer` answer: the Accept/Content-Type/
  JSON-RPC/batch/protocol-version checks with their status codes, `initialize` (version negotiation), `ping`,
  `tools/list` (the SDK's JSON Schemas kept verbatim in `mcp/tools.json`, generated from the TS tools) and
  `tools/call` with the SDK's error texts (`MCP error -32602: …`, zod issue messages). The reply is hijacked like
  `reply.hijack()`: only `content-type: application/json`, no `onSend` headers (`http::Hijacked`).
* **AI edits (`edit_design`):** TS runs a whole batch in one `ydoc.transact`; yrs transactions do not nest, so every
  operation is applied in its own transaction (the op-log fallback merges them into one `doc_updates` row; live
  collaborators may receive one update per operation). The live path is a small Hocuspocus 3 client on
  tokio-tungstenite (`mcp/live.rs`) instead of `@hocuspocus/provider`; the transcript covers only the fallback (the
  collab URL points at a closed port, so both sides wait the 5 s connect timeout per change).
* **OAuth test seam:** `OC_OAUTH_FAKE_BASE` (ignored in production) fetches client metadata documents
  `https://host/path` from `<base>/host/path`; services/api's oauth.ts has the same seam. The real fetch resolves
  the host, refuses any non-public address and pins the checked addresses (TS: a guarded `lookup` on
  `https.request`; Rust: `resolve_to_addrs`), never follows redirects, 5 s, 64 KB.
* **AI connector security decisions (2026-10-23):** token audience — OAuth access tokens only on `/mcp` (resource =
  the MCP endpoint, NULL on old rows), PATs on both `/mcp` (intended: CLI clients such as Claude Code and Codex)
  and `/v1`; `/mcp` limited per IP + token hash, failed token lookups 20/min per IP (recently valid tokens pass);
  codes spent only by a successful exchange; consent POST needs the exact Origin; export links bound to the user and
  grant (not single-use: assistants and chat previews fetch links); the worker drops OAuth clients unused for 30
  days without a live grant.

* **Supabase token keys** (ring instead of WebCrypto): RSA keys over 8192 bits are refused (jose accepts them), and
  a P-256 JWK whose point is not on the curve fails verification (401) instead of the import (TS: 503). Neither
  occurs with Supabase-issued keys.

## TS behaviour reproduced on purpose (worth fixing in both)

* `POST /api/invites/accept` consumes the invite before checking the account, so a wrong-account attempt burns
  the invite (`teams`).
* Supabase bridge (`supabase`): a brand-new account is created from the sign-up metadata before the Supabase MFA
  step-up is answered (only setting aside a squatted account waits for the step-up), so a token holder without the
  second factor still creates the account (no session is issued); and account creation / squat tombstoning run
  before the token is marked used (a replay then finds the linked account and answers `token_used`).
* `PATCH /api/social/posts/:id` parses `PostBody.partial()` the same way: omitted `pages`, `caption`, `hashtags`,
  `accounts`, `options`, `timezone` reset to the defaults and `status` becomes `scheduled`, so a patch with only
  `scheduledAt` fails with `no_accounts` (`social`).
* `PUT /api/designs/:id/site` without `description`/`noindex` clears them (`sites`).
* `GET /api/social/callback/:provider` deletes the OAuth state before comparing the provider, so a callback for
  the wrong provider burns the state (`social`).
* `GET /api/designs/:id/site/insights?to=2024-02-30`: V8's `Date.parse` accepts day 30 of February, Postgres
  rejects the date → 500 (`sites`).
* `POST /api/social/posts/:id/publish-now` queues the job with `delay = scheduled_at (database now()) − Date.now()`;
  under the transcripts' pinned 2025 clock that is a delay of many months (`social`). The job queue takes the delay
  as a multiple of `interval '1 millisecond'`: `'<n> milliseconds'::interval` overflowed on Postgres < 15.
* `GET /api/billing?workspace=<not a member>` answers **200** `{error: 'not_found'}` (`billing`).
* `POST /api/forms/:designId/:elementId`: an array value for an `email` field (even `[]`) is always "Enter a valid
  email" (`String(array)`), and `_hp: []` counts as a filled honeypot (`forms`).
* `POST /api/billing/local/confirm`: any failure inside the confirm (incl. database errors, the audit and
  analytics writes) answers 400 `expired`; anyone holding the session token can confirm it (`billing`).
* `GET /v1/designs/:id` (and document) answers a trashed design to its owner (`designRole` checks the owner
  before the trash) (`v1`).
* `POST /api/events` answers `stored: n` counting events whose props were too large to store (`growth`).
* `PUT /api/print/addresses/:id` with `default: true` clears the user's other defaults before finding that the
  id does not exist (404) (`print`).
* Staff `POST /api/admin/print/orders/:id/status` with `cancelled` + `refund: true` marks a never-paid order
  `refunded` (`print`); any staff status change clears `error` unless the new status is `failed`.
* `POST /api/print/orders/:id/local-pay` with a second valid checkout session of an already paid order consumes
  it and answers `{ok: true}` (`print`).
