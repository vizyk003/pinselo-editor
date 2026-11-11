# MCP connector

Pinselo has no built-in AI. You bring your own assistant, such as Claude, ChatGPT or Codex, and connect it over the [Model Context Protocol](https://modelcontextprotocol.io). The assistant works in your account as a live collaborator, with the access you grant it, and its changes can be undone.

The connector is `POST /mcp` on your Pinselo server, for example `https://example.com/mcp`. The source is in `crates/oc-api/src/mcp/`, with OAuth in `crates/oc-api/src/oauth.rs` and `crates/oc-api/src/routes/oauth.rs`.

## Connecting an assistant

In the web app, the connect buttons and your exact URL are under **Settings > Developers > AI assistants**, in the editor's **AI** panel, and on the Home page.

| Assistant | How to connect |
|---|---|
| Claude (web and desktop) | Select **Add to Claude**. Check the prefilled name and URL and select Add, then Connect. Sign in to Pinselo and select **Allow**. |
| Claude Code | `claude mcp add --transport http pinselo https://example.com/mcp`, then type `/mcp` in Claude Code to sign in. |
| ChatGPT | Select **Copy URL and open ChatGPT**. On the Plugins page, select + and then Add custom MCP server, paste the URL, choose OAuth and continue. Sign in, select **Allow**, then create the plugin. ChatGPT offers custom MCP servers on some plans only. |
| Codex (CLI, IDE extension, app) | `codex mcp add pinselo --url https://example.com/mcp && codex mcp login pinselo`. Your browser opens, so sign in and select **Allow**. |
| Other MCP clients | Use the connector URL with OAuth, or send a [personal access token](#personal-access-tokens). |

Codex can also be configured in `~/.codex/config.toml`, followed by `codex mcp login pinselo`:

```toml
[mcp_servers.pinselo]
url = "https://example.com/mcp"
```

Hosted assistants connect from the internet, so Claude on the web and ChatGPT need your server at a public https address. Claude Code and Codex running on the same computer can use a local server.

Once connected, the editor's **Edit with AI** button copies a prompt that names the open design, ready to paste into your assistant.

## Authentication

`/mcp` accepts two kinds of bearer token:

| Token | Issued by | Use |
|---|---|---|
| `oc_oat_...` | The OAuth flow below | Normal for assistants. Works only on `/mcp`. |
| `oc_pat_...` | **Settings > Developers > API access tokens** | Clients without OAuth. Works on `/mcp` and [`/v1`](api.md). |

### OAuth flow

Pinselo is an OAuth 2.1 authorization server. MCP clients normally run all of this for you.

1. A request without a token gets `401` with this header:

   ```
   WWW-Authenticate: Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource/mcp"
   ```

2. The client reads that document (RFC 9728), which names the authorization server, then reads `/.well-known/oauth-authorization-server` (RFC 8414).
3. The client identifies itself in one of two ways:
   - **Dynamic registration (RFC 7591):** `POST /api/oauth/register`. The default `token_endpoint_auth_method` is `none`. `client_secret_post` and `client_secret_basic` return an `oc_cs_` secret.
   - **Client ID metadata document:** the `client_id` is an https URL of a JSON document whose `client_id` matches its own URL. It must have a `client_name` and use `token_endpoint_auth_method: none`. Pinselo fetches it with a 5 second timeout and a 64 KB limit, without following redirects, only from public addresses, and caches it for a day.
4. The client sends the user to `/oauth/authorize` with `response_type=code`, `client_id`, `redirect_uri`, an S256 `code_challenge`, and optionally `scope`, `state` and `resource`. PKCE with S256 is required.
5. The user signs in if needed and sees a consent page listing what the app may do. An app from a metadata document shows **Verified app from** its domain. A dynamically registered app shows a warning that Pinselo cannot verify who made it.
6. **Allow** redirects with `code`, `state` and `iss`. **Cancel** redirects with `error=access_denied`. Invalid requests never redirect.
7. The client exchanges the code at `POST /api/oauth/token`, using `grant_type=authorization_code`, `code`, `code_verifier` and client credentials. The body can be form-encoded or JSON.

```json
{ "access_token": "oc_oat_...", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "oc_ort_...", "scope": "designs:read designs:write assets:write exports" }
```

Rules the server enforces:

| Item | Rule |
|---|---|
| Authorization code | Valid for 5 minutes and single-use. Replaying a spent code revokes the tokens issued from it. A failed exchange does not spend the code. |
| Access token | Valid for 1 hour. |
| Refresh token | Valid for 60 days and rotated on every use. Reusing an old one signs out the whole connection. A refresh can keep or narrow scopes, never widen them. |
| Redirect URIs | 1 to 20 per client. https, or http on `localhost`, `127.0.0.1` or `[::1]`. No fragments or credentials. Must match exactly, except that loopback URIs may use any port. |
| `resource` (RFC 8707) | Must name this server's `/mcp` endpoint or its origin, otherwise `invalid_target`. |
| Consent POST | Must come from the server's exact origin. |
| Revocation | `POST /api/oauth/revoke` follows RFC 7009. Unknown tokens are ignored. |
| Unused clients | A client unused for 30 days with no live grant or pending code is deleted. |

### Scopes

| Scope | Consent page text |
|---|---|
| `designs:read` | See your designs and templates |
| `designs:write` | Create and edit designs |
| `assets:write` | Upload images to your account |
| `exports` | Export and download your designs |

All four are granted when a client asks for none. `openid`, `profile`, `email` and `offline_access` are ignored. Any other scope, including `webhooks`, is refused with `invalid_scope`.

### Personal access tokens

Create a token with the `designs:read`, `designs:write`, `assets:write` and `exports` scopes, and send it directly. The server requires an `Accept` header that includes both JSON and event streams:

```sh
curl https://example.com/mcp \
  -H "Authorization: Bearer $PINSELO_TOKEN" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

### Disconnecting

**Settings > Developers > Connected apps** lists each app with its scopes and last use. **Disconnect** revokes all of its tokens and pending codes straight away, and its export links stop working. The equivalent routes are `GET /api/oauth/connections` and `DELETE /api/oauth/connections/{client_id}`.

Revoking a personal access token has the same effect on that token's export links.

## Protocol

The transport is MCP streamable HTTP in stateless JSON mode: no sessions and no server-initiated streams.

| Item | Behaviour |
|---|---|
| Methods | `POST /mcp` only. `GET` and `DELETE` answer `405`. |
| JSON-RPC methods | `initialize`, `ping`, `tools/list`, `tools/call`. Others answer `-32601`. |
| Protocol versions | `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`, `2024-10-07`. |
| Required `Accept` | Must include `application/json` and `text/event-stream`, otherwise `406`. |
| Required `Content-Type` | `application/json`, otherwise `415`. |
| Batches | Up to 100 messages. `initialize` must be sent alone. |

Tool failures come back as a normal result with `isError: true` and a message the assistant can act on.

## Tools

Each call runs as the token's user, with the same design roles as the editor. Without the needed scope, the tool says which permission is missing. Unknown arguments are dropped, and invalid ones return an `Input validation error`.

### Discovery

| Tool | Scope | Arguments | Returns |
|---|---|---|---|
| `get_design_guide` | none | none | How designs, coordinates and operations work, plus layout advice. Assistants are told to read it once before editing. |
| `list_designs` | `designs:read` | `query` (up to 200 characters), `limit` (1 to 50, default 20) | Your own, team and shared designs, most recently edited first: `id`, `title`, `type`, `size`, `pages`, `updated`, `editUrl`. |
| `list_size_presets` | none | `query` (up to 80 characters) | Presets: `id`, `label`, `group`, `type`, `w`, `h`, `unit`. |
| `search_templates` | `designs:read` | `query` (up to 200 characters), `limit` (1 to 40, default 12) | Templates, including team templates, with `id`, `title`, `category`, `preset`, `size`, `tags` and `thumbnailUrl`. |
| `search_media` | `designs:read` | `query` (required, 1 to 100 characters), `type` (`image`, `gif`, `sticker`, `video`, `audio`; default `image`), `page` (1 to 20) | Up to 30 stock results from the providers enabled on the server, with `duration` for clips and sounds when the provider gives it. |
| `list_uploads` | `designs:read` | `query` (up to 100 characters), `limit` (1 to 100, default 30), `type` (`image`, `video`, `audio`, `all`; default `image`) | Your uploads: `asset`, `name`, `w`, `h`, `uploaded`; videos and sounds also have `kind` and `duration` (seconds), and sounds no size. |
| `get_brand_kit` | `designs:read` | none | See [Brand kits](#brand-kits). |
| `create_brand_kit` | `designs:write` | `name` (required), `team`, `colors`, `palettes`, `fonts`, `text_styles`, `voice`, `motion`, `logos`, `graphics`, `photos`, `guidelines` | The new kit, as `get_brand_kit` shows it. See [Making brand kits](#making-brand-kits). |
| `update_brand_kit` | `designs:write` | `brand_kit` (required), `new_name`, the fields of `create_brand_kit`, `add_logos`, `add_graphics`, `add_photos`, `remove_assets` | The changed kit, as `get_brand_kit` shows it. |
| `search_fonts` | none | `query` (up to 80 characters), `category` (`sans`, `serif`, `display`, `handwriting`, `mono`), `limit` (1 to 100, default 30) | Font `family` and `category`. An exact name match comes first, then by popularity. |

### Creating and reading

| Tool | Scope and role | Arguments | Returns |
|---|---|---|---|
| `create_design` | `designs:write` | `title` (required), `preset`, `width` and `height` (16 to 20000), `unit` (`px`, `mm`, `cm`, `in`, `pt`), `template_id` | `id`, `editUrl`, `design`. A unit other than px needs both width and height. |
| `copy_design` | `designs:write`, view | `design_id` (required), `title`, `preset`, `width`, `height`, `unit`, `scale_content` (default false) | `id`, `editUrl`, `resized`, `design`. When resized, also `check`, a layout check of up to 10 pages. |
| `bulk_create` | `designs:write`, view | `design_id` (required), `rows` (required, 1 to 50 objects of field to value), `titles` (up to 50) | `created` (each with `id`, `title`, `editUrl`, `filled`), plus optional `warning` and `check`. |
| `get_design` | `designs:read`, view | `design_id` (required), `page` (id or 1-based number) | `id`, `editUrl`, `title`, `type`, `size`, `pageCount`, `pages`, and `brand` when a team's kit governs the design. For a design that uses time, also `timeline`, page `duration` and `transition`, element `animation`, `visibleFrom` and `visibleTo`, and `videoCheck`. See [Video and motion](#video-and-motion). |
| `screenshot_page` | `designs:read`, view | `design_id` (required), `page` (default first), `width` (120 to 2000, default 1024), and one of `time` (seconds), `frames` (2 to 12) or `times` (up to 12 seconds) | A PNG image. With a time, the page at that moment of its playback; with `frames` or `times`, a filmstrip, four frames a row, and a line naming the times. |
| `export_design` | `exports`, view | `design_id` (required), `format` (required: `png`, `jpg`, `pdf`, `pdf-print`, `pptx`, `svg`, `mp4`, `gif`), `pages` (1-based), `scale` (0.25 to 4, default 1), `width` (MP4 only, 16 to 3840 px) | `file`, `size`, `downloadUrl`. |
| `edit_design` | `designs:write`, edit | `design_id` (required), `operations` (required, 1 to 200) | See below. |
| `undo_ai_changes` | `designs:write`, edit | `design_id` (required) | `restored`, `savedAt`, `editUrl`. See [Undoing AI changes](#undoing-ai-changes). |

`bulk_create` fills text only. In each copy, a text element bound to a column gets that field's value as its whole text. Any other field name is replaced literally in the text, matching case and whole words, so placeholders like `{name}` work. `warning` lists fields that matched nothing.

`copy_design` without `scale_content` keeps text and shapes at their size and repositions them. With a resize, give a preset or both width and height.

MP4 and GIF exports play only the pages given (default: all visible pages), each for its duration; the soundtrack and captions move with the first page given. An MP4 comes out at the design's own size times `scale`, or `width` px wide; the long side stays within 3,840 px.

`export_design` has no bleed, marks or colour profile arguments. For those, use [`POST /v1/designs/{id}/exports`](api.md#post-v1designsidexports).

### edit_design

Coordinates are px at 96 dpi from the page's top-left, and colours are hex (`#rrggbb` or `#rrggbbaa`). Operations run in order. A later operation can refer to an earlier new item as `"$name"` if that item was given `ref: "name"`. A page reference is a page id, a `$ref` or a 1-based number.

Every `add_*` operation except `add_audio`, and `update`, also take the time fields `animation`, `visibleFrom` and `visibleTo` (see [Video and motion](#video-and-motion)).

Add operations share these optional placement fields, except `add_line`, which takes only `page`, `parent` and `name`: `page`, `parent` (a group), `rotation`, `opacity` (0 to 1), `name`, and `role` (`headline`, `subhead`, `body`, `quote`, `caption`, `cta`, `logo`, `hero-image`, `background`, `decor`, `price`, `legal`).

| Operation | Required | Optional |
|---|---|---|
| `add_text` | `text`, `x`, `y`, `w` | `ref`, `h` (leave out to fit the text), `font`, `size`, `weight` (100 to 900), `italic`, `color`, `align`, `lineHeight`, `letterSpacing`, `case`, `list` (`bullet`, `number`) |
| `add_shape` | `shape`, `x`, `y`, `w`, `h` | `ref`, `fill`, `stroke`, `radius` |
| `add_line` | `x1`, `y1`, `x2`, `y2` | `ref`, `color`, `width`, `dash`, `start`, `end` |
| `add_image` | `source`, `x`, `y`, `w`, `h` | `ref`, `fit` (`cover` by default, or `contain`), `radius`, `alt` |
| `add_table` | `data`, `x`, `y`, `w` | `ref`, `h`, `font`, `size`, `color`, `headerFill`, `headerColor` |
| `add_chart` | `chart`, `labels`, `series`, `x`, `y`, `w`, `h` | `ref`, `showLegend`, `showValues`, `font`, `textColor` |
| `add_qr` | `data`, `x`, `y`, `size` | `ref`, `color`, `background` |
| `add_video` | `source`, `x`, `y`, `w`, `h` | `ref`, `fit`, `trim` (`{ start, end }` in seconds of the clip), `muted`, `loop`, `volume` (0 to 1), `radius` |
| `add_audio` | `source` | `ref`, `start`, `trim`, `duration` (default: to the end of the video, looping if shorter), `volume`, `fadeIn` (default 0), `fadeOut` (default 0.8), `loop` |
| `update_audio` | `id` | `start`, `trim`, `duration`, `volume`, `fadeIn`, `fadeOut`, `loop`, `fit` (to the end of the video as it is now) |
| `remove_audio` | `id` | |
| `set_captions` | | `cues` (`[{ start, end, text }]`, replaces the track's cues), `style` (`font`, `size`, `color`, `background`, `position`), `visible` |
| `animate` | `ids` | `in`, `out`, `start`, `stagger` (default: the kit's, else 0.15 s), `visibleFrom`, `visibleTo` |
| `group` | `ids` (at least 2) | `ref`, `name` |
| `ungroup` | `id` | |
| `update` | `id` | `x`, `y`, `w`, `h`, `rotation`, `opacity`, `name`, `role`, `hidden`, `text`, text style fields, `fill`, `stroke` (`null` removes it), `radius`, `image`, `alt`, time fields, and for videos `muted`, `loop`, `volume`, `trim` |
| `delete` | `ids` | |
| `reorder` | `id`, `to` (`front`, `back`, `forward`, `backward`) | |
| `arrange` | `ids`, `direction` (`row`, `column`, `grid`) | `gap`, `columns`, `within` (`page`, `items`), `padding`, `x`, `y`, `width`, `height`, `align`, `justify`, `stretch` |
| `align` | `ids`, `to` (`left`, `center`, `right`, `top`, `middle`, `bottom`) | `relativeTo` (`page`, `selection`) |
| `distribute` | `ids` (at least 3), `axis` (`horizontal`, `vertical`) | |
| `add_page` | | `ref`, `after`, `duplicate`, `title`, `background`, `duration`, `transition` |
| `update_page` | `page` | `title`, `background`, `notes`, `duration`, `transition` |
| `delete_page` | `page` | |
| `move_page` | `page`, `after` (`null` makes it first) | |
| `set_title` | `title` | |
| `replace_text` | `find`, `replace` | `match_case`, `whole_word`, `page`, `ids` |

Value formats:

- **`fill`, `background`:** a hex colour, or `{ "gradient": [2 to 8 colours], "angle": n, "radial": bool }`.
- **`stroke`:** `{ "color": hex, "width": 0 to 500, "dash": [numbers] }`.
- **`source`, `image`:** one of the forms below.

| Source | Meaning | Needs `assets:write` |
|---|---|---|
| `{ "url": "https://..." }` | Downloaded and imported. Public https hosts only. | Yes |
| `{ "stock": { "provider", "id", "type" } }` | A result from `search_media`. | Yes |
| `{ "asset": "<id>" }` | An asset already in the design, or one you or your team own. | No |
| `{ "brand": { "kit", "name" } }` | A file in a brand kit, by file name. | No |

`shape` and `chart` accept the kinds listed by `get_design_guide`, for example `rect`, `ellipse`, `star`, `heart` and `column`, `line`, `pie`, `radar`.

The result:

```json
{
  "applied": "3 of 3 operations",
  "refs": { "title": "Sabc123" },
  "created": ["Sabc123", "Sdef456"],
  "check": "No layout problems found (text fits, stays on the page and is readable).",
  "next": "Look at it with screenshot_page if you want to judge the design itself."
}
```

| Field | Meaning |
|---|---|
| `applied` | How many operations ran. |
| `refs`, `created` | Ids of new items. |
| `replaced` | Matches changed by `replace_text`. |
| `error` | Processing stops at the first failed operation. Earlier ones stay applied. `isError` is true only if none were applied. |
| `warning` | The change was sent, but saving was not confirmed within 10 seconds. |
| `check` | Layout problems on the touched pages, as `{id, problem, message}`, such as text that does not fit or runs off the page, poor contrast and overlaps. Left out if the render service does not answer in time. |
| `next` | A suggested next step. |

Arguments are validated before anything runs, and images are imported before the edit starts. A bad image or a brand control violation means no operation is applied.

### Video and motion

Times are in seconds. Pages play one after another in videos, GIFs and auto-playing presentations.

- **Page timing:** `duration` (0.1 to 600; `null` is automatic: 5 s, or as long as the page's videos) and `transition` (`{ type, duration, direction }`; `type` is a page transition such as `fade`, `slide`, `push` or `wipe`, and `none` or `null` cuts). A transition is how the page comes in from the one before.
- **`animation`:** `{ in, out }`; `null` removes both, and `null` for either removes that one.
  - `in`: `type` (`fade`, `rise`, `drop`, `slide-left`, `slide-right`, `slide-up`, `slide-down`, `scale`, `zoom`, `pop`, `wipe`, `draw`, `typewriter`, `blur`), `delay` (from when the element appears), `duration`, `easing` (`linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`), `direction`, `distance` (px of travel for `rise`, `drop` and slides), and for text `by` (`element`, `line`, `word`, `letter`) with `stagger`. `draw` reveals a line from its first point to its last.
  - `out`: `type` (`fade`, `rise`, `drop`, `slide`, `scale`, `zoom`, `wipe`, `blur`), `at` (the page second it starts, which sets `visibleTo` to `at + duration`) and `duration`.
- **`visibleFrom`, `visibleTo`:** the page seconds an element appears and disappears (`null` clears), so texts can take turns on one page. Give `visibleTo` or `out.at`, not both.
- **Defaults:** without a brand kit, an entrance is `fade` for 0.5 s (`draw` 0.6 s) with an ease-out, an exit 0.4 s, a transition `fade` for 0.4 s. A kit's `motion` replaces these (see below).

`edit_design`'s `check` and `get_design`'s `videoCheck` add these problems for designs that use time:

| `problem` | Meaning |
|---|---|
| `text_too_brief` | Text can't be read in the time it is fully on screen, at about 3 words a second plus half a second. |
| `caption_too_brief` | The same for a caption cue. |
| `outside_safe_zone` | In a 9:16 video, text, a QR code or an element with a text, logo, button or price role reaches into the top 14% or the bottom 35%, which Reels covers with its interface. Stories cover 14% at each end. |
| `never_shown`, `cut_by_page_end` | An element appears after its page ends, or its exit is cut off by the page end. |
| `video_too_long` | A 9:16 video over 60 s (a Stories card), or a 4:5 or square video over 15 s (Meta's advice for feed ads). |
| `audio_ends_early` | The soundtrack stops more than half a second before the video ends. |

A brand kit's `motion` sets the defaults, in milliseconds and px: `enter` (`type`, `duration`, `easing`, `distance`), `text` (the same, for text), `draw` (`duration`, `easing`), `exit` (`type`, `duration`), `stagger` (between elements in `animate`), `transition` (`type`, `duration`), `bounce` (`false` refuses `pop`) and `note`. For example `{ "transition": { "type": "fade", "duration": 150 }, "draw": { "duration": 400 }, "text": { "type": "rise", "distance": 8 }, "stagger": 60, "bounce": false }`. The design's kit is the one it was made with, else its team's default kit. `get_brand_kit` and `get_design` show it.

`screenshot_page` with a time draws animations, timing and captions as in the video. Videos show their poster frame, and page transitions are not drawn.

## How edits reach the editor

The assistant joins the design as a live collaborator. Anyone with the design open sees its avatar, a **{name} is editing** label and its changes as they happen.

The server keeps that connection open for a minute after the last change. If the collaboration server cannot be reached within 5 seconds, changes are written to the stored document instead. Everything still works, but without presence.

## Brand kits

`get_brand_kit` returns every kit in your personal workspace and your teams:

| Field | Content |
|---|---|
| `name`, `team`, `default` | The kit's name, its team (not shown for personal kits) and whether it is the default. |
| `colors`, `palettes`, `fonts` | Brand colours, palettes and fonts by role. |
| `logos`, `graphics`, `photos` | `{asset, name}` entries, capped at 30, 60 and 30 per kit. |
| `voice` | Brand voice notes, when set. |
| `motion` | How the brand moves, when set. See [Video and motion](#video-and-motion). |

### Making brand kits

Assistants can set up a brand from a brand book, design system, website or files with `create_brand_kit`, and change it later with `update_brand_kit`. The same rules and limits apply as in Brand Hub:

- The kit goes in your personal workspace, or in the team named by `team` if you are an owner, admin or designer there. `team` matches like `kit` below.
- Kit names must be unique among the kits you can see, since tools find kits by name.
- `logos`, `graphics` and `photos` take asset ids of your own image uploads. Assistants upload each file first with `upload_image` or `create_upload_link`. Ids of anyone else's files are refused.
- Colours are `#rrggbb`. `text_styles` take the fields `get_brand_kit` shows (`name`, `role`, `weight`, `italic`, `uppercase`, `letterSpacing`, `lineHeight`, `size`, `colors`, `note`). `guidelines` are `{ title, body }` sections, up to 60 and 300,000 characters in all.
- In `update_brand_kit`, every field given replaces the kit's, except `fonts`, which changes only the roles given (`""` clears one). `add_*` appends images and `remove_assets` takes them out of all three lists.
- Font files can't be uploaded through the connector. A kit font that isn't uploaded shows in `missingFonts`, and the user uploads it in Brand Hub.

Changes are recorded in the team's audit log like Brand Hub edits, marked as made by an AI assistant.

To place a kit file, use `add_image` with `{ "asset": id }` or `{ "brand": { "kit", "name" } }`:

- `kit` matches an exact name ignoring case, or the only kit whose name contains the text. Otherwise the error lists your kits.
- `name` compares only the file name, ignoring case, so a path like `assets/underlines/wave.png` also works. A wrong name lists the kit's files.

The server instructions and guide tell assistants to use the matching kit's real logo and graphics, not to draw imitations of them, not to invent quotes, people, prices or figures, and to say so if they use an image from outside the kit. These are instructions to the assistant, which the server cannot enforce.

Two rules are enforced on the server:

- **Brand controls:** when a team kit governs a design, `get_design` includes `brand` with its kits, fonts by role and colours (up to 40). If the team has **Brand colours only** or **Brand fonts only** on and you are not an admin, `brand.enforced` is set and `edit_design` rejects other colours and fonts, applying nothing. An empty kit never locks anyone out.
- **Locks:** `get_design` marks elements the user locked `"locked": true`, and assistants cannot change them.

## Undoing AI changes

Before an assistant's first change in a session, Pinselo saves a checkpoint version named "Before {assistant}'s changes". A new checkpoint is saved after that assistant has been quiet on the design for 30 minutes.

| Where | What happens |
|---|---|
| Editor | A bar offers **Undo {name}'s changes** for a day after the session. Press ⌘Z or Ctrl+Z afterwards to bring the changes back. |
| Assistant | `undo_ai_changes` restores the latest checkpoint. It does not save a new one, so calling it twice restores the same state. |
| Version history | Checkpoints are versions of the design. See [the user guide](user-guide.md#version-history). |

Both undo paths restore the whole design to the checkpoint, so changes anyone made since then are undone too. The latest checkpoint is used whichever assistant saved it.

## Limits

| Limit | Value |
|---|---|
| Requests to `/mcp` | 240 per minute, per client IP and token. |
| Failed token lookups | 20 per minute per IP, then `429`. Tokens that resolved in the last 10 minutes are not blocked. |
| `/api/oauth/register` | 20 per minute. |
| `/api/oauth/token`, `/api/oauth/revoke` | 60 per minute each. |
| Operations per `edit_design` call | 200. |
| Listed per page by `get_design` | 400 elements, with text clipped at 2,000 characters. `omitted` counts the rest. |
| Rows per `bulk_create` call | 50. It checks the first three copies. |
| Tables in `add_table` | 100 rows, 30 cells a row, 2,000 characters a cell. |
| Charts in `add_chart` | 12 series, 200 labels or values each. |
| Export time | 5 minutes. |
| Screenshot time | 2 minutes. |
| Export links | Valid for 24 hours, tied to your user and grant. They can be opened without signing in and are not single-use. |

`screenshot_page`, `export_design` and the `check` field need the render service. Without it, those two tools answer "The render service is not running." See [self-hosting.md](self-hosting.md).

See also [api.md](api.md) for the REST API and [user-guide.md](user-guide.md) for the editor.
