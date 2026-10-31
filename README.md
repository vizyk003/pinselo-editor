# Pinselo

Pinselo is an open-source design editor for graphics, presentations, documents, websites and video, with real-time collaboration, print-ready PDF export and a self-hostable backend. The backend and the editor engine are written in Rust, and the same renderer runs on the server and, through WebAssembly, in the browser.

Pinselo has no built-in AI. You connect your own assistant, such as Claude or ChatGPT, through the [MCP connector](docs/mcp-connector.md).

## Documentation

| Guide | For |
|---|---|
| [User guide](docs/user-guide.md) | Using the editor, including print export |
| [Getting started](docs/getting-started.md) | Running Pinselo locally for development |
| [Self-hosting](docs/self-hosting.md) | Running Pinselo in production |
| [Configuration](docs/configuration.md) | Every environment variable |
| [Running on Supabase](docs/supabase.md) | Using Supabase for the database, auth or storage |
| [Oracle Cloud example](docs/deploy-oracle.md) | A worked single-server deployment |
| [Architecture](docs/architecture.md) | How the services and crates fit together |
| [Testing](docs/testing.md) | Test layers, goldens and CI |
| [HTTP API](docs/api.md) | The API and the public `/v1` surface |
| [MCP connector](docs/mcp-connector.md) | Designing with your own AI assistant |
| [Architecture decisions](docs/adr/0001-rust-core.md) | Why the core is Rust |

## Run it locally

Requirements: Node 22+ and pnpm 10 for the web app and the tooling; a stable Rust toolchain
(`cargo`) with the `wasm32-unknown-unknown` target and the `wasm-bindgen` CLI at the version in `Cargo.lock` (the
backend and the editor engine are Rust, see `packages/engine/README.md`); a local Postgres; `ffmpeg` (video export
and ingest).

```bash
pnpm install
createdb opencanvas            # or set OC_DATABASE_URL
pnpm --filter @oc/engine build # the seed step needs the WebAssembly engine
pnpm run setup                 # fonts/emoji/graphics/ICC sync → db:migrate (oc-migrate) → seed (188 templates + thumbnails)
pnpm dev                       # web :3000, api :4000, collab :4100, render :4200, worker
```

