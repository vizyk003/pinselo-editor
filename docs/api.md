# HTTP API reference

The Pinselo API server (`crates/oc-api`) exposes three kinds of HTTP surface:

| Surface | Path | Auth | Stability |
|---|---|---|---|
| Public REST API | `/v1/*` | Personal access token | The documented integration surface. Described by an OpenAPI 3.1 document at `GET /v1/openapi.json`. |
| AI connector | `/mcp` | OAuth access token or personal access token | See [mcp-connector.md](mcp-connector.md). |
| Web app API | `/api/*`, `/site/*`, `/healthz`, `/.well-known/*` | Mostly a session cookie | Serves the web app. Summarised under [Other route groups](#other-route-groups). |

Examples use `https://example.com` as the server. Links the API returns (`editUrl`, `statusUrl` and so on) start with the server's configured web origin; see [configuration.md](configuration.md).

## Authentication

### Personal access tokens

Every `/v1` route except `/v1/openapi.json` needs this header:

```
Authorization: Bearer oc_pat_<token>
```

Create a token in the web app under **Settings > Developers > API access tokens**, or with `POST /api/developer/tokens` while signed in. The token is shown once. The server stores only its SHA-256 hash.

The web app offers expiry in 30 days, 90 days, a year, or never. The endpoint accepts `expiresInDays` from 1 to 366, or no expiry when the field is left out.

A token can be limited to the designs of one workspace (**Designs** in the form, `workspaceId` on the endpoint). Designs in other workspaces answer `404`, exactly as if they did not exist. Such a token may only have the `designs:read`, `exports` and `designs:export` scopes, and it works on `/v1` only, not on `/mcp`. Use one for tools that publish designs unattended.

Revoking a token takes effect on the next request, and the public links of its [JPEG exports](#jpeg-exports-for-publishing) stop working too.

### Scopes

| Scope | Allows |
|---|---|
| `designs:read` | List and read designs and their documents. |
| `designs:write` | Create designs and run autofill. |
| `assets:write` | Upload assets. |
| `exports` | Start exports, read their status, download the files. |
| `designs:export` | The same as `exports`. Pair it with `designs:read` for a token that can only read and export designs. |
| `webhooks` | List, create and delete webhooks. |

`GET /v1/me` needs a valid token but no particular scope.

### OAuth access tokens

`oc_oat_` tokens are issued to connected AI apps and only work on `/mcp`. `/v1` refuses them with `401 invalid_token`. See [mcp-connector.md](mcp-connector.md#authentication).

### Session cookies

The web app signs in through `/api/auth/*`, which sets an HttpOnly cookie named `oc_session`, or `__Host-oc_session` in production. Sessions last 30 days. `/v1` does not read this cookie. `/api/developer/*` and most of `/api/*` do.

## Conventions

### Errors

Errors are JSON with an `error` code. Many also have a human-readable `message`.

| Status | Body | When |
|---|---|---|
| 400 | `{"error":"invalid_request","issues":[...]}` | The body or query failed validation. Each issue has `code`, `message` and `path`. |
| 400 | `{"error":"<parser message>"}` | The JSON body could not be parsed. |
| 401 | `{"error":"unauthenticated","message":"Send Authorization: Bearer <personal access token>."}` | The header is missing or does not start with `Bearer oc_pat_` or `Bearer oc_oat_`. |
| 401 | `{"error":"invalid_token"}` | The token is unknown, expired or revoked, or its user is disabled. An OAuth token gets the same code plus a `message`. |
| 403 | `{"error":"insufficient_scope","message":"This token needs the <scope> scope."}` | The token lacks the route's scope. |
| 404 | `{"error":"not_found"}` | The resource does not exist, or the token's user cannot open it. The two cases look the same on purpose. |
| 429 | `{"error":"Rate limit exceeded, retry in <time>"}` | See [Rate limits](#rate-limits). |
| 500 | `{"error":"internal"}` | Unexpected failure. Non-UUID ids on routes that cast to a UUID also land here, for example `DELETE /v1/webhooks/nope`. |

The checks run in this order: token, rate limit, body parsing, then scope. So a malformed body on a token without the right scope answers 400, not 403.

### Rate limits

| Routes | Limit | Counted per |
|---|---|---|
| `/v1/*` token routes except autofill | 120 per minute, with a separate counter for each route | `Authorization` header |
| JPEG exports (`POST /v1/designs/{id}/exports` with `format: "jpeg"`) | 200 per 24 hours. Idempotent replays are not counted. | Token |
| `POST /v1/designs/{id}/autofill` | 30 per minute | Client IP |
| `/mcp` | 240 per minute | Client IP plus token hash |
| Other routes without their own limit, including `/v1/openapi.json` | 600 per minute | Client IP |

Limited responses carry `x-ratelimit-limit`, `x-ratelimit-remaining` and `x-ratelimit-reset` (seconds). A 429 also carries `retry-after`. On `/v1`, a rejected token is not counted and gets no rate limit headers.

### Size limits

Request bodies are limited to 10 MiB, apart from uploads. Uploaded media is limited to `OC_MAX_MEDIA_BYTES` (default 1 GiB) and images to `OC_MAX_UPLOAD_BYTES` (default 50 MiB).

### The Design object

```json
{
  "id": "Salpha0000000000",
  "title": "Poster",
  "type": "graphic",
  "width": 1080,
  "height": 1080,
  "pages": 1,
  "createdAt": "2025-01-01T00:00:00.000Z",
  "updatedAt": "2025-01-01T00:00:00.000Z",
  "editUrl": "https://example.com/design/Salpha0000000000",
  "thumbnailUrl": "https://example.com/api/designs/Salpha0000000000/thumbnail.png"
}
```

`type` is the design's surface, such as `graphic`, `presentation` or `print`. `width` and `height` are in px at 96 dpi. A4 returns a height of `1122.52`, for example.

## /v1 reference

### GET /v1/openapi.json

The OpenAPI 3.1 description of this API. No token needed.

### GET /v1/me

The token's user and scopes.

```sh
curl https://example.com/v1/me -H "Authorization: Bearer $PINSELO_TOKEN"
```

```json
{
  "user": { "id": "00000000-0000-4000-8000-000000000001", "name": "Ada", "email": "ada@example.com" },
  "scopes": ["designs:read"]
}
```

### GET /v1/designs

Scope: `designs:read`. Lists designs the user owns, designs shared with them, and designs in their non-personal teams, most recently updated first. Trashed designs are left out.

| Query | Type | Default | Notes |
|---|---|---|---|
| `limit` | integer, 1 to 100 | 25 | Page size. |
| `before` | ISO 8601 datetime | | Only designs updated before this time. Pass the previous response's `next`. |
| `q` | string, up to 100 characters | | Case-insensitive title search. |

```sh
curl "https://example.com/v1/designs?limit=2&q=poster" -H "Authorization: Bearer $PINSELO_TOKEN"
```

```json
{ "items": [ { "id": "Salpha0000000000", "title": "Poster" } ], "next": null }
```

`items` holds [Design objects](#the-design-object), shortened here. `next` is the last item's `updatedAt` when there are more results, otherwise `null`.

### POST /v1/designs

Scope: `designs:write`. Creates a design in the user's personal workspace.

| Field | Type | Notes |
|---|---|---|
| `title` | string, 1 to 200 characters | Default `Untitled design`. |
| `preset` | string | Size preset id, for example `ig-post`. An unknown id is ignored rather than refused. |
| `width`, `height` | number, 16 to 20000 | Override the preset. Default 1080 × 1080 without a preset. |
| `document` | object | A full design document, in the form `GET /v1/designs/{id}/document` returns. When present, `preset`, `width` and `height` are ignored. `title` is written into it. |

```sh
curl https://example.com/v1/designs \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"title":"Launch post","preset":"ig-post"}'
```

Answers `201` with a [Design object](#the-design-object) and emits the `design.created` webhook event.

### GET /v1/designs/{id}

Scope: `designs:read`. Needs at least view access. Answers a [Design object](#the-design-object) or `404`. `{id}` may also be the design's edit link, URL-encoded (`https%3A%2F%2Fexample.com%2Fdesign%2FSalpha0000000000`), because spreadsheets and calendars often store the link.

This route adds two fields to the Design object:

```json
{
  "version": 14,
  "pageList": [
    { "index": 1, "id": "pg_a1", "name": "Cover", "hidden": false },
    { "index": 2, "id": "pg_b2", "name": "Page 2", "hidden": false }
  ]
}
```

`version` is the design's content version. It starts at 1 and goes up with every saved change, from the editor's autosave, AI assistants, or brand template updates. Record it when a design is approved and send it with the export (see [JPEG exports for publishing](#jpeg-exports-for-publishing)). `pageList` lists the pages in order. `index` is the page number that exports use, and `name` is the page title or `Page <n>`.

A trashed design is still returned to its owner.

### GET /v1/designs/{id}/document

Scope: `designs:read`. Needs at least view access. Answers `{"document": { ... }}`, the full design as JSON with `meta`, `pages`, `elements` and `assets`.

### POST /v1/designs/{id}/autofill

Scope: `designs:write`. Needs at least view access to the source. Copies a design and fills its data-bound elements. The source is not changed. The copy stays linked to the source's brand template, so the team's brand controls keep applying.

| Field | Type | Notes |
|---|---|---|
| `title` | string, up to 200 characters | Default `<source title> (autofill)`. |
| `data` | object of strings, each up to 10,000 characters | Required. Keys are the column names bound to elements in **Bulk create > Connect data to elements**. |

How values are applied:

- **Text elements:** the value replaces the text and keeps the first run's style. `\n` starts a new paragraph.
- **Image and frame elements:** the value is an `https://` URL, which is downloaded and imported, or the id of one of the user's image assets. The image is cropped to cover the element.
- Keys that match no element, and bound elements with no matching key, are skipped.

```sh
curl https://example.com/v1/designs/Sautotpl00000000/autofill \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"title":"Badge for Ada","data":{"name":"Ada Lovelace","photo":"https://example.com/ada.png"}}'
```

```json
{ "id": "Snewdesign000000", "editUrl": "https://example.com/design/Snewdesign000000" }
```

Answers `201` and emits `design.created`.

| Status | `error` | Meaning |
|---|---|---|
| 400 | `bad_url` | The image URL points at a non-public host. |
| 400 | `fetch_failed` | The image URL did not answer successfully. |
| 400 | `bad_image` | The value is neither an https image URL nor one of the user's image asset ids. |

Upload errors from [POST /v1/assets](#post-v1assets) can also occur for imported URLs.

### POST /v1/designs/{id}/exports

Scope: `exports`. Needs at least view access. Queues an export.

| Field | Type | Notes |
|---|---|---|
| `format` | `pdf`, `pdf-print`, `pptx`, `mp4`, `gif`, `m4a` | Required. PNG and JPG are not offered here. |
| `quality` | `720p`, `1080p`, `4k` | Video resolution. |
| `print` | object | Print options, below. |

`print` options:

| Field | Type | Notes |
|---|---|---|
| `bleedMm` | number, 0 to 10 | Required. |
| `cropMarks` | boolean | Required. |
| `registrationMarks`, `colorBars` | boolean | Optional marks. |
| `slugMm` | number, 0 to 20 | Slug area. |
| `standard` | `none`, `pdfx4`, `pdfx1a` | PDF standard. |
| `flatten` | boolean | Flatten transparency. |
| `profile` | string or `{"id":"<uuid>"}` | A bundled profile (`pso-coated-v3`, `pso-uncoated-v3`, `iso-coated-v2`, `srgb`) or an uploaded one. |
| `intent` | `relative`, `perceptual` | Rendering intent. |

```sh
curl https://example.com/v1/designs/Salpha0000000000/exports \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"format":"pdf-print","print":{"bleedMm":3,"cropMarks":true,"standard":"pdfx4","profile":"pso-coated-v3"}}'
```

Answers `202`:

```json
{ "id": "00000000-0000-4000-8000-000000007000", "status": "queued", "statusUrl": "https://example.com/v1/exports/00000000-0000-4000-8000-000000007000" }
```

| Status | `error` | Meaning |
|---|---|---|
| 404 | `profile_not_found` | The colour profile does not exist or is not shared with the user. |

With `"format": "jpeg"`, this route makes the [JPEG export for publishing](#jpeg-exports-for-publishing) instead: no queue, public links, and an optional version lock.

### JPEG exports for publishing

The public guide for integrators is the web app's `/developers/api` page (`apps/web/src/pages/ApiDocs.tsx`). Keep it in step with this section.

For tools that post designs on their own, for example to Instagram through Meta's Graph API. Scope: `exports` or `designs:export`. Needs at least view access.

```sh
curl https://example.com/v1/designs/Salpha0000000000/exports \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: P-002-v14" \
  -d '{"format":"jpeg","version":14,"width":1080,"pages":[1,2,3],"wait":true}'
```

| Field | Type | Notes |
|---|---|---|
| `format` | `jpeg` (or `jpg`) | Required. |
| `version` | integer | Optional. When sent, the export only runs if the design is still at this [version](#get-v1designsid). Otherwise it answers `409 version_changed` without rendering anything. |
| `width` | integer, 320 to 1440 | Output width in px. The default is 1080, and the height follows the design. Instagram accepts 320 to 1440. |
| `quality` | integer, 60 to 100 | The default is 90. A page over 7.5 MiB is rendered again 5 lower each time, down to 60. |
| `pages` | array of page numbers | 1 is the first page. Files come back in this order. The default is every visible page, in order. At most 10 pages, Instagram's carousel limit. |
| `wait` | boolean | `true` holds the request for up to 25 seconds until the files are ready. Otherwise, or if rendering takes longer, the route answers `202` and you poll `GET /v1/exports/{id}`. |

The `Idempotency-Key` header is optional, 1 to 200 visible ASCII characters. For 24 hours, the same key on the same token returns the same export and the same links without rendering again. A key reused for a different design answers `422`.

Answers `200` when the export is done:

```json
{
  "id": "exp_7Hq2uY1pQ0aZ3cV8bN5mKw",
  "status": "done",
  "designId": "Salpha0000000000",
  "designVersion": 14,
  "createdAt": "2026-11-14T17:23:02.114Z",
  "statusUrl": "https://example.com/v1/exports/exp_7Hq2uY1pQ0aZ3cV8bN5mKw",
  "warnings": [],
  "files": [
    {
      "page": 1,
      "url": "https://example.com/v1/files/exp_7Hq2uY1pQ0aZ3cV8bN5mKw/1.jpg?e=1794763390&sig=…",
      "width": 1080,
      "height": 1350,
      "bytes": 412883,
      "contentType": "image/jpeg"
    }
  ],
  "expiresAt": "2026-11-15T17:23:10.552Z"
}
```

Every file meets Instagram's image rules:

- It is a baseline JPEG, with transparency flattened onto white and the sRGB profile embedded.
- Each file is 7.5 MiB or less.
- Fonts must load. If one can't, the export fails with `render_failed` rather than rendering a fallback font.

`warnings` holds `{ "code": "aspect_ratio", "message": "…" }` when the design is taller than 4:5 or wider than 1.91:1, which Instagram's feed does not accept as is.

The file links work without signing in for 24 hours (`expiresAt`). Meta's servers download them directly.

- **The request:** a plain `GET`. No cookies or headers are needed, and the path ends in `.jpg`.
- **The response:** `Content-Type: image/jpeg`, with no redirects.
- **When a link stops working:** after it expires, when it was altered, or when the token that made it is revoked. It then answers `404`.

The export checks the version again after its last page has rendered. If the design was edited while rendering, the export fails with `version_changed` instead of returning a mix of old and new pages.

| Status | `error` | Meaning |
|---|---|---|
| 202 | | Still rendering (`"status": "rendering"`). Poll `statusUrl`. |
| 404 | `not_found` | Unknown design, or one the token cannot see, including designs outside a workspace-limited token's workspace. |
| 409 | `version_changed` | The design is no longer at `version`. The body carries `currentVersion`. Treat it as "changed after approval". |
| 422 | `invalid_export` | A bad field, such as a width outside 320 to 1440, a page out of range, more than 10 pages, a document or spreadsheet design, or a page still over 7.5 MiB at quality 60. The `message` says which. |
| 429 | `rate_limited` | 200 exports in the last 24 hours. Wait `retry-after` seconds. |
| 500 | `render_failed` | Rendering failed, for example because a font could not be loaded. Retry once later. |
| 503 | `unavailable` | The render service or file storage is unreachable. Retry later. |

Errors found before rendering answer `{"error":"<code>","message":"…"}`, plus `currentVersion` on a 409. An export that fails while rendering, with `wait: true`, answers the same status and adds the export's fields.

### GET /v1/exports/exp_…

Scope: `exports` or `designs:export`. The JPEG export as `POST` returned it. `status` is `rendering`, `done` or `failed`. A failed export answers `200` with `"status": "failed"` and `"error": { "code": "…", "message": "…" }`, using the codes above. An export interrupted by a server restart is reported as failed after 10 minutes. Links are signed once, at completion, so repeated reads return the same URLs.

### GET /v1/exports/{job}

Scope: `exports`. The status of one of the user's export jobs.

```json
{
  "id": "00000000-0000-4000-8000-000000007000",
  "status": "done",
  "progress": 1,
  "filename": "Poster.pdf",
  "size": 48213,
  "downloadUrl": "https://example.com/v1/exports/00000000-0000-4000-8000-000000007000/file"
}
```

`status` is `queued`, `running`, `done`, `error` or `cancelled`. `progress` runs from 0 to 1. `error` holds the failure text when a job fails. `filename`, `size` and `downloadUrl` appear only when the job is done. Another user's job, or an unknown or malformed id, answers `404 not_found`.

### GET /v1/exports/{job}/file

Scope: `exports`. Downloads a finished export as an attachment with the file's content type. A job that is unfinished or belongs to someone else answers `404 {"error":"not_ready"}`.

```sh
curl -OJ https://example.com/v1/exports/$JOB/file -H "Authorization: Bearer $PINSELO_TOKEN"
```

### POST /v1/assets

Scope: `assets:write`. Uploads a multipart file. Images may be PNG, JPEG, WebP, GIF or SVG; video MP4, MOV or WebM; audio MP3, M4A, WAV or OGG.

```sh
curl https://example.com/v1/assets -H "Authorization: Bearer $PINSELO_TOKEN" -F file=@photo.png
```

Answers `201`:

```json
{ "asset": { "id": "Sassetimg0000000", "kind": "image", "mime": "image/png", "name": "photo.png", "sha256": "cd6bb5fc...", "w": 2, "h": 3 } }
```

| Status | `error` | Meaning |
|---|---|---|
| 400 | `no_file` | The form has no file. |
| 402 | `upgrade_required` | The user's storage quota is full. |
| 406 | `the request is not multipart` | Wrong content type. |
| 413 | `too_large` or `request file too large` | Over the size limit. |
| 415 | `unsupported_type` | Not an accepted file type. |

### GET /v1/webhooks

Scope: `webhooks`.

```json
{ "items": [ { "id": "00000000-0000-4000-8000-000000009100", "url": "https://example.com/hook", "events": ["design.created"], "active": true, "created_at": "2025-01-01T00:00:00.000Z" } ] }
```

### POST /v1/webhooks

Scope: `webhooks`. Subscribes an https URL to events.

| Field | Type | Notes |
|---|---|---|
| `url` | URL, up to 2000 characters | Required. |
| `events` | array, at least one | `design.created`, `design.deleted`, `export.completed`, `form.response`, `comment.created`, `site.published`, `ping`. |

```sh
curl https://example.com/v1/webhooks \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com/hook","events":["design.created","export.completed"]}'
```

Answers `201`. `secret` is returned only here, so store it now:

```json
{ "id": "<uuid>", "url": "https://example.com/hook", "events": ["design.created", "export.completed"], "secret": "whsec_..." }
```

### DELETE /v1/webhooks/{id}

Scope: `webhooks`. Answers `204` whether or not the webhook existed. An id that is not a UUID answers `500`.

## Webhook deliveries

Each event is sent as a JSON `POST`:

```json
{ "id": "<delivery uuid>", "event": "design.created", "created": "2025-01-01T00:00:00.000Z", "data": { "designId": "Salpha0000000000", "title": "Launch post", "via": "api" } }
```

| Header | Value |
|---|---|
| `OC-Signature` | `t=<unix seconds>,v1=<hex HMAC-SHA256>` |
| `OC-Delivery` | The delivery id. |
| `User-Agent` | `Pinselo-Webhooks/1` |

To verify a delivery, compute HMAC-SHA256 of the string `<t>.<raw body>` with the signing secret and compare it with `v1`.

```js
import { createHmac, timingSafeEqual } from 'node:crypto';

function verify(header, rawBody, secret) {
  const parts = Object.fromEntries(header.split(',').map((p) => p.split('=')));
  const want = createHmac('sha256', secret).update(`${parts.t}.${rawBody}`).digest();
  const got = Buffer.from(parts.v1, 'hex');
  return got.length === want.length && timingSafeEqual(got, want);
}
```

Delivery rules:

- Any 2xx response counts as delivered. Redirects are not followed, and each attempt times out after 10 seconds.
- A failed delivery is retried with exponential backoff, up to 6 attempts.
- After 50 consecutive failures the webhook is switched off. Sending a test with `POST /api/developer/webhooks/{id}/ping` switches it back on.
- URLs must use https. In production, hosts that resolve to a private address are refused at delivery time, not when the webhook is created.

## Token and webhook management (session cookie)

These routes back **Settings > Developers** and need a signed-in session, not a token.

| Method and path | Body | Response |
|---|---|---|
| `GET /api/developer` | | `{tokens, webhooks, scopes, events}`. Tokens never include their secret. Each webhook includes `failures` and its last 10 deliveries. |
| `POST /api/developer/tokens` | `name` (1 to 80 characters), `scopes` (at least one), optional `expiresInDays` (1 to 366) | `{id, token}` |
| `DELETE /api/developer/tokens/{id}` | | `{ok:true}` |
| `POST /api/developer/webhooks` | `url`, `events` | `{id, secret}` |
| `POST /api/developer/webhooks/{id}/ping` | | `{ok:true}`, or `{ok:false}` if it is not yours. Reactivates the webhook, subscribes it to `ping` and sends a test event. |
| `DELETE /api/developer/webhooks/{id}` | | `{ok:true}` |

## Other route groups

The routes below serve the web app. They can change with it, so prefer `/v1` for integrations. Request and response shapes are in the handlers named in each heading (under `crates/oc-api/src/routes/`) and in the recorded transcripts in `tests/golden/api/`.

Unless a section says otherwise, expect these routes to use the session cookie. A handler can accept signed-out callers, so check it before relying on that. Routes under `/api/public/` and `/site/` serve signed-out visitors. `/api/admin/*` is limited to staff accounts.

### Accounts and sign-in (`account.rs`, `supabase.rs`)

| Methods | Path |
|---|---|
| GET | `/api/auth/providers`, `/api/auth/me` |
| POST | `/api/auth/signup`, `/api/auth/login`, `/api/auth/mfa`, `/api/auth/logout` |
| POST | `/api/auth/magic`, `/api/auth/magic/verify`, `/api/auth/verify`, `/api/auth/verify/resend` |
| POST | `/api/auth/reset/request`, `/api/auth/reset` |
| POST | `/api/auth/passkey/register/options`, `/api/auth/passkey/register/verify`, `/api/auth/passkey/login/options`, `/api/auth/passkey/login/verify` |
| GET | `/api/auth/oauth/{provider}/start`, `/api/auth/oauth/{provider}/callback` |
| POST | `/api/auth/supabase/session`, `/api/account/supabase/sync` |
| GET, PATCH, DELETE | `/api/account` |
| PUT | `/api/account/email-preferences` |
| POST | `/api/account/password`, `/api/account/sessions/revoke-others` |
| GET | `/api/account/sessions` |
| DELETE | `/api/account/sessions/{id}`, `/api/account/passkeys/{id}` |
| POST | `/api/account/2fa/setup`, `/api/account/2fa/enable`, `/api/account/2fa/disable` |
| POST, GET | `/api/account/export`, `/api/account/exports/{job}` |

### Designs (`designs.rs`, `docs.rs`, `imports.rs`)

| Methods | Path |
|---|---|
| GET, POST | `/api/designs` |
| POST | `/api/designs/bulk` |
| GET, PATCH, DELETE | `/api/designs/{id}` |
| POST | `/api/designs/{id}/restore`, `/api/designs/{id}/copy`, `/api/designs/{id}/touch`, `/api/designs/{id}/export` |
| GET | `/api/designs/{id}/ticket`, `/api/designs/{id}/doc`, `/api/designs/{id}/thumbnail.png`, `/api/designs/{id}/pages/{page}` |
| GET, POST | `/api/designs/{id}/shares`, `/api/designs/{id}/versions` |
| DELETE | `/api/designs/{id}/shares/{user_id}` |
| GET | `/api/designs/{id}/versions/{vid}` |
| POST | `/api/imports`, `/api/docs/import-docx`, `/api/designs/{id}/doc/convert` |
| GET | `/api/imports/{job}` |

### Assets, media and content (`assets.rs`, `media.rs`, `content.rs`, `graphics.rs`, `embeds.rs`)

| Methods | Path |
|---|---|
| GET, POST | `/api/assets` |
| GET, PATCH, DELETE | `/api/assets/{id}` |
| GET | `/api/assets/trash`, `/api/assets/{id}/download`, `/api/assets/{id}/poster`, `/api/assets/{id}/svg`, `/api/assets/{id}/recolor/{pairs}`, `/api/assets/{id}/waveform` |
| POST | `/api/assets/batch`, `/api/assets/import-url`, `/api/assets/{id}/restore` |
| GET | `/api/export-jobs/{job}`, `/api/export-jobs/{job}/file` |
| POST | `/api/designs/{id}/export-jobs` |
| GET, POST | `/api/stock/providers`, `/api/stock/search` (GET); `/api/stock/import` (POST) |
| GET, POST | `/api/emoji/index.json`, `/api/emoji/{hex}` (GET); `/api/emoji/{hex}/asset` (POST) |
| GET, POST | `/api/graphics/sets`, `/api/graphics/search`, `/api/graphics/{file}` (GET); `/api/graphics/{id}/asset` (POST) |
| GET, POST, DELETE | `/api/fonts/custom` (GET, POST); `/api/fonts/custom/{id}` (DELETE) |
| GET | `/api/fonts/manifest.json`, `/api/fonts/preview/{file}`, `/api/fonts/files/{*rel}`, `/api/fonts/files/custom/{file}`, `/api/designs/{id}/fonts`, `/api/designs/{id}/credits` |
| POST | `/api/embeds` |

### Templates, folders, comments and access (`misc.rs`, `content.rs`, `sharing.rs`, `search.rs`)

| Methods | Path |
|---|---|
| GET, POST | `/api/templates` |
| GET, PATCH, DELETE | `/api/templates/{id}` |
| GET | `/api/templates/{id}/preview`, `/api/templates/{id}/thumbnail.png`, `/api/admin/templates` |
| GET, POST | `/api/folders` |
| PATCH, DELETE | `/api/folders/{id}` |
| GET, POST | `/api/designs/{id}/comments` |
| PATCH, DELETE | `/api/comments/{id}` |
| POST | `/api/comments/{id}/reactions` |
| GET | `/api/designs/{id}/collaborators`, `/api/designs/{id}/access-requests`, `/api/tasks` |
| POST | `/api/designs/{id}/transfer`, `/api/designs/{id}/request-access`, `/api/access-requests/{id}` |
| GET | `/api/designs/{id}/sharing`, `/api/sharing/people` |
| POST, DELETE | `/api/designs/{id}/group-shares`; `/api/designs/{id}/group-shares/{group_id}` |
| PUT | `/api/designs/{id}/team-access`, `/api/designs/{id}/slug` |
| POST | `/api/designs/{id}/make-private`, `/api/template-links/{token}/use` |
| POST, DELETE | `/api/designs/{id}/template-link` |
| GET | `/api/public/template-links/{token}`, `/api/public/template-links/{token}/thumbnail.png` |
| GET | `/api/search`, `/api/search/suggest` |

### Brand and teams (`brand.rs`, `brandhub.rs`, `teams.rs`)

| Methods | Path |
|---|---|
| GET, POST | `/api/brand-kits` |
| GET, PATCH, DELETE | `/api/brand-kits/{id}` |
| POST | `/api/brand-kits/{id}/default` |
| GET, PUT | `/api/brand`, `/api/workspaces/{id}/brand-controls` |
| GET | `/api/designs/{id}/brand` |
| POST | `/api/templates/{id}/republish` |
| GET | `/api/notifications` |
| POST | `/api/notifications/read` |
| GET, POST | `/api/workspaces`, `/api/workspaces/{id}/groups` |
| PATCH | `/api/workspaces/{id}` |
| GET | `/api/workspaces/{id}/members` |
| POST | `/api/workspaces/{id}/invites`, `/api/invites/accept`, `/api/workspaces/{id}/groups/{gid}/members` |
| DELETE | `/api/workspaces/{id}/invites/{inv}`, `/api/workspaces/{id}/groups/{gid}/members/{user_id}` |
| PATCH, DELETE | `/api/workspaces/{id}/members/{user_id}`, `/api/workspaces/{id}/groups/{gid}` |

### Public views, websites and forms (`brand.rs`, `embeds.rs`, `sites.rs`, `forms.rs`)

| Methods | Path |
|---|---|
| GET, POST | `/api/designs/{id}/public` |
| GET | `/api/public/designs/{id}`, `/api/public/designs/{id}/thumbnail.png`, `/api/public/oembed` |
| POST | `/api/public/designs/{id}/view` |
| GET, POST | `/site/{slug}` |
| GET | `/site/{slug}/{file}`, `/api/sites/domain-check`, `/api/sites/og/{file}` |
| POST | `/api/sites/{id}/e` |
| GET, PUT | `/api/designs/{id}/site` |
| POST | `/api/designs/{id}/site/publish`, `/api/designs/{id}/site/preview`, `/api/designs/{id}/site/unpublish`, `/api/designs/{id}/site/domains`, `/api/designs/{id}/site/domains/{domain}/verify` |
| DELETE | `/api/designs/{id}/site/domains/{domain}` |
| POST | `/api/forms/{design_id}/{element_id}` |
| GET | `/api/designs/{id}/forms/{element_id}/responses` |
| DELETE | `/api/designs/{id}/forms/{element_id}/responses/{rid}` |

### Print (`print_profiles.rs`)

| Methods | Path |
|---|---|
| POST | `/api/print/gamut` |
| GET, POST | `/api/print/profiles` |
| DELETE | `/api/print/profiles/{id}` |

### Social scheduling (`social.rs`)

| Methods | Path |
|---|---|
| GET | `/api/social/providers`, `/api/social/accounts`, `/api/social/media/{file}`, `/api/social/callback/{provider}`, `/api/social/posts/{id}/events` |
| POST | `/api/social/connect/{provider}`, `/api/social/posts/{id}/publish-now` |
| PATCH, DELETE | `/api/social/accounts/{id}`, `/api/social/posts/{id}` |
| GET, POST | `/api/social/posts` |

### AI connector support (`oauth.rs`, `mcp/`)

Described in [mcp-connector.md](mcp-connector.md).

| Methods | Path |
|---|---|
| POST | `/mcp` |
| GET | `/.well-known/oauth-protected-resource`, `/.well-known/oauth-protected-resource/mcp`, `/.well-known/oauth-authorization-server` |
| POST | `/api/oauth/register`, `/api/oauth/token`, `/api/oauth/revoke`, `/api/oauth/authorize` |
| GET | `/api/oauth/authorize/info`, `/api/oauth/connections`, `/api/mcp/files/{sha}`, `/api/designs/{id}/ai-checkpoint` |
| DELETE | `/api/oauth/connections/{client_id}` |

### Operations, support and moderation (`health.rs`, `growth.rs`, `safety.rs`, `dev.rs`)

| Methods | Path |
|---|---|
| GET | `/healthz`, `/api/config`, `/api/status`, `/api/dev/mail` |
| GET, POST | `/api/consent`, `/api/support` |
| POST | `/api/events`, `/api/support/{id}/reply`, `/api/reports` |
| GET | `/api/admin/analytics`, `/api/admin/support`, `/api/admin/overview`, `/api/admin/reports`, `/api/admin/users`, `/api/admin/audit`, `/api/admin/jobs` |
| POST | `/api/admin/support/{id}/reply`, `/api/admin/incidents`, `/api/admin/incidents/{id}/updates`, `/api/admin/reports/{id}/decide`, `/api/admin/users/{id}/{action}`, `/api/admin/hashes` |

## Testing against the API

The recorded HTTP transcripts in `tests/golden/api/` replay against the router in `cargo test -p oc-api --test parity`. See [testing.md](testing.md).
