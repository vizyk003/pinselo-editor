# oc-render pixel parity

> **Historical record.** This is how oc-render was checked against CanvasKit before the TS renderer was deleted
> (plan 13 stage 4; `tools/render-parity.ts` and `tests/parity.rs` went with it, the last commit that has them is
> `aa18983`). The renderer is now guarded by the PS-1 baselines: `tests/ps1.rs` (native) and
> `packages/engine/test/ps1.test.ts` (WebAssembly) against `tests/golden/ps1/baselines.json`.
How closely the tiny-skia painter reproduces the CanvasKit renderer it replaces (`@oc/render-canvaskit`, as the
editor and the render service draw today). Pixel-identical output is not the goal — the PS-1 baselines are
regenerated at cutover — but every *systematic* difference (missing effects, blend modes, gradient geometry,
stroke alignment, colour handling, image fit, z-order, clipping) was hunted down until what remains is
rasterisation-level noise.

## Method

```sh
pnpm tsx tools/render-parity.ts                          # CanvasKit references → target/render-parity/
cargo test -p oc-render --test parity -- --nocapture     # tiny-skia renders, compares, writes the report
```

`tools/render-parity.ts` renders, with the existing node renderer (`createNodeRenderer`, pinned fonts only, no
network):

- every page of every `fixtures/templates-seed/*.json` (306 pages, scaled to at most 1600 px),
- the hand-made scenes of `tools/golden/render-scenes.ts` (every element type, shape kind, paint, stroke,
  marker, effect, blend mode, text effect, image edit and format, frame, grid, chart, table, form, embed, QR code,
  animation times, captions: 28 pages),
- page transitions of every kind at two progress values (28 frames),

and, for a sample of 25 static pages, the vector PDF of today's exporter (`services/render/src/pdfVector.ts`).
The Rust test renders the same pages from the same documents and asset bytes and compares, over white:

- **ΔE**: CIE76 colour difference in CIELAB per pixel — mean, 95th percentile, and the share of pixels with
  ΔE > 10 (a clearly visible difference);
- **SSIM**: mean structural similarity of L* over 8×8 windows.

`target/render-parity/report.md` lists every page (worst first) and `diff/<page>.png` shows the worst ones side
by side (CanvasKit | tiny-skia | ΔE heat map). The test also guards against regressions: mean ΔE over all pages
≤ 0.25, and per page mean ΔE ≤ 1.5, SSIM ≥ 0.95 and ΔE > 10 on at most 3 % of the pixels. Without the references
(CI), the test reports that and passes.

## Result

362 pages:

| | pages | mean ΔE | p95 ΔE (mean) | pixels ΔE > 10 | SSIM |
|---|---|---|---|---|---|
| templates | 306 | 0.119 | 0.234 | 0.166 % | 0.9988 |
| scenes | 28 | 0.114 | 0.290 | 0.205 % | 0.9985 |
| transitions | 28 | 0.053 | 0.143 | 0.065 % | 0.9997 |
| **all** | **362** | **0.114** (median 0.072, max 0.77) | **0.231** | **0.161 %** (max 1.19 %) | **0.9988** (min 0.977) |

Six pages have a mean ΔE above 0.5 and two an SSIM below 0.99. A mean ΔE of 1 is about the smallest colour
difference a viewer can notice side by side.

### Worst pages and why

| page | mean ΔE | SSIM | cause |
|---|---|---|---|
| tpl-mw-web-restaurant-lume-p1 | 0.77 | 0.997 | 2000 px photo (with a `warm` filter) under a dark gradient: resampling texture |
| tpl-pr-flyer-open-house-p1 | 0.70 | 0.977 | large photo downscaled 3–4×: mip sampling of fine brick texture |
| tpl-sa-ig-quote-landscape-p1 | 0.67 | 0.998 | 2000 px photo, fine detail |
| tpl-sb-fbc-cafe-photo-p1 | 0.67 | 0.997 | photo |
| tpl-sb-fb-retreat-escape-p1 | 0.66 | 0.997 | photo |
| tpl-vd-thumb-travel-p1 | 0.61 | 0.997 | photo |
| scene-images-c0-p1 | 0.50 | 0.993 | SVG assets drawn sharper (see deviations); tone/clarity evaluated per image pixel; Skia's clamp-blur corner artefacts |
| scene-text-c0-p1 | 0.42 | 0.991 | glyph rasterisation; stroked (hollow/outline), curved and rotated text are unhinted paths in both but rasterised differently |
| tpl-mw-eh-welcome-plants-p1 | 0.27 | 0.986 | 512 px emoji artwork downscaled 4×: sharp edges under trilinear sampling |

