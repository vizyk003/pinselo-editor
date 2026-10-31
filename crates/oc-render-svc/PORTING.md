# services/render → crates/oc-render-svc

> **The TypeScript render service (`services/render`) was deleted in plan 13 stage 4 (2026-10-27; the last commit that has it is `aa18983`).**
> The goldens it wrote stay in `tests/golden` as frozen records of its behaviour, and this file records how the port
> was verified. New behaviour is written and tested in Rust only.

Port status of the render service (ADR-0001, phase 5). The Rust binary is a drop-in: same port
(`OC_RENDER_PORT`, bind host `OC_RENDER_HOST`, default `127.0.0.1`), same routes, the internal service token
(`x-oc-internal`, 403 `{ error: 'forbidden' }` without it), 1 MiB body limit, the `export` job queue
(concurrency 1), the `render` heartbeat, the same SQL and the same outputs. Renders are serialised like the TS
`serial` queue.

Parity is pinned by `tests/golden/render-svc/`, written by `tools/golden/render-svc.ts` (`pnpm golden render-svc`;
deterministic and offline — `Date` pinned, `Math.random`/`getRandomValues` seeded, fixed Y.Doc client ids,
`fetch` disabled), and for video/GIF/audio by `tests/golden/render-video/`, written by `tools/golden/render-video.ts`
(`pnpm golden render-video`; needs ffmpeg, deterministic: the synthetic renderer is integer arithmetic, the test
media are encoded once with bitexact flags and kept).

```sh
CARGO_TARGET_DIR=target/wf-rsvc cargo test -p oc-render-svc                       # all parity + no-DB service tests
CARGO_TARGET_DIR=target/wf-rsvc cargo test -p oc-render-svc --test video          # video/GIF/audio parity (ffmpeg)
OC_DATABASE_URL=postgres://localhost:5432/opencanvas CARGO_TARGET_DIR=target/wf-rsvc \
  cargo test -p oc-render-svc --test service                                      # + every route on a throwaway DB
```

## Modules