`pnpm dev` (`tools/scripts/dev.ts`) builds the four services — `crates/oc-api`, `oc-collab`, `oc-worker` and
`oc-render-svc` — in release into `target/dev` (`OC_DEV_TARGET_DIR` overrides; the first build takes a few minutes)
and runs them next to the Vite dev server, which builds `@oc/engine` (the Rust core compiled to WebAssembly) when the
crates changed. `OC_DEV_WATCH=1` rebuilds and restarts the services when `crates/` change; `OC_DEV_SERVICES=api,collab`
starts only those. `pnpm db:migrate` and `pnpm seed` build and run the `oc-migrate` and `oc-templates` binaries the
same way (`tools/scripts/rust.ts`). Seeding never rewrites tracked files; `pnpm seed --fixtures` also refreshes
`fixtures/templates-seed` (the goldens' corpus) for templates whose source changed.

Open http://localhost:3000 and create an account, or use the development sign-in (email only, disabled in production). Without SMTP, every email (sign-in links, invites, resets) lands in the dev mailbox at http://localhost:3000/dev/mail. Set `OC_STAFF_EMAILS=you@example.com` on the API to get the staff console at `/admin`.

Tests:

| Command | What |
|---|---|
| `pnpm typecheck`, `pnpm test` | the web app and the tooling (vitest; `@oc/engine` must be built) |
| `pnpm --filter @oc/engine test` | the engine through the WebAssembly: goldens, sync, text, and PS-1 (pixels and layout equal the native renderer) |
| `cargo test --workspace` | the Rust crates: goldens (`tests/golden`), PS-1 baselines, parity transcripts of every API route; DB tests need `OC_DATABASE_URL` (they create throwaway databases) |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints, as CI runs them |
| `bash tools/golden/check.sh` | the CI golden job: `cargo test --workspace`, failing if a test rewrote `tests/golden/` |
| `bash tests/smoke/api.sh <url>`, `pnpm exec tsx tests/smoke/stack.ts <api> <collab ws>` | smoke tests against a running stack |

CI (`.github/workflows/ci.yml`) runs all of these against Postgres 17.

Production: `deploy/` holds a Dockerfile (the `runtime` image with the Rust binaries, the `gateway` image with Caddy and the web app), a compose stack (Postgres, migrate, api, collab, render, worker, gateway) and `.env.example`. The gateway sets CSP/HSTS and serves uploads from a separate user-content domain. Services refuse to start in production with a development secret. Supabase (database, auth, storage): `docs/supabase.md`.

## Repository layout

The backend and the editor engine are Rust ([ADR-0001](docs/adr/0001-rust-core.md)); TypeScript is the React shell.

| Path | What |
|---|---|
| `crates/oc-schema` | Document model (pages, elements, text runs, paints, effects): serde types, validation, ids, fractional ordering, size presets, migrations |
| `crates/oc-geometry` | Transforms, world bounds, hit-testing, shape outlines, smart-guide snapping, align/distribute, connectors |
| `crates/oc-crdt` | yrs mapping with the browser's yjs layout (flat element map, atomic `xf`, rich text), `DocOp`, the single validated write path, per-user undo |
| `crates/oc-text` | Text engine: pinned font registry, glyph fallback, paragraph layout, caret/selection geometry |
| `crates/oc-render` | Page → display list; tiny-skia raster, SVG and krilla PDF writers (the same painter in the browser and on the server) |
| `crates/oc-editor-core` | DOM-free editor engine: selection, gestures with snapping, crop, text editing, clipboard, grouping, pages |
| `crates/oc-sheet`, `oc-color`, `oc-import` | Sheets (formulas, XLSX); colour management (ICC, CMYK); PowerPoint/PDF/Word import |
| `crates/oc-wasm` → `packages/engine` | The core compiled to WebAssembly and its typed TS facade (`@oc/engine`, the document types in `src/types`) |
| `crates/oc-api` | HTTP API: sessions, designs, sharing/roles, uploads, templates, fonts, comments, versions, exports, `/v1`, the MCP connector |
| `crates/oc-collab` | Real-time sync: the Hocuspocus 3 protocol over yrs, ticket auth, read-only roles, ack-after-persist, version snapshots |
| `crates/oc-render-svc` | PNG/JPG/PDF/SVG/PPTX/video exports and thumbnails, drains the `export` queue; `oc-templates` seeds the template catalogue |
| `crates/oc-worker` | Email delivery, webhooks, account export and erasure, social publishing, trash purge, cleanup, object GC |
| `crates/oc-platform/*` | Config, Postgres pool and migrator (`oc-db/migrations`), job queue, object store (local or S3), mail (`oc-mail/locales`), audit chain, webhooks, service tickets, Google Fonts |
| `apps/web` | React/Vite app: home, templates, projects, editor shell, panels, toolbar, comments, versions, share, export, present |
| `apps/mobile` | Capacitor shell for iOS/Android |
| `packages/config` | Brand and shell configuration for the web app and the tooling |
| `tools/scripts` | Fixture sync (fonts, emoji, graphics, ICC), template sources (`templates/`), dev runner, i18n extraction |

## Features

Create a design from 28 size presets or a custom size, or from 188 original templates. In the editor you can:
- **Add content:** text, 12 shapes, lines and arrows, frames, QR codes, and uploaded images (drag and drop, paste, or the Uploads panel).
- **Edit elements:** select, marquee-select, move, resize, rotate with smart guides; group/ungroup, lock, layers, align/distribute/tidy, arrange.
- **Format:** rich text editing with IME support, fonts, size, colour, B/I/U/S, case, alignment, lists, spacing, and text effects. Fills and gradients, borders, corner radius, transparency, drop shadows, links.
- **Edit images:** crop and pan, zoom, flip, 8 filters, adjustments.
- **Pages and history:** multi-page designs with thumbnails, reorder, hide and speaker notes; undo/redo.
- **Collaborate:** real-time co-editing with live cursors and selections, offline cache, comments with element pins, version history and restore, sharing by person or link (view/comment/edit).
- **Output:** export to PNG/JPG/PDF/ZIP; resize the design; Present mode.
- **More content:** 32 shapes, freehand draw (pen, marker, highlighter), tables with inline cell editing, charts (column, bar, line, area, pie, donut) with a data editor, sticky notes.
- **Motion:** 10 entrance animations per element or per page, page transitions (fade, dissolve, slide, zoom), page durations, live preview. Present mode plays them, and **MP4 / GIF export** renders them on the server through ffmpeg.
- **Exports:** PDFs carry an invisible, positioned text layer (selectable, searchable) and clickable links. SVG export keeps real `<text>` with embedded fonts.
- **Brand kit:** colours, heading/subheading/body fonts and logos per workspace. Brand colours appear in every colour picker, plus one-click "apply brand fonts".
- **AI:** no built-in model. Connect your own Claude, ChatGPT, Claude Code or Codex to the MCP connector (`/mcp`, set up under Settings → Developers) and it designs in your account: live presence, a checkpoint before its first edit and one-click "Undo AI changes".
- **Video editor:**
  - **Media:** upload video (MP4/MOV/WebM) and audio (MP3/M4A/WAV/OGG). The server makes an H.264/AAC proxy, a poster frame and waveform peaks; phone rotation is handled, and playback can seek.
  - **On the page:** video elements with trim, speed (0.25–3×), volume, mute, loop, rounded corners and "fill page". Elements get visibility windows, exit animations and motion paths.
  - **Timeline:** play/pause (Space), scrub, split the selected video or the whole page at the playhead (S), drag page durations, element timing and trim handles.
  - **Soundtrack and captions:** soundtrack clips with waveforms, move/trim/split, volume and fades; beat sync that snaps page cuts to detected beats; a caption track (manual editing, styling, SRT/VTT download, auto captions through a local whisper.cpp).
  - **Recording:** record yourself presenting (camera bubble, one segment per page) or record your screen.
  - **Export:** MP4 (720p/1080p/4K) with real video frames and the mixed audio, GIF, and audio-only M4A, all as background jobs with progress. Present mode plays video and the soundtrack.
- **Accounts and security:**
  - **Sign-in:** password (scrypt), magic links, passkeys (WebAuthn), Google/Apple/any OIDC provider (PKCE), and TOTP two-step verification with recovery codes.
  - **Account safety:** sign-up age gate (13+) and terms clickwrap, email verification, password reset, new-device sign-in emails, a session list with remote logout, and per-route rate limits.
  - **Data rights:** self-serve data export (zip, emailed link) and account deletion.
- **Teams:** team workspaces with owner/admin/designer/member roles, email invites, role changes and leave/remove with last-owner protection. Access changes take effect live: collab closes affected connections and refuses older tickets.
- **Trust & safety:**
  - **Uploads:** a perceptual-hash gate against a blocklist.
  - **Reports:** a report button on public views, with a DMCA/DSA notice form; NCII/CSAM reports hide the public link immediately.
  - **Staff console:** a prioritised queue with takedown/remove/disable/restore actions and statements of reasons emailed to affected users, user enforcement, a job monitor and the audit-chain check.
- **Plans and billing:**
  - **Plans:** Free, Pro and Teams (per seat, from 3 people), monthly or yearly, with 30-day trials.
  - **Limits enforced on the server:** premium templates (a third of the catalogue), storage quota, 4K export, version-history window and team seat caps. Hitting one shows an upgrade prompt.
  - **Payments:** Stripe Checkout, the Customer Portal and signed, idempotent webhooks when `OC_STRIPE_SECRET_KEY` (and `OC_STRIPE_PRICE_*`, `OC_STRIPE_WEBHOOK_SECRET`) are set. Without them, a local provider activates plans with no charge (development only).
  - **Billing pages:** a pricing page, Settings → Billing (usage, invoices, cancel/resume, seats).
- **Legal and consent:** Terms, Privacy, Cookies, Acceptable Use, IP notices and Subprocessors pages (templates — have counsel review them), and a cookie banner with equal-weight choices that you can change any time.
- **Analytics:** first-party and consent-gated: client events are batched to `/api/events` and checked against an allowlist; server-side events are anonymous without consent. Staff see a dashboard with daily actives, sign-ups, designs, exports, the activation funnel and paid plans.
- **Help and status:**
  - **Help centre:** searchable articles at `/help`, a contact form that suggests articles, and ticket threads at `/help/requests`. Staff reply from the console, and replies are emailed.
  - **Status page:** a public page at `/status` built from service heartbeats and the database check, with staff-published incidents and updates.
- **Durable editing:** every accepted update is appended to an op log before the debounced snapshot, so a server crash loses nothing. Load replays the log, runs schema migrations and repairs orphans, parent cycles and invalid transforms.
- **Sharing and presenting:** public view links with no sign-in (owner toggle in Share), a presenter-view window (notes, timer, next slide) kept in sync with Present mode, and in-app notifications for @mentions, replies, comments and shares.

- **Editor power tools:** shrink-to-fit and curved text, a position panel, rulers and guides, spacing guides, Alt-drag duplicate, copy/paste style, find & replace, connectors that follow their shapes, photo grids, an eyedropper, 20 filters plus duotone, blend modes, alt text and a page grid view.
- **Content:** a template studio (publish team templates), Noto emoji and stickers, a searchable graphics & icon library (~12k open-licence icons from Lucide, Tabler and Phosphor plus illustrations and original decorative graphics; `pnpm graphics:sync`) with non-destructive SVG recolouring, animated GIFs (stored as looping video), custom font upload, 185 bundled font families, and stock media from Openverse by default (Unsplash, Pexels, Pixabay and GIPHY with keys) with automatic credits.
- **Exports and imports:** vector PDF with embedded fonts, print PDF with bleed and crop marks, PPTX export and import, PDF import (editable text over a background), and HTML email export.
- **Design types:** whiteboards that grow as you draw, Docs (rich text, real-time, convert to a paginated design), websites (publish to a URL or a custom domain with automatic TLS), forms that collect responses (CSV export), and bulk create from CSV.
- **Public API and webhooks:** personal access tokens with scopes (Settings → Developers), `/v1` REST endpoints for designs, autofill, exports and assets, an OpenAPI document at `/v1/openapi.json`, and signed webhooks (`OC-Signature`) with retries and delivery logs.
- **Languages:** the whole UI and the emails are available in English, Spanish, French, German, Brazilian Portuguese, Italian, Japanese, Simplified Chinese and Arabic (right-to-left). The account language is used in emails. `pnpm i18n:check` reports missing strings.
- **Accessibility:** a screen-reader outline of the canvas, live announcements of selection and edits, full keyboard editing, and an accessibility checker (alt text, contrast, small text, link names, document language).
- **Install and mobile:** an installable PWA (offline app shell, update prompt, shortcuts), touch editing (pinch zoom, long-press menu, double-tap, larger handles), a phone layout with bottom sheets, and a Capacitor shell for iOS/Android in `apps/mobile`.
- **Content Planner:** schedule designs to social networks from the editor (**Schedule**) or **Home → Content Planner** (month/week calendar with drag-to-reschedule, list view, account filter). Pick pages (several = carousel), PNG/JPG/MP4, accounts, a caption with per-network character counts, hashtags suggested from the design's text, and a date/time in any time zone. The worker renders the export at the scheduled time, posts it, records per-account results and an activity log, retries transient errors with backoff, and notifies you in-app and by email. Accounts are connected in **Settings → Connected accounts** (OAuth 2.0, PKCE where supported); tokens are encrypted at rest (AES-256-GCM, key derived from `OC_SECRET`) and never sent to the browser. Without any network configured, the built-in **Download & remind** target emails you (and notifies you) with the rendered files and the caption to copy at the scheduled time.

  | Network | Env vars (enable it) | Redirect URI to register | Notes |
  | --- | --- | --- | --- |
  | Facebook Pages + Instagram | `OC_FACEBOOK_CLIENT_ID`, `OC_FACEBOOK_CLIENT_SECRET` (optional `OC_FACEBOOK_GRAPH_VERSION`, default `v23.0`) | `<OC_WEB_ORIGIN>/api/social/callback/facebook` and `…/callback/instagram` | Meta App Review for `pages_manage_posts`, `instagram_content_publish` (+ Business verification). Instagram needs a professional account linked to a Page; images are sent as JPEG. |
  | LinkedIn | `OC_LINKEDIN_CLIENT_ID`, `OC_LINKEDIN_CLIENT_SECRET` (optional `OC_LINKEDIN_VERSION`, `YYYYMM`) | `…/callback/linkedin` | "Share on LinkedIn" + "Sign In with LinkedIn using OpenID Connect" products. Tokens last 60 days; refresh tokens only for approved apps. Posts to the member's profile. |
  | X | `OC_X_CLIENT_ID`, `OC_X_CLIENT_SECRET` | `…/callback/x` | OAuth 2.0 PKCE, scopes incl. `media.write`. Posting needs a paid API tier. |
  | TikTok | `OC_TIKTOK_CLIENT_KEY`, `OC_TIKTOK_CLIENT_SECRET` (optional `OC_TIKTOK_MODE=inbox\|direct`, default inbox) | `…/callback/tiktok` | Content Posting API. Until TikTok audits the app, direct posts are private-only; inbox mode sends drafts the creator finishes in TikTok. Photo posts pull from our media URLs, whose domain must be verified with TikTok. |
  | YouTube | `OC_YOUTUBE_CLIENT_ID`, `OC_YOUTUBE_CLIENT_SECRET` (Google OAuth client) | `…/callback/youtube` | Videos only (vertical ≤ 3 min → Short). Unverified/unaudited projects can only upload private videos. |
  | Pinterest | `OC_PINTEREST_CLIENT_ID`, `OC_PINTEREST_CLIENT_SECRET` | `…/callback/pinterest` | Trial access is limited until Pinterest approves Standard access. Pins go to the chosen board (default: first board). |
  | Threads | `OC_THREADS_CLIENT_ID`, `OC_THREADS_CLIENT_SECRET` | `…/callback/threads` | Threads use case on a Meta app; App Review for `threads_content_publish`. |
  | Bluesky | none (app passwords); optional `OC_BLUESKY_SERVICE`, `OC_BLUESKY_DISABLED=1` | — | Images only (≤ 4, ≤ 1 MB each). |

  Instagram, Threads, Pinterest, TikTok photos and Facebook fetch media from signed, expiring links at `/api/social/media/…`, so `OC_WEB_ORIGIN` (or `OC_USERCONTENT_ORIGIN`) must be reachable from the internet for those networks.

- **Templates and fonts:** 188 original templates across all 27 size categories (CC0/Unsplash-licensed photos, Google Fonts, Noto art), the full Google Fonts catalogue (1,950 families, fetched on demand and pinned so every service renders identically) and automatic glyph fallback for emoji and every script, in the editor and in PDFs.
- **Graphics:** 12,000+ searchable icons and graphics (Lucide, Tabler, Phosphor, Gravity UI, originals) with per-colour recolouring; 66 shapes, 12 line ends, 72 frames (devices, letters, polaroid…) that also hold video.
- **Motion and presenting:** 18 entrance presets with direction/intensity, text by line/word/letter, looping emphasis, keyframes (position, scale, rotation, opacity, blur) with a record mode, 14 page transitions; click builds, autoplay/loop, laser pointer and blank screen in Present mode.
- **Data:** 16 chart types with number formats and CSV paste; Sheets (formulas, formatting, real-time, XLSX/CSV) with charts linked to sheet ranges; tables with merged and rich-text cells.
- **Workflow:** comments as tasks with due dates and reactions, team folders, access requests, ownership transfer, expiring/password view links, brand kits/templates/controls and approvals, a content planner that publishes to 9 networks (or emails a reminder), and print ordering (Gelato/Printful or a local print-ready file).
- **More surfaces:** whiteboard facilitation (timer, voting, sections, mind maps, follow/spotlight, reactions), Docs with design/chart blocks and DOCX/Markdown export and import, websites with mobile reflow, link-in-bio and privacy-friendly insights, and embeds (YouTube, Vimeo, Spotify, SoundCloud, Maps, Loom, Figma, CodePen).
- **Editing comfort:** Styles panel (palettes, font sets, replace colour everywhere), "/" command palette, familiar keyboard shortcuts, photo tone/detail controls and auto-enhance, video filters.

## Limitations

- **Auto captions** need whisper.cpp installed locally (`OC_WHISPER_CMD`, `OC_WHISPER_MODEL`); without it, captions are manual.
- **Not built yet:** image and video AI (background removal, text-to-image/video, video background removal), stock video and music, automated image classifiers, and a desktop app. Native iOS/Android projects are generated from `apps/mobile` with `pnpm --filter @oc/mobile add:ios`/`add:android`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Report security problems as described in [SECURITY.md](SECURITY.md).

## License

Pinselo contains code under two licenses. Each source file's SPDX header says which applies to it.

| Code | License |
|---|---|
| `apps/web`, `apps/mobile`, `packages/*`, `crates/oc-sheet` and the sheet code in `crates/oc-wasm` | [AGPL-3.0-only](LICENSE) |
| The other Rust crates | [Apache-2.0](LICENSE-APACHE) |
| `crates/vendor/krilla`, a patched third-party library | MIT or Apache-2.0; see its [notice](crates/vendor/krilla/NOTICE.md) and [patch list](crates/vendor/krilla/PATCHES.md) |

The web app includes AGPL code, so running Pinselo as a whole, including a modified version offered as a network service, is subject to the AGPL. That includes its requirement to offer your modified source code to the service's users.