In every case the structure is identical (same positions, same z-order, same clips); the heat maps show only
edges and photographic texture. What remains is inherent to two rasterisers: tiny-skia's coverage vs FreeType's
glyph rasteriser, skrifa's light autohinter vs FreeType's, 8-bit mip levels, and colour matrices applied to the
decoded image (once, before resampling) rather than per sample in Skia's pipeline.

## Systematic differences found and fixed

| step | mean ΔE | ΔE > 10 | SSIM |
|---|---|---|---|
| first complete painter | 0.299 | 0.861 % | 0.9931 |
| trilinear mipmaps with `SkMipmap::ComputeLevel`'s −0.5 "sharpen" bias, on the more minified axis | 0.280 | 0.833 % | 0.9939 |
| glyphs hinted like CanvasKit's FreeType scaler under SkParagraph's `kSlight` (light autohinter, up to 256 px), origins quantised like Skia's glyph masks (x to ¼ px, y to whole px) | 0.187 | 0.581 % | 0.9978 |
| Skia's A8 glyph-mask gamma/contrast correction (`SkTMaskGamma`, contrast 0.2, sRGB), also for unblurred text shadows | **0.114** | **0.161 %** | **0.9988** |

Also fixed along the way: highlight boxes use the run's own half-leading line box (not the whole line);
`edits.adjust.blur` extends the clamped edge pixels over the blur's reach with hard edges like Skia's
`TileMode::Clamp` image filter; clamp bounds are the content's rounded pixel bounds.

Checked and already identical: all 16 blend modes (separable and non-separable), opacity layers, drop shadow
geometry (offset in element space, σ = blur/2), layer blur, gradient angles/extents and stop handling (unsorted
and out-of-range stops), radial gradients, image fit/crop/tile, stroke alignment (inside/outside as clipped
double-width strokes), caps, joins and dashes, line markers, clipping and nesting, QR codes (non-anti-aliased),
video posters and placeholders, colour matrices of every filter preset, duotone, vignette, text effects
(background, hollow, outline, shadow, neon, echo, glitch), list markers, decorations, curved text, captions,
premultiplied compositing in sRGB space (no linear blending, like CanvasKit's legacy colour handling).

## Deviations (intended)

- **SVG assets** are rasterised by resvg at the size they are drawn (the TS host rasterised them at the asset's
  natural width and CanvasKit scaled that raster), so icons and illustrations are sharper.
- **Text shadow blur** (shadow/neon) scales with the canvas transform, so zoomed editor views and 2× exports look
  like the 1× page; CanvasKit's SkParagraph used a device-pixel σ (`respectCTM = false`). Identical at scale 1.
- **Odd dash arrays** draw a solid stroke (no valid dash, as Skia's `MakeDash` returns null); CanvasKit's JS
  binding threw for them and the whole page failed.
- **QR data too long** for a QR code resolves to a placeholder op; the TS resolver threw for the whole page.

## PDF writer vs pdfVector.ts

Both PDFs rasterised by hayro (a pure-Rust PDF renderer) and compared with each other and with the CanvasKit
raster. Where pdfVector.ts is right, the two PDFs rasterise almost identically (ΔE ≤ 0.03 on 18 of 25 pages:
same fonts, same glyph positions to the sub-pixel, same geometry). Where they differ, the Rust PDF is the one that
matches the painter:

| page | ours ↔ TS PDF | ours ↔ raster | TS PDF ↔ raster | size TS → ours |
|---|---|---|---|---|
| tpl-sa-igp-editorial-cover-p1 | 26.2 | 0.61 | 26.2 | 6.8 MB → 1.4 MB |
| tpl-sb-pin-travel-guide-p1 | 6.87 | 1.22 | 6.87 | 5.8 MB → 0.9 MB |
| scene-media-c0-p1 | 2.42 | 0.47 | 2.66 | 388 KB → 36 KB |
| scene-shapes-c0-p1 | 1.92 | 0.45 | 2.28 | 24 KB → 41 KB |
| tpl-vd-deck-company-p1 | 0.75 | 1.23 | 0.99 | 19.5 MB → 3.4 MB |

pdfVector.ts paints gradients as plain PDF shadings, which carry no alpha, so translucent gradient overlays (the
usual darkening over a cover photo) come out opaque; it also strokes with a gradient's first colour and draws
tiled image fills once. The Rust writer keeps per-stop opacity (soft masks), gradient strokes and tiling.
pdfVector.ts also failed outright on 7 of the 32 sampled pages (fontkit buffer errors, CanvasKit aborts). Fonts
are embedded as subsets, so the files are 5–50× smaller.