| TS | Rust | status | parity |
|---|---|---|---|
| `main.ts` (routes, job, data access) | `app.rs`, `service.rs`, `main.rs` | done | `tests/service.rs`: guard/health without a DB; with `OC_DATABASE_URL`, every route and the export job end to end on an `oc_rust_test_*` DB (always dropped) |
| `docMarkdown.ts` | `doc_markdown.rs` | done | byte-identical on 14 hand-made docs (every block type, marks, escapes, edge cases), with and without an origin; also read back from the stored Yjs state through oc-crdt |
| `docx.ts` (the `docx` npm package) | `docx/` | done | every XML part identical after normalisation (attributes sorted, hyperlink relationship ids renamed, core.xml dates masked) on 12 docs + 2 error cases (same error messages, same deps calls in the same order); media sha256 identical; the Rust .docx read back by **mammoth** on the TS side gives the same HTML, text and messages |
| `docLayout.ts` `blocksOf` / `docHeadings` | `doc_layout.rs` | done | identical blocks (incl. the TS's invalid-colour errors) |
| `docLayout.ts` `docToDesign` | `doc_layout.rs` | done | with the real painter: same pages, element order, types, texts, assets and boxes as SkParagraph (see deviations for one environment effect) |
| `docExport.ts` | `service.rs` | done | `pickPage`, `docBodyOf`, embed pictures (allow-list, generated assets, 64-entry cache), chart PNGs, Word/Markdown wiring — covered by the DB test |
| `siteReflow.ts` (+ `siteReflow.test.ts`) | `site_reflow.rs` | done | identical items, plans, reading orders, rows on all 47 pages of the 9 website fixtures, 2 hand-made sites, the 10 render scenes and 60 seeded random layouts; `contains`/`mobileFontSize`/`shouldReflow` tables; all 13 vitest cases ported |
| `siteMobile.ts` | `site_mobile.rs` | done | phone layout HTML, reading order, text mirror, `textHtml`, `paintCss`, `textKind`, `maxRunSize` byte-identical on 47 pages |
| `site.ts` | `site.rs` | done | full `buildSite` HTML identical (inner SVG content masked) with the renderer replayed, and with the real painter (`@font-face` lines aside, see deviations); `scopeSvgIds`, `describe`, `formHtml`, semantic tags |
| `pptx.ts` | `pptx/` | done | every XML part of 28 decks (15 presentation fixtures, 10 render scenes, 3 hand-made: transitions, hidden slides, image/gradient backgrounds, curved/shrink text, links, frames with and without content/chrome, recoloured SVG, edited images, videos, tables, groups, polylines, CMYK, out-of-range page sizes) identical after id renumbering, both replayed and with the real painter; media identical; the element pictures requested (page, scale, skip set) identical |
| `email.ts` | `email.rs` | done | HTML and plain text identical on 7 layouts (4 email fixtures, link edge cases, 2 scenes), replayed and with the real painter |
| `pdfVector.ts`, `exporters.ts` `exportRasterPdf` | `oc_renderer.rs` → `oc_render::pdf::write_pdf` | done (oc-render's writer, not a port) | DB test checks the files; link areas computed as the TS does (`link_areas`) |
| `exporters.ts` `exportSvg` | `oc_renderer.rs` → `oc_render::svg` | done | fonts embedded (picked faces), assets inlined |
| `exporters.ts` `frames` (timeline frame loop) | `video.rs` `frames` | done | `tests/video.rs` plans: the renderer calls in order (page, page time, caption time, scale, the bytes of every video frame handed to the painter) identical on 3 template decks given timing, transitions of all 14 kinds and directions, entrance/emphasis/keyframe/exit/click animations and a hidden page, and on a video design (video elements with trims, speeds 0.25–3, loops, mute, volume; a frame and a grid cell playing video; missing media; hidden elements and pages; captions); every frame written to the encoder identical (8×6 colour grid + checksum of a pixel sample) where rendered, within 1.0 mean / 8 max (of 255) where composed by a transition (measured: worst 0.47 / 2, on the blur/spin pages; the rest ≤ 0.01 / 1); progress calls identical |
| `exporters.ts` `composeTransition` | `oc_renderer.rs` `compose_frames` → `oc_render::raster::transition::draw_transition` | done | the transition frames above (CanvasKit `drawTransition` vs oc-render, same `transitionFrame`) |
| `media.ts` `VideoDecoder` | `media.rs` `VideoDecoder` (ffmpeg → raw RGBA → `RasterOptions::video_frames` → oc-render `VideoFrames`) | done | identical decoder command lines (start time, fps, size, `libvpx-vp9` for alpha WebM), restarts on backward seeks / loops, and identical frame bytes reaching the painter |
| `media.ts` `audioMixArgs` | `media.rs` `audio_mix_args` | done | identical inputs and `-filter_complex` graphs (soundtrack trims, volumes, fades, offsets, clips past the end or without media; video audio with speed → `atempo` chains, loops expanded, mute, `hasAudio`), for the MP4 (first input 1) and M4A (first input 0) |
| `exporters.ts` `exportMp4` | `video.rs` `export_mp4` | done | identical encoder (`-s`, `-r`, odd-size crop, x264 settings) and mux command lines at 720p and 1080p; real export with the oc-render painter and ffmpeg: same ffprobe stream layout (h264 yuv420p 30/1 + aac 48 kHz stereo) and duration as the TS file, decoded frames mean ΔE ≤ 1 (measured ≤ 0.04) |
| `exporters.ts` `exportGif`, `gifenc` (npm) | `video.rs` `export_gif`, `gif.rs` (port of gifenc 1.0.3 `quantize`/`applyPalette`/`GIFEncoder`) | done | gifenc: palettes (2/16/64/256 colours, incl. the no-sqrt weighting path), indices and multi-frame GIF bytes identical on 9 fixed RGBA inputs; `exportGif` over the synthetic renderer: the file byte-identical (palette reuse key, 12 fps, delays) |
| `exporters.ts` `exportAudio` | `video.rs` `export_audio` | done | identical command lines; real export: same ffprobe layout and duration; silent designs → `None` (`400 no_audio` / job error) |
| `main.ts` `mediaFor` | `service.rs` `media_for` | done | DB test: proxies of video slots and soundtrack clips (`width \|\| 2`, `proxy_mime = video/webm` → alpha), MP4/GIF/M4A routes and an MP4 job with progress |
| `main.ts` `recoloredSvgRaster` | `service.rs` | done | via `Renderer::rasterize_svg` (resvg) |

## Endpoints

| route / format | status |
|---|---|
| `GET /healthz` | done |
| `POST /internal/thumbnail` (coalesced 1.2 s → `designs.thumb_sha256`) | done |
| `POST /internal/doc-to-design` | done |
| `POST /internal/page-png` | done |
| `POST /internal/site-build` | done |
| `POST /internal/template-thumbnail` | done |
| `POST /internal/export` `md`, `docx` | done |
| `POST /internal/export` `png`, `jpg`, `zip`, `svg`, `pdf`, `pdf-print`, `pdf-flat`, `pptx`, `html-email` | done |
| `POST /internal/export` `mp4`, `gif`, `m4a` | done (`m4a` of a silent design: `400 { error: 'no_audio' }`) |
| `export` job `docx`, `pdf`, `pdf-print` (+ product bleed 0–10 mm), `pdf-flat`, `pptx` | done (`{ sha, type, filename, size }`, `export.completed` webhook) |
| `export` job `mp4` (`quality` 720p/1080p/4k), `gif`, `m4a` | done (progress written to the job row, throttled to 300 ms and capped at 0.99 like the TS) |
| `pdf-print` `print` options: marks, colour bars, `standard` (PDF/X-4, PDF/X-1a), ICC `profile`, `intent`, `flatten` | **Rust only** (`src/print.rs`; the TS service ignores them and stays RGB) |
| `POST /internal/icc/inspect`, `POST /internal/print/gamut` | **Rust only** (moxcms via crates/oc-color) |

## The Renderer trait

`renderer.rs` — every painter call the exporters make (implemented by `OcRenderer` on crates/oc-render;
`StubRenderer` answers 501 everywhere):

| method | replaces (TS) |
|---|---|
| `render_rgba(host, doc, page, RasterOptions)` | `renderer.renderRgba` (+ `painter.videoFrames` via `RasterOptions::video_frames`) |
| `render_image(host, doc, page, RasterOptions, ImageFormat)` | `renderer.render` (PNG/JPEG) |
| `compose_transition(prev, next, TransitionSpec)` | `composeTransition` / `drawTransition` (video) |
| `vector_pdf(host, doc, pages, VectorPdfOptions)` | `exportVectorPdf` |
| `raster_pdf(host, doc, pages, RasterPdfOptions)` | `exportRasterPdf` |
| `page_svg(host, doc, page, SvgOptions)` | `displayListToSvg(resolvePage(...))` |
| `elements_svg(host, doc, page, ids, SvgOptions)` | `displayListToSvg` of a display-list subset (`svgOf`) |
| `text_faces(host, doc, page)` | `TextEngine.faces` over a page's text items |
| `pick_face(host, FaceRequest)` | `fonts.pickFace` |
| `layout_text(host, body, width)` | `painter.text.layout` + `getLineMetrics` |
| `encode_png(rgba)` | `ck.MakeImage(...).encodeToBytes(PNG)` |
| `transcode_png(bytes)` | `ck.MakeImageFromEncoded(...).encodeToBytes(PNG)` |
| `rasterize_svg(svg, width)` | `new Resvg(svg).render().asPng()` |
| `path_commands(d)` | `ck.Path.MakeFromSVGString(d).toCmds()` |

`RenderHost` (implemented by `Service`): `asset_bytes(key)`, `font_file(file)`, `custom_families(families)`,
`manifest()`, `fonts_dir()`.

`pnpm dev` runs this service (tools/scripts/dev.ts builds it in release into `target/dev`, with the other Rust
services). The same crate holds `oc-templates` (`src/bin/oc-templates.rs`), the template catalogue's seeder and
preview renderer (`pnpm seed`, `pnpm tpl:preview`).

## Deviations

- **Mark order from the CRDT.** yrs keeps a text run's formatting attributes in a hash map, so oc-crdt returns
  marks sorted by type; yjs keeps insertion order. Word output only depends on mark order when a run has both
  `code` and `highlight` (whose shading wins) or `code` and a `textStyle` font (whose font wins). Markdown is
  order-independent. The goldens feed the TS-read body to both sides.
- **Concurrency order.** The TS renders a website row's nodes and a PowerPoint group's children with
  `Promise.all`, so the `s<page>c<n>` SVG id prefixes and the shape/relationship ids and media names inside groups
  follow promise timing (in production, database latency). Rust builds them in paint order. Ids stay unique; the
  tests compare after renumbering.
- **Hyperlink relationship ids** in .docx are `rIdl<n>` (the `docx` package uses random nanoids).
- **Unknown element types** keep their stored `xf` for world bounds (`geom.rs`), as the TS does; oc-geometry gives
  untyped elements no transform.
- **`pdf-flat`** is one picture per page (150 dpi PNG) with link annotations and an invisible text layer, written by
  oc-render's PDF writer (`PdfOptions::invisible_text`): every text op of the page (with its transforms and clips)
  in the faces it was laid out with, glyph for glyph, at fill opacity 0. The TS layer was one Helvetica string per
  line, stretched to the line's width (WinAnsi only: other characters became `?`), for text elements only; ours
  also covers table/chart/form text and any script. `tests/parity/live.rs` extracts it with pdftotext.
- **Soundtrack `loop`.** `audioMixArgs` ignores `AudioClip.loop` (a clip plays once for `min(durationMs, media length −
  trim)`); so does the port, to keep the filter graphs identical. Video loops are expanded as in the TS.
- **Degenerate inputs.** A looping video clip of length 0 would make the TS audio loop spin forever; the port adds it
  once. An unknown job `quality` is 1080p (the TS computed a NaN scale). A decoder that cannot start shows the poster
  (the TS saw a process that closed without data — the same result).
- **Video frames reach the painter by playback key** for the duration of one render (`RasterOptions::video_frames`),
  instead of a `videoFrames` callback set on the long-lived painter.
- **PDF/SVG/raster drawing** is oc-render's (see crates/oc-render/PARITY.md: mean ΔE 0.11 against CanvasKit; its
  PDF keeps gradient opacity, embeds font subsets).
- **Doc pagination** lays text out with the faces it needs loaded first; the TS measured with whatever fonts the
  long-lived renderer had loaded (the generator preloads them to match).
- **Fallback faces (oc-text).** `oc_text::fallback_families` does not report Noto Color Emoji for some symbols
  @oc/text does (★ U+2605: `Extended_Pictographic` in V8's data, not in ICU4X's), so a website's `@font-face` list
  can lack that face. The live site test compares everything but the `@font-face` lines; the replay test pins
  their logic.
- In the golden run (offline) CanvasKit measured ☐ with some loaded face while oc-text uses notdef metrics: those
  task-list lines are 2 px apart in the doc layout test, which allows exactly that.

## Remaining

Nothing. (Not covered by a golden: the alpha WebM decode path runs the same code with `-c:v libvpx-vp9` added, which
the unit test pins; 4k export only changes the scale.)
