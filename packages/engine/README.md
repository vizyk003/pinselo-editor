# @oc/engine — the Rust core in the browser

`@oc/engine` is the browser bridge of ADR-0001 (docs/adr/0001-rust-core.md). It loads `crates/oc-wasm` (the Rust
editor, schema, geometry, CRDT, text, sheet and render crates compiled to WebAssembly with wasm-bindgen) behind the
names and signatures of the TypeScript core packages it replaced (`@oc/editor-core`, `@oc/geometry`, `@oc/schema`,
`@oc/sheet`, `@oc/crdt`, `@oc/text`, `@oc/render-*`, all deleted in plan 13 stage 4). `apps/web` imports only this
package and `@oc/config`; so do the template tooling (`tools/scripts/templates`) and the root tests.

The document and editor types live in `src/types/` (one folder per crate, see its README): they are the contract
between the TypeScript shell and the crates' serde types, hand-maintained next to the Rust. When a crate gains a field,
an element type or a wasm export, extend the declaration there in the same change; the facade types every export as
`typeof S.fn` against them, so `pnpm --filter @oc/engine build` fails on a drift.

```ts
import { Editor, EngineTextService, SheetEngine, sheetRoot, sortedPages, worldBounds, SIZE_PRESETS } from '@oc/engine';

const editor = new Editor({ ydoc, actor: user.id });   // ydoc: the provider's Y.Doc (source of truth)
editor.textService = new EngineTextService(manifest);    // fonts: textService.registerFont(bytes)
const sheet = new SheetEngine(sheetRoot(ydoc));          // formulas over a sheet design's Y.Doc
```

## Build and test

Prerequisites (once per machine):

- A cargo with the `wasm32-unknown-unknown` target. The build script uses the isolated rustup home in `~/.rustup`
  when it exists (`RUSTUP_HOME=~/.rustup CARGO_HOME=~/.rustup/cargo`), else `cargo` on `PATH`; override with
  `OC_WASM_CARGO=/path/to/cargo`.
- The wasm-bindgen CLI with **exactly** the version of the `wasm-bindgen` crate in `Cargo.lock` (0.2.129 today).
  The build checks it and prints the install command, e.g.
  `RUSTUP_HOME=~/.rustup CARGO_HOME=~/.rustup/cargo ~/.rustup/cargo/bin/cargo install wasm-bindgen-cli --version 0.2.129 --locked`
  (never into `~/.cargo/bin`, where Homebrew cargo looks up subcommands). Override with `OC_WASM_BINDGEN`.
- `pnpm install` (brings `wasm-opt` via the `binaryen` dev dependency; nothing is installed globally).

| Command | What it does |
|---|---|
| `pnpm --filter @oc/engine build` | release wasm (size profile) → `wasm-bindgen --target web` → `wasm-opt -Oz` → `pkg/`, then `tsc` checks the facade against the generated `.d.ts` |
| `… build -- --dev` | unoptimised wasm, ~20 s incremental (all tests pass on it, benchmarks are meaningless) |
| `… build -- --lean` | without the complex-script line-break dictionaries (see sizes) |
| `… build -- --no-sheet` | without the oc-sheet module (formula engine, XLSX) |
| `… build -- --opt=s` | one rustc opt-level for everything (default: `z`, the renderer crates `3`) |
| `… build -- --no-render` / `--no-hot` / `--no-simd` | without oc-render / renderer crates at `z` too / without wasm SIMD |
| `… build -- --no-opt` / `--no-check` | skip wasm-opt / the type check |
| `pnpm --filter @oc/engine test` | vitest: editor, helper and sheet goldens, sync, text, benchmark (needs a build) |
| `pnpm --filter @oc/engine bench` | the benchmark only |
| `pnpm --filter @oc/engine bench:render [template.json …]` | the editor paint benchmark (plain Node; vitest's workers run wasm several times slower; `OC_ENGINE_WASM` loads another build) |
| `cargo test -p oc-wasm` | native tests of the bridge core (`tests/session.rs`, `tests/sheet.rs`) |
| `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=~/.rustup/cargo/bin/wasm-bindgen-test-runner RUSTUP_HOME=~/.rustup CARGO_HOME=~/.rustup/cargo ~/.rustup/cargo/bin/cargo test -p oc-wasm --target wasm32-unknown-unknown --test web` | wasm-bindgen tests in Node (`tests/web.rs`) |

All cargo commands should run with `CARGO_TARGET_DIR=<repo>/target/engine-wasm` (the build script sets it).

The release profile (opt-level `"z"`, fat LTO, one codegen unit, `panic = "abort"`, no debug info) is applied by the
build script through `CARGO_PROFILE_RELEASE_*` variables: profiles can only be declared in the workspace root, and
the native services keep their own release settings.

**License.** The default build links oc-sheet (AGPL-3.0-only, like `@oc/sheet` and `@oc/editor-core`, which this
package replaces), so the package is AGPL-3.0-only; a `--no-sheet` build links only this repo's Apache-2.0 crates
(and their permissively licensed dependencies).

**Generated files** go to `packages/engine/pkg/` (gitignored). They are not committed: the wasm is ~12 MB of
binary, fully reproducible from the Rust sources with the pinned toolchain, and would conflict on every core
change. The tests are not part of the root `pnpm test` / `pnpm typecheck` (CI runs them in the `verify` job after the
build); the package has no `typecheck` script for the same reason — `build` type-checks.

## Using it from Vite

`@oc/engine` instantiates the wasm with **top-level await**, so every export (constants included) is usable as soon
as the import resolves, exactly like the TS packages. That needs an ES2022 build target (apps/web already sets
`build.target: 'es2022'`). The `.wasm` is referenced as `new URL('../pkg/oc_wasm_bg.wasm', import.meta.url)`,
which Vite emits as an asset in dev and build; workspace packages are not pre-bundled, but if one ever is, add
`optimizeDeps: { exclude: ['@oc/engine'] }`. In Node (vitest, scripts) the file is read with `node:fs` through
`process.getBuiltinModule`, so no Node import reaches browser bundles.

`@oc/engine/lazy` is the same API without the top-level await: call `await initEngine(input?)` first (input: bytes,
URL, `Response` or a compiled `WebAssembly.Module`; `globalThis.__OC_ENGINE_WASM__` can supply it too).

## Architecture

```
apps/web (React, DOM events, IME, TipTap, ARIA)            Rust (crates/oc-wasm → oc-editor-core, oc-schema, …)
  │  editor.pointerMove(p, mods) ───── numbers ─────────▶  Editor::pointer_move  ─┐
  │  ◀──────────────── status word (f64) ──────────────────────────────────────────┤ version*4 + overrides*2 + pending
  │  editor.select(ids) ──── JSON args ── dispatch ─────▶  Editor::dispatch(name, json)
  │  editor.state / doc / view ─── lazily, once per version: state if changed, doc delta, overrides
  │  sortedPages(doc) ─── DocRef (no serialisation) ────▶  oc_schema::tree::sorted_pages → ids → the doc's objects
  │
  Y.Doc (provider, TipTap, board)  ◀── local updates (lib0 v1, engine origin, transaction marked local) ── yrs mirror
                                   ─── every other update ─────────────────────────────────────────────▶ apply_remote_update
```

- **Sync.** The browser's yjs `Y.Doc` stays the network/TipTap source of truth. `new Editor(store | ydoc)` starts a
  yrs mirror from `Y.encodeStateAsUpdate(ydoc)` (its own client id), applies every yjs update not made by itself
  (`applyRemoteUpdate`), and after each call applies its own updates to the yjs doc in one transaction whose origin
  is `editor.origin`. That transaction is marked local: `Y.applyUpdate` marks it remote, and yjs would then run its
  remote-only YText formatting cleanup on the user's own edits and write the result back (found by the text golden;
  TS edits were local). Undo lives in the mirror and tracks only the editor's own changes.
- **Boundary.** Hot paths (pointer down/move/up, hover, pan/zoom, viewport) take plain numbers; modifier keys are a
  bit set. Every other member goes through `Editor::dispatch(name, argsJson)` — the facade contract the Rust
  goldens verify: JSON args (`JSON.stringify` semantics: trailing `undefined`s trimmed, `undefined` patch values sent
  as `null` = remove, as DocOps treat them), results as JS objects via serde-wasm-bindgen. A status word comes
  back from every call: the facade notifies listeners when the version moved, forwards pending updates, and knows
  whether the view differs from the document.
- **Snapshots.** `state` is fetched only when it changed (`stateIfChanged`, so document-only changes keep its
  identity like TS). `doc` is a JS mirror patched from Rust deltas computed by comparing snapshot `Arc`s:
  untouched elements keep their object identity (structural sharing, like the TS DocStore). `view` overlays the
  gesture overrides in JS exactly like the TS getter, memoised per (doc, overrides).
- **Helpers on documents.** Functions taking a design receive a `DocRef`, cached per JS object (`WeakMap`):
  documents handed out by an editor resolve to the editor's own parsed snapshot (an `Arc` clone), a `Pick` of such a
  document (`{ elements: view.elements }`) to its owner, anything else is parsed once. Element/page results come
  back as ids and are mapped to the caller's objects, so `childrenOf`, `hitTest`, `pageSlots`, `locateTime`… return
  the same objects as TS.
- **Listeners** live in the facade, not in Rust, so a listener may call back into the editor (a JS callback from
  inside a `&mut self` Rust call would hit wasm-bindgen's aliasing guard).
- **Injected services.** The clock defaults to `performance.timeOrigin + performance.now()` (epoch-based: DocOp
  tombstones need wall time; `'manual'` + `setNow` for tests); ids come from Web Crypto, or Mulberry32 with
  `seed`. Text measurement: an `EngineTextService` (oc-text, called from Rust directly, fonts registered by SHA-256
  from the manifest) or any object implementing the TS `TextService` interface (called back per measurement; it must
  not call into the editor).

### Sheets

`SheetEngine` follows the same model as the editor, with one yrs mirror per yjs doc (created on first use, freed
with the doc). The grid keeps writing some `Y.Map`s directly (`tab.colW.set(…)`, `tab.map.set('frozenRows', …)`)
and `Y.UndoManager` keeps tracking `SHEET_LOCAL` on the yjs doc; every yjs update not made by the mirror is applied
to it, and its live formula engine (oc-sheet's HyperFormula-compatible workbook) recalculates. The model writes
(`setRaw`, `patchFormat`, `setMerge`, `writeRows`, `addTab`, `initSheet`, `insertTracks`/`deleteTracks`,
`deleteTab`) and the engine's structural edits (`insert`/`remove` rows and columns, `renameTab`, `deleteTab`, with
formula rewrites) run on the mirror, and their update is applied to the yjs doc at once: inside the caller's
`ydoc.transact(…, SHEET_LOCAL)`, so undo records it under that origin as before, or in a `SHEET_LOCAL` transaction
of its own for the engine's structural edits (one undo step with its formula rewrites, like TS). `TabSnap`s keep the
TS shape (row/column ids plus the tab's live yjs maps), re-snapped only when the Rust side reports a structure change.
The accessors that only read those maps (`sheetRoot`, `snapTab`, `keyAt`, `rawAt`, `fmtAt`, `colWidth`, `rowHeight`)
are yjs plumbing and stay in TS; everything that computes runs in Rust. oc-sheet is AGPL-3.0-only like `@oc/sheet`,
so a build with the (default) `sheet` feature is too.

### Renderer (oc-render)

`crates/oc-wasm/src/render.rs` (feature `render`, on by default) and `src/render.ts` replace `@oc/render-core`,
`@oc/render-canvaskit` and CanvasKit; `src/text.ts` replaces `@oc/text`.

```ts
const fonts = new FontRegistry(manifest, (file) => fetch(`/api/fonts/files/${file}`).then((r) => r.arrayBuffer()));
const painter = new Painter(new TextEngine(fonts), (asset) => fetch(assetUrl(asset)).then((r) => r.arrayBuffer()));
editor.textService = fonts.service;                 // measuring, caret geometry and drawing share one oc-text engine
const t = new RenderTarget().begin(0, 0, w, h);     // a device rectangle (a tile, a thumbnail)
t.clear(1, 1, 1);
t.scale(k);
painter.draw(t, resolvePage(doc, pageId));          // DisplayList held in Rust; ops cross only when read
ctx.putImageData(t.end(), 0, 0);
```

- **Display lists** stay in Rust (`DisplayList.raw`); `items`/`background`/`assets` serialise on first read.
  `painter.dirtyRegion(prev, next)` compares two lists of a page by element group (oc-render `bounds`: per-element
  op runs with conservative bounds — path and stroke reach, markers, text laid out with overflow and effects, layer
  shadows and blurs) and returns the changed page area, `'full'` (background, size or stacking order changed) or
  `null`.
- **Painting** is tiny-skia into a region canvas (`Canvas::new_region`): a `RenderTarget` covers a rectangle of a
  larger device and keeps the device's coordinates, so every tile of a scene rasterises exactly like the whole
  scene would and a repainted tile is bit-identical to the full repaint (tested over every seed template). The
  editor paints its scene in scene-anchored 256 px tiles (`apps/web/src/editor/scene.ts`): pans shift pixels and
  paint the exposed strips, edits repaint the dirty tiles, zooms show the scaled last frame and refill tiles within
  a 10 ms per-frame budget. Tiles are painted with a margin of the widest filter reach (`filterReach`). The scene
  primitives are on the target: `clear`, `fillRect`, `shadowRect` (an analytic Gaussian, skipping the area under an
  opaque page), `dots` (whiteboard grid), `clipRect`/`clipCircle`, `saveLayer(alpha, blur)`.
- **Device pixel ratio** is the caller's scale (`k = zoom × dpr`); the editor keeps scroll offsets in whole device
  pixels.
- **Assets** load on demand: a draw that needs bytes it lacks queues the key; the painter fetches each once
  (`AssetSource`), hands the bytes over (`addAsset`, which forgets the placeholder decode) and fires
  `onInvalidate`. Failed keys keep their placeholder.
- **Video**: `painter.videoFrames(asset, sourceTimeMs, key)` returns a `<video>` (or `ImageData`); its frame is sampled
  once per animation frame (at most 1280 px) into the Rust frame store (`VideoFrames`, shared `Arc` pixmaps).
- **Transitions**: `painter.drawTransitionFrame` draws oc-render's `draw_transition`; `drawTransition(c, w, h, f,
  drawPage)` is the same layering over any `TransitionCanvas` (a `RenderTarget`, or `canvas2dTransitionTarget(ctx)`
  for GPU compositing of pre-painted page bitmaps, which Present mode uses). The two are pixel-identical on a
  `RenderTarget` (tested for every kind).
- **Fonts** (`FontRegistry`, `TextEngine`): oc-text records the faces a layout (measuring or drawing) needed and did
  not have (`report_missing_faces` / `takeMissingFaces`, each file once). `FontRegistry.pump()` fetches them through
  the loader (bundled, Google and uploaded faces alike, by manifest path), registers them and calls `onLoad`
  listeners: the painter repaints, the editor re-measures. The painter and `TextEngine.layout` pump after each call;
  hosts that only measure pump on editor changes. `idle()` resolves when nothing is loading.
- **Text**: `TextEngine.layout(key, body, width, effect)` is the shared engine's cached layout (the one drawn), with
  `caretRect`, `selectionRects`, `offsetAt`, `wordAt`, `verticalMove`.

### Adding modules

Each Rust module is a file in `crates/oc-wasm/src/` with `#[wasm_bindgen]` exports behind a cargo feature (`sheet`
and `render` are the examples), plus a TS file in `src/` re-exported from `api.ts`. The build script maps its flags
to cargo features (`--lean`, `--no-sheet`, `--no-render`).

## Sizes and speed (release build, Apple M-series, Node 22)

| Build (`oc_wasm_bg.wasm` after wasm-opt) | raw | gzip -9 | brotli -11 |
|---|---|---|---|
| default (`complex-scripts`, `sheet`, `render`; renderer crates at opt-level 3, SIMD) | 11.99 MB | 5.59 MB | 4.29 MB |
| before the renderer (`complex-scripts`, `sheet`; all at opt-level `z`) | 9.05 MB | 4.49 MB | 3.53 MB |
| `--lean` (no complex-script dictionaries) saves about | 3.6 MB | 2.3 MB | 2.0 MB |

(MiB.) The renderer adds ~3 MB raw / 0.8 MB brotli (tiny-skia, resvg/usvg, the image decoders, oc-render); the vector
PDF writer (krilla, with its print colour management and PDF/X checker) is behind oc-render's `pdf` feature and left out. Compiling the painter's hot crates (tiny-skia,
oc-render, oc-text, image decoders, skrifa, harfrust, parley) at opt-level 3 instead of `z` makes painting ~2× faster
for +0.1 MB brotli (all at 3: +0.9 MB).

**apps/web download, editor (wasm + JS, production build):** before (Rust core + CanvasKit + the TS renderer and text
packages): 20.1 MB raw, 8.74 MB gzip, 6.96 MB brotli (`canvaskit.wasm` alone 7.32 MB / 2.96 / 2.28); after: 15.65 MB
raw, 6.91 MB gzip, 5.42 MB brotli — about 22 % less, one wasm instead of two.

**Painting** (`pnpm --filter @oc/engine bench:render`: apps/web's scene painter on 1440 × 900 CSS px at DPR 2,
2880 × 1800 device px, the page fitted at 90 %; medians on a machine shared with other builds, so p95s are noisy):

| template | full repaint (all tiles, spread over frames) | drag frame | typing frame |
|---|---|---|---|
| vd-deck-lesson (71 elements, 13 photos) | 111 ms | 2.7–3.7 ms | 13.7 ms |
| mw-web-services-greenleaf (104 elements, blur + shadows) | 503–526 ms | 8.7–10.6 ms | 17–25 ms |
| pr-flyer-open-house (22 elements, 1142 × 1620 px page) | 101–117 ms | 1.7–2.7 ms | 5.7–12.6 ms |

A drag frame is `pointerMove` + `resolvePage` of the view + `dirtyRegion` + the dirty tiles from cached layers (the
dragged groups as a sprite); a typing frame adds `editor.apply` and redraws only the changed lines. Full repaints
(zoom, resize) refill within a 10 ms per-frame budget while the scaled previous frame is shown. In Chrome (headless,
same viewport) a drag frame painted in 1.3 ms median; the Rust editor's own `pointerMove` is 4–5 ms on these pages
(the O(page) override clone below), so it dominates a drag frame now. Thumbnails (160 px wide) take 1–5 ms.

Pixel parity with CanvasKit is crates/oc-render/PARITY.md (re-run against fresh references after the tiled painter:
363 pages, worst mean ΔE 0.77, unchanged). In the browser, the oc-render editor and the CanvasKit editor on the same
designs and viewport differ by 0.1–0.5 levels on average (≤ 0.07 % of pixels by more than 40 levels: anti-aliasing and
photo resampling).

### Core benchmark (editor, sheet)

Benchmark (`pnpm --filter @oc/engine bench`; best of three round-robin runs; a 23-element template and the same
with 300 extra shapes, one shape dragged). Ranges are two runs of the final build under different machine load
(other builds running / idle):

| | 23 elements | 323 elements |
|---|---|---|
| `pointerMove` (typed hot path) | 8,300–14,400/s (121–70 µs) | 1,600–1,930/s (616–517 µs) |
| frame: `pointerMove` + `state` + `view` | 8,000–11,800/s | 1,500–1,820/s (668–551 µs) |
| `pointerMove` via JSON `dispatch` | 10,000–13,100/s | 1,570–1,880/s |
| boundary only: status word / trivial JSON dispatch / typed `setHover` | 0.03 µs / 0.3–0.4 µs / 0.3–0.5 µs | same |
| full document → JS (serde-wasm-bindgen / JSON + parse), first read only | | 2.4–2.9 ms / 1.7–2.2 ms |
| state → JS (serde-wasm-bindgen / JSON + parse) | | 4.1–4.8 µs / 11–13 µs |
| sheet: 1,000-cell viewport (`value`+`kind`+`formatValue` per cell / `displayRows`) | 4.5–6.4 ms / 3.6–6.1 ms | |
| sheet: cell edit (`setRaw` + recalc + yjs update) | 2,180–3,100/s (458–323 µs) | |

The boundary costs well under a microsecond per call, so a drag frame is the Rust editor's own work: 60 fps holds
with a wide margin on these pages (1,500–1,800 frames/s at 323 elements), but `pointer_move` is O(page): every move
rebuilds the overridden view by deep-cloning the typed `DesignDoc` (`with_overrides` in oc-editor-core/src/doc.rs)
before snapping against sorted siblings (≈1.5 µs per element in wasm; natively 6 ms at 323 elements with the macOS
allocator). That is the thing to fix for big whiteboards (Arc'ed elements in `DocView`). The typed hot path is not
measurably faster than JSON `dispatch` for a single move; it saves the per-move allocations. Sheet grids are bound
by oc-sheet's number formatting: `generalNumber`/`formatValue` cost ~9 µs per number because
`jsnum::exact_digits` formats 1,100 decimal digits to emulate `toPrecision` exactly, so a 1,000-cell viewport spends
~4–6 ms per full render (fine at idle, tight while scrolling); a shortest-digits fast path would fix it.

## API mapping

Every facade function is typed as its TS original (`export const f: typeof S.f = …`; sheet helpers that take an
engine take this package's `SheetEngine` instead), the `Editor` class `implements Pick<TSEditor, keyof TSEditor>`
minus `store`/`overrides`, and `SheetEngine` the TS engine's members minus `hf` (HyperFormula itself): a signature
drift fails `build`.

| TS package export (used by apps/web) | Facade (`@oc/engine`) | Rust |
|---|---|---|
| `@oc/editor-core` `Editor` (all 169 public members, incl. `state`, `doc`, `view`, `version`, `subscribe`, `brandAdmin`, `user`, `textService`, `CLIPBOARD_MIME`) | `Editor` (`src/editor.ts`): hot paths typed, the rest via `dispatch` | `oc_wasm::Editor` → `oc_editor_core::Editor` (`dispatch.rs`, `pointer_*`, `set_viewport…`) |
| `editor.store.{ydoc, actor, doc, checkpoint, apply, undo.undoStack/redoStack.length}` | `EngineStore` (+ `replaceContent`) | `DocStore` of the mirror, `store_apply_with`, `can_undo/can_redo`, `oc_crdt::replace_content` |
| `STICKY_COLORS`, `formatClock`, `isMindNode`, `isSection`, `isSticky`, `makeTimer`, `mindNodeOf`, `sectionsOf`, `tallyVotes`, `timerAdd/Pause/Remaining/Reset/Running/Start`, `topVoted`, `voteAllowance`, `voteScope` | `src/editor-fns.ts` | `oc_editor_core::whiteboard` |
| `auditDesign`, `describeElement` | `src/editor-fns.ts` | `oc_editor_core::a11y` |
| `designColors`, `fontSetOps` | `src/editor-fns.ts` | `oc_editor_core::styles` |
| `pageSlots`, `screenToScene` | `src/editor-fns.ts` | `oc_editor_core::viewport` |
| `templatePages` | `src/editor-fns.ts` (+ `editor.templatePages` on the editor's id stream) | `oc_editor_core::templates` |
| types `A11yIssue`, `ClipboardPayload`, `EditorState`, `ElementKindLabel`, `FindMatch`, `Handle`, `HitTarget`, `Mods`, `TextService`, `Viewport` (+ the other 9 type exports) | re-exported (`src/types.ts`) | serde types in `oc_editor_core::state` |
| `@oc/geometry` `worldMatrix`, `worldBounds`, `worldQuad`, `hitTest`, `anchorPoint`, `ANCHORS` | `src/geometry.ts` (documents via `DocRef`) | `oc_geometry::scene`, `connectors` |
| `apply`, `multiply` | `src/geometry.ts` (`Float64Array`) | `oc_geometry::matrix` |
| `shapePath`, `scaleUnitPath`, `lineMarkerShape`, `trimLineForMarkers` | `src/geometry.ts` | `oc_geometry::shapes`, `markers` |
| type `AlignEdge` (+ `Guide`, `MarkerShape`, `SnapResult`, `SpacingMark`) | re-exported | `oc_geometry` |
| `@oc/schema` constants: `ANIM_PRESETS`, `BORDER_PRESETS`, `CALLOUT_TONES`, `CHART_COLORS`, `CHART_KINDS`, `CURRENT_COLOR`, `DOC_CONTENT_WIDTH`, `EMBED_FRAME_HOSTS`, `EMPHASIS_PRESETS`, `FILTER_PRESETS`, `FRAME_CATEGORIES`, `FRAME_LIBRARY`, `GRID_LAYOUTS`, `KEYFRAME_NEUTRAL`, `KEYFRAME_PROPS`, `LINE_MARKERS`, `POPULAR_LAYOUTS`, `PRESET_CATEGORIES`, `SHAPE_KINDS`, `SIZE_PRESETS`, `TABLE_STYLE_PRESETS`, `TRANSITION_KINDS` (+ `DEFAULT_PRINT_CATALOG`) | `src/schema.ts`, deep-frozen, read once at init | `oc_schema::{elements, table, doc_blocks, factory, svg, embeds, frames, keyframes, presets, design, print}` |
| `bodyPlainText`, `textBody`, `hex`, `solid`, `toHex`, `toCss`, `newId` | `src/schema.ts` | `oc_schema::{text, factory, color, ids}` |
| `makeText/Shape/Line/Image/Frame/Grid/Qr/Table/Chart/Video/Form/Embed`, `makeStroke`, `emptyDesign`, `placeAll`, `clipLengthMs`, `formLayout` | `src/schema.ts` (`placeAll` spreads Rust z keys over the caller's objects) | `oc_schema::factory` |
| `sortedPages`, `childrenOf`, `descendants`, `pageOf` | `src/schema.ts` (ids → the doc's objects) | `oc_schema::tree` |
| `pageDurationMs`, `timelineOf`, `locateTime`, `elementWindow`, `mediaSourceTime`, `videoSlots` | `src/schema.ts` | `oc_schema::timeline` |
| `tableGeometry`, `cellRect`, `canUnmerge`, `tableStyleSwatch` | `src/schema.ts` | `oc_schema::table` |
| `findPreset`, `presetsInCategory`, `presetPlatforms`, `popularPresets`, `tryNewPresets`, `pushRecentPreset`, `searchPresets`, `lockedSide`, `pxToUnit`, `unitToPx`, `formatDims` | `src/schema.ts` | `oc_schema::presets` |
| `expectedPages`, `preflightTargetFor`, `standalonePrintTarget`, `preflightPrint` (with `outOfGamut`), `printColors`, `printColorKey`, `quotePrint`, `validateAddress` | `src/schema.ts` | `oc_schema::print` |
| `calloutTone`, `docBlockWidth`, `parseDocChart`, `serializeDocChart`, `docChartElement`, `parseEmbedUrl`, `extractSvgColors`, `keyframeValue` | `src/schema.ts` | `oc_schema::{doc_blocks, embeds, svg, keyframes}` |
| all 131 `@oc/schema` type exports | declared in `src/types/` (listed in `src/types/index.ts`) | serde types in `oc_schema` |
| `@oc/sheet` `SheetEngine` (`root`, `tabs`, `version`, `ydoc`, `subscribe`, `destroy`, `rebuild`, `tab`, `value`, `kind`, `rangeValues`, `insert`, `remove`, `renameTab`, `deleteTab`) | `SheetEngine` (`src/sheet.ts`, + `formula`) | `oc_wasm::sheet::SheetEngine` → `oc_sheet::engine::SheetEngine` on a yrs mirror |
| `initSheet`, `hasSheet`, `addTab`, `deleteTab`, `setRaw`, `patchFormat`, `setMerge`, `writeRows`, `insertTracks`, `deleteTracks`, `uniqueTabName`, `usedRange`, `mergeRanges`, `rawRows`, `splitKey` | `src/sheet.ts` (writes go through the mirror into the caller's yjs transaction) | `oc_sheet::{model, csv}` |
| `sheetRoot`, `snapTab`, `keyAt`, `rawAt`, `fmtAt`, `colWidth`, `rowHeight`, `SHEET_LOCAL` | `src/sheet.ts` (yjs reads; `SHEET_LOCAL` is this package's symbol) | — |
| `displayRows`, `tabToCsv`, `rangeToTable`, `sheetToDesign`, `readRange`, `openSheetState`; `@oc/sheet/xlsx` `sheetToXlsx`, `readXlsx` | `src/sheet.ts` | `oc_sheet::{csv, table, linked, xlsx}` |
| `a1`, `colName`, `colIndex`, `parseA1`, `normRange`, `parseRange`, `formatRange`, `quoteSheet`, `shiftFormula`, `formatValue`, `autoAlign`, `generalNumber`, `parseDelimited`, `quoteField`, `toDelimited`; `@oc/sheet/chart` `rangeToChartData`, `chartPatch`, `chartChanged` | `src/sheet.ts` | `oc_sheet::{a1, format, csv, chart}` |
| `DEFAULT_ROW_H` (+ `SHEET_ROOT`, `DEFAULT_ROWS/COLS/COL_W`, `MAX_ROWS/COLS`); types `CellFormat`, `CellValue`, `NumberFormat`, `Range`, `TabSnap` (+ 6 more) | `src/sheet.ts`, `src/types.ts` | `oc_sheet::model` |
| (`@oc/crdt` `toY`/`fromY` for seeding a doc) | `encodeDesign`, `decodeDesign` | `oc_crdt::{encode_doc_value, decode_doc_value}` |
| `@oc/render-core` `resolvePage`, `DisplayList`, `ResolveOptions`, `DLItem`, `smoothPath`, `sliceColor`, `pageSettleMs`, `clickStepCount`, `transitionProgress`, `transitionFrame`, `TransitionFrame` (`tableGeometry` is the schema one) | `src/render.ts` (`DisplayList` held in Rust) | `oc_render::{resolve, builders, motion}` |
| `@oc/render-canvaskit` `Painter` (`draw`, `onInvalidate`, `videoFrames`, `imageSize`, `preload`), `drawTransition` | `Painter`, `RenderTarget` (replaces the CanvasKit canvas/surface), `drawTransition` over `RenderTarget` or `canvas2dTransitionTarget` | `oc_render::{Renderer, raster::Canvas, raster::transition, bounds}` |
| `@oc/text` `FontRegistry`, `TextEngine` (`layout`, `caretRect`, `selectionRects`, `offsetAt`, `wordAt`, `verticalMove`, `requestFonts`), `FontFamily`, `FontManifest` | `src/text.ts` (one oc-text engine shared with `EngineTextService` and the painter) | `oc_text::{FontRegistry, TextEngine, TextLayout}` |
| `@oc/crdt` board helpers (`castVote`, `clearVotes`, `observeBoard`, `readTimer`, `readVotes`, `readVoteSession`, `writeTimer`, `writeVoteSession`), `DocOp`; `brandLockError` | yjs plumbing in `src/board.ts`; `brandLockError` in Rust (`src/editor-fns.ts`) | — ; `oc_crdt::brandlock` |

Coverage, checked by script against the current apps/web tree: every one of the 161 value and 83 type imports from
`@oc/editor-core`, `@oc/geometry`, `@oc/schema`, `@oc/sheet`, `@oc/sheet/chart` (and the dynamic `@oc/sheet/xlsx`)
across 59 files is exported by the facade. Exports apps/web does not use (e.g. `resizeBox`, `zoomAt`, `validateDoc`,
`migrate`, `recolorSvg`, `connectorGeometry`, `encodeSheetDesign`) are not in the facade yet; each is a few lines when
needed.

### Parity evidence (`pnpm --filter @oc/engine test`)

- **Editor goldens**: all 27 sessions of `tests/golden/editor/` (2,082 steps, recorded from the TS editor) replayed
  through the facade's typed methods, the yjs update handler (`$remote`) and a JS text service; after each step the
  return value, state, document, overrides, undo/redo, version and random draws match, listeners ran iff the version
  moved, and at the end the yjs doc (fed only by the facade) decodes to the editor's document.
- **Helper goldens**: 11,303 cases of `tests/golden/{editor/modules,schema,geometry}` plus 25,318 scene checks
  (world matrices/quads/bounds, grid hit tests with tolerance and entered groups, anchors) on the hand and
  fixture-derived scenes; all 23 constants equal `constants.json`.
- **Sync**: yjs ⇄ wasm in both directions (commands, drags, typing, undo, direct yjs writes, remote deletes
  sanitising the selection), two browsers linked like a provider converging after sequential and concurrent edits,
  a late joiner, `replaceContent` as one undo step.
- **Text**: `tests/golden/text/hand.json` through `EngineTextService` with fonts registered by hash: 43/43 heights
  exact, `wordAt` exact, `offsetAt` 1/880 and `verticalMove` 10/1242 probes differ (oc-text's own acceptance
  allows 5%); the editor measures auto-height text through it.
- **Sheets** (`tests/golden/sheet/`): 1,553 a1/format/csv/chart cases; the model write sequence (`misc.model`) on a
  yjs doc; range → table and sheet → design layout; `readRange` over the state the TS package stored; all six recalc
  sequences (57 steps: cells, formats, row/column insert/delete with formula rewrites, tab add/rename/delete/reorder,
  merges, sizes, frozen panes, `writeRows`) applied exactly like the generator's `applyStep` — facade writers inside
  `ydoc.transact(…, SHEET_LOCAL)` plus direct yjs writes — with the tab state and every value, kind, display and
  formula text compared after each step. Plus two browsers converging, `Y.UndoManager` undo/redo of facade writes,
  an engine insert and its formula rewrite undone as one step, and an XLSX round trip.
- **Renderer** (`test/render.test.ts`): all 306 settled template pages, the animated cases and the render helpers
  against `tests/golden/render` (TS); tiles equal the page painted at once; dirty tiles repaint it exactly; images load
  once and repaint; host video frames replace the poster; `drawTransition` equals oc-render's for every kind; the font
  loader (missing faces fetched once, registered, text re-flows exactly as with the face preloaded, uploaded families,
  the editor's auto-height re-measure); the caret sits on the drawn glyphs. `test/drop-helpers.test.ts`: the drop
  target helpers against the TS ones. oc-render's own `tests/bounds.rs`: partial repaints bit-identical over 1,000+
  edits, layered painting, moves recognised as translations, typing dirties only the changed lines.
- **Rust**: `cargo test -p oc-wasm` (12 native tests: status word, update forwarding, deltas, changed-only snapshots,
  mirror convergence, seeded ids, tolerant parsing, fonts by hash, sheet mirrors) and 7 wasm-bindgen tests in Node
  (JS value shapes, injected JS clock, JS text service, errors as JS exceptions, `DocRef` from JS, sheet clock).

### Deviations and gaps

- **Notifications are coalesced per call** (listeners run once when a call moved the version; TS ran them per
  internal emit). `version` itself is exact.
- **Construction**: `new Editor(store | ydoc, viewport?, opts?)` takes anything with a `ydoc` (a TS `DocStore`
  works) or a `Y.Doc`; `editor.store` is an `EngineStore`, not a `DocStore` (`undoStack.length` is 0 or 1).
  `store.apply(ops, origin)` takes origin names (`'migration'`…); `@oc/crdt` singletons are matched by class name,
  which minification may break — pass names.
- **Writes made with `@oc/crdt` directly on the yjs doc** (e.g. `replaceContent(store.ydoc, …, LocalOrigin.I)`)
  reach the mirror as remote changes, so they are not undoable: use `editor.store.replaceContent(doc)`.
- **Clock**: one epoch-based clock (TS mixed `Date.now()` and `performance.now()`), so `state.preview.startedAt`
  and `playAnchor.at` are epoch ms: compare with `editor.now()`, not `performance.now()`.
- `paste()` / `paste(editor.copy()-result)` use the internal clipboard like TS; any other object is pasted as an
  external payload. `cellTextEl(id, r, c, doc)` ignores `doc` (always the current view). `overrides` is a snapshot.
- Document key order is the mirror's (sorted ids) rather than yjs insertion order — the known `designColors` tie
  order deviation of oc-editor-core. Results returned as fresh objects (`selectionBox()`, `copy()`…) are not
  identical across calls, element/page results are.
- Inherited Rust deviations (documented in oc-editor-core / oc-schema / oc-sheet): untracked reactions to undo,
  `list: null`, `parseEmbedUrl` on malformed escapes, oc-sheet's XLSX/array-formula notes. `shapePath` of an unknown
  kind returns `undefined` like TS (Rust gives `""`).
- **Sheets**: writers address tabs by id and resolve positions against the mirror's current tab structure (TS: the
  snapshot passed in; the same unless the structure changed since). `writeRows`' `snap` argument is not needed.
  Engine listeners run when a write happens rather than at the end of the yjs transaction. `SheetEngine.hf` (the
  HyperFormula instance) does not exist; `formula(tab, r, c)` gives the normalised formula text. `readXlsx` returns
  formula cells as `{ formula, result }` like exceljs.

## Wiring apps/web (done on branch `engine-web`)

apps/web runs on `@oc/engine` only: `apps/web/scripts/engine.mjs` builds the engine before `dev`/`build` when `pkg/` is
missing or older than the Rust sources (`typecheck` only checks that it exists), and the CI verify job installs the
wasm32 target and wasm-bindgen CLI. CanvasKit, `@oc/render-core`, `@oc/render-canvaskit`, `@oc/text` and
`@oc/crdt` are no longer dependencies of the web app (the whiteboard board/vote helpers and the `DocOp` type are
re-exported by the facade).

The original plan, kept for reference:

1. Add `"@oc/engine": "workspace:*"` to apps/web and run `pnpm --filter @oc/engine build` before the web build and
   in CI (the web job then needs the wasm toolchain and wasm-bindgen CLI, or a cached `pkg/`).
2. Switch every import from `@oc/editor-core`, `@oc/geometry`, `@oc/schema`, `@oc/sheet`, `@oc/sheet/chart` and the
   dynamic `import('@oc/sheet/xlsx')` to `@oc/engine` (names are identical; type-only imports may switch too). The
   sites, by file (regenerated from the current tree):

| apps/web/src file | imports that switch |
|---|---|
| `api.ts` | schema: types AssetRef, DesignDoc, EmbedProvider |
| `components/ChartDataEditor.tsx` | schema: CHART_COLORS, hex, toHex; types ChartKind, ChartSeries, ColorValue |
| `components/EmbedOverlay.tsx` | geometry: worldMatrix — schema: EMBED_FRAME_HOSTS; types DesignDoc, EmbedEl |
| `components/FormOverlay.tsx` | geometry: worldMatrix — schema: formLayout, toCss; types DesignDoc, FormEl |
| `components/TemplatePreview.tsx` | schema: formatDims, sortedPages; types DesignDoc, PageId |
| `doc/DocEditor.tsx` | schema: CALLOUT_TONES; types CalloutTone |
| `doc/blocks.tsx` | schema: CALLOUT_TONES, CHART_KINDS, DOC_CONTENT_WIDTH, EMBED_FRAME_HOSTS, calloutTone, docBlockWidth, docChartElement, emptyDesign, parseDocChart, parseEmbedUrl, placeAll, serializeDocChart, sortedPages; types CalloutTone, ChartKind, DocChartData |
| `editor/A11y.tsx` | editor-core: auditDesign, describeElement; types A11yIssue, ElementKindLabel — schema: childrenOf, sortedPages; types DesignDoc, Element, ElementId, PageId |
| `editor/BrandLock.tsx` | schema: types Element |
| `editor/BrandPanel.tsx` | editor-core: designColors, fontSetOps; types Editor — schema: hex, solid; types TextEl |
| `editor/BulkCreate.tsx` | schema: bodyPlainText; types AssetId, Element |
| `editor/Canvas.tsx` | editor-core: pageSlots, screenToScene; types Handle, HitTarget, Mods — geometry: ANCHORS, anchorPoint, apply, hitTest, multiply, worldMatrix, worldQuad — schema: cellRect, childrenOf, locateTime, toCss; types DesignDoc, Element, ElementId, Mat2D, PageId, TableEl, TextEl, Vec2 |
| `editor/ColorPicker.tsx` | schema: hex, toCss, toHex; types ColorValue, DesignDoc, Paint |
| `editor/CommandPalette.tsx` | schema: makeShape, makeText, makeLine |
| `editor/Dialogs.tsx` | schema: formatDims, lockedSide, PRESET_CATEGORIES, presetsInCategory, searchPresets, SIZE_PRESETS, unitToPx, pxToUnit; types PresetCategoryId, Unit |
| `editor/DownloadPanel.tsx` | schema: sortedPages, videoSlots; types DesignDoc |
| `editor/EditorShell.tsx` | editor-core: Editor; types ClipboardPayload, TextService — schema: bodyPlainText, parseEmbedUrl, videoSlots; types TextEl |
| `editor/FindReplace.tsx` | editor-core: types FindMatch |
| `editor/FormTools.tsx` | schema: solid; types FormEl, FormField, FormFieldKind |
| `editor/Frames.tsx` | geometry: scaleUnitPath — schema: FRAME_CATEGORIES, FRAME_LIBRARY, makeFrame, toHex; types Element, FrameCategory, FrameEl, FrameLibraryItem, MediaClip |
| `editor/GraphicsPanel.tsx` | editor-core: types Editor — schema: types AssetRef, Vec2 |
| `editor/PageGrid.tsx` | schema: sortedPages; types PageId |
| `editor/PageStrip.tsx` | schema: sortedPages; types PageId |
| `editor/Panels.tsx` | editor-core: STICKY_COLORS, templatePages — geometry: lineMarkerShape, shapePath, trimLineForMarkers — schema: childrenOf, makeChart, parseEmbedUrl, makeLine, makeQr, makeShape, makeStroke, makeTable, makeText, SHAPE_KINDS, sortedPages, makeGrid, GRID_LAYOUTS, makeForm; types ChartKind, Element, ElementId, FrameClip, LineMarker, ShapeKind |
| `editor/Present.tsx` | schema: childrenOf, pageDurationMs, sortedPages, timelineOf, videoSlots; types EmbedEl |
| `editor/PublishSite.tsx` | schema: sortedPages |
| `editor/Rulers.tsx` | editor-core: types pageSlots |
| `editor/StylesPanel.tsx` | editor-core: designColors |
| `editor/SvgColors.tsx` | schema: CURRENT_COLOR, extractSvgColors, hex, solid, toHex; types ImageEl |
| `editor/TemplateDetails.tsx` | schema: formatDims |
| `editor/Timeline.tsx` | schema: clipLengthMs, elementWindow, KEYFRAME_NEUTRAL, KEYFRAME_PROPS, keyframeValue, pageDurationMs, timelineOf; types AudioClip, CaptionCue, Element, KeyframeProp, VideoEl |
| `editor/Toolbar.tsx` | geometry: types AlignEdge — schema: FILTER_PRESETS, GRID_LAYOUTS, hex, solid, toHex, LINE_MARKERS; types BlendMode, Effect, Element, GridEl, ImageEditStack, LineMarker, Paint, RunStyle, ShapeEl, Stroke, TextEffect, TextEl |
| `editor/ToolbarExtras.tsx` | schema: BORDER_PRESETS, canUnmerge, tableGeometry, TABLE_STYLE_PRESETS, tableStyleSwatch, clipLengthMs, CHART_COLORS, CHART_KINDS, hex, solid, toHex, ANIM_PRESETS, EMPHASIS_PRESETS, TRANSITION_KINDS; types BorderPreset, TableStylePreset, VideoEl, AnimPreset, AnimDirection, AnimUnit, EmphasisPreset, TransitionKind, ChartEl, ChartKind, DrawEl, Element, Page, TableEl |
| `editor/VideoPanel.tsx` | schema: newId, sortedPages, timelineOf; types AudioClip, CaptionCue, PageId |
| `editor/Whiteboard.tsx` | editor-core: formatClock, isMindNode, isSection, isSticky, makeTimer, mindNodeOf, sectionsOf, STICKY_COLORS, tallyVotes, timerAdd, timerPause, timerRemaining, timerReset, timerRunning, timerStart, topVoted, voteAllowance, voteScope; types Editor, Viewport — geometry: worldBounds — schema: childrenOf, newId, pageOf, sortedPages; types DesignDoc, ElementId |
| `editor/context.ts` | editor-core: types Editor, EditorState — schema: types DesignDoc |
| `editor/insert.ts` | editor-core: types Editor — geometry: hitTest — schema: makeEmbed, makeImage, makeText, parseEmbedUrl; types AssetRef, PageId, RunStyle, Vec2 |
| `editor/media.ts` | editor-core: types Editor — schema: locateTime, pageDurationMs, descendants, mediaSourceTime, videoSlots; types DesignDoc |
| `editor/placeIntent.ts` | editor-core: types Editor — schema: makeImage, makeVideo; types AssetRef |
| `lib/engine.ts` | schema: types AssetId |
| `lib/templates.ts` | schema: SIZE_PRESETS; types DesignDoc, Surface |
| `pages/EditorPage.tsx` | editor-core: Editor |
| `pages/Home.tsx` | schema: types PresetCategoryId |
| `pages/Presenter.tsx` | schema: sortedPages; types DesignDoc, PageId |
| `pages/PublicView.tsx` | schema: descendants, sortedPages; types DesignDoc, EmbedEl, FormEl, PageId |
| `pages/home/CreateDialog.tsx` | schema: findPreset, formatDims, lockedSide, POPULAR_LAYOUTS, popularPresets, PRESET_CATEGORIES, presetPlatforms, presetsInCategory, pushRecentPreset, pxToUnit, searchPresets, tryNewPresets, unitToPx; types PresetCategoryId, SizePreset, Unit |
| `pages/home/Dashboard.tsx` | schema: types PresetCategoryId |
| `pages/home/Designs.tsx` | schema: formatDims; types Unit |
| `pages/home/HomeSections.tsx` | schema: formatDims, SIZE_PRESETS |
| `pages/home/StartFromUpload.tsx` | schema: SIZE_PRESETS |
| `pages/home/quickTiles.tsx` | schema: formatDims, PRESET_CATEGORIES, SIZE_PRESETS; types PresetCategoryId, SizePreset |
| `pages/home/startMedia.ts` | schema: types AssetRef |
| `print/Address.tsx` | schema: validateAddress; types AddressField, AddressProblem, ShippingAddress |
| `print/Preflight.tsx` | schema: preflightPrint, printColorKey, printColors, sortedPages, standalonePrintTarget; types ColorValue, ElementId, PageId, PreflightFix, PreflightIssue, PreflightResult |
| `print/printExport.ts` | schema: DEFAULT_PRINT_CATALOG; types ColorValue, DesignDoc |
| `print/PrintDialog.tsx` | schema: expectedPages, preflightPrint, preflightTargetFor, quotePrint; types PreflightResult, PriceBreakdown, PrintCatalog, PrintProduct |
| `print/api.ts` | schema: types PreflightResult, PriceBreakdown, PrintCatalog, ShippingAddress |
| `sheet/SheetEditor.tsx` | editor-core: types ClipboardPayload — schema: CHART_KINDS, makeChart; types ChartKind — sheet: a1, addTab, autoAlign, colName, colWidth, DEFAULT_ROW_H, displayRows, fmtAt, formatRange, formatValue, hasSheet, initSheet, keyAt, mergeRanges, normRange, parseDelimited, parseRange, patchFormat, rangeToChartData, rangeToTable, rawAt, rawRows, rowHeight, setMerge, setRaw, SHEET_LOCAL, SheetEngine, sheetRoot, shiftFormula, snapTab, splitKey, tabToCsv, toDelimited, usedRange, writeRows; types CellFormat, NumberFormat, Range, TabSnap — sheet/xlsx: (dynamic import) |
| `sheet/linkedCharts.tsx` | editor-core: types ClipboardPayload, Editor — schema: types ChartEl, ChartSource, Element — sheet: types CellValue — sheet/chart: chartChanged, chartPatch, rangeToChartData |
| `social/EditorSchedule.tsx` | schema: bodyPlainText, sortedPages, videoSlots; types TextEl |

3. Beyond the import swap:
   - `pages/EditorPage.tsx`: `new DocStore(ydoc)` + `store.actor = …` + `new Editor(store)` → `new Editor({ ydoc, actor: user.id })`;
     `store.doc.meta.title` → `editor.doc.meta.title`.
   - `editor/EditorShell.tsx`: build an `EngineTextService` from the font manifest, feed it the font files the
     current registry loads (`registerFont(bytes)`), `editor.textService = ts`, `ts.onLoad(() => editor.remeasureText())`.
     Until oc-render replaces CanvasKit the existing JS `TextService` object also works unchanged (called back).
   - `editor/Drawers.tsx`: `replaceContent(editor.store.ydoc, doc, LocalOrigin.I, editor.store.actor)` between two
     checkpoints → `editor.store.replaceContent(doc)` (keep the checkpoints).
   - `sheet/linkedCharts.tsx`: `editor.store.apply(op, MigrationOrigin.I)` → `editor.store.apply(op, 'migration')`.
   - `editor/Canvas.tsx` (animation preview): `performance.now() - pv.startedAt` → `editor.now() - pv.startedAt`.
   - `editor/Whiteboard.tsx`, `DocEditor`: board/votes and docs keep using `editor.store.ydoc` (unchanged).
   - `sheet/SheetEditor.tsx`: nothing beyond the imports — `new SheetEngine(root)`, `new Y.UndoManager(root, {
     trackedOrigins: new Set([SHEET_LOCAL]) })`, `ydoc.transact(fn, SHEET_LOCAL)`, the direct `tab.colW.set` /
     `tab.map.set('frozen…')` writes and `engine.*` calls keep working; `SHEET_LOCAL` must come from `@oc/engine` (it
     is that package's symbol), and `await import('@oc/sheet/xlsx')` becomes `sheetToXlsx` from `@oc/engine`. For
     grid rendering, `displayRows(engine, tab, visibleRange)` reads a viewport in one call (same cost today, see the
     benchmark).
   - `sheet/linkedCharts.tsx`: `chartChanged`, `chartPatch`, `rangeToChartData` from `@oc/engine`.
4. Remove CanvasKit text measurement once oc-render is wired (phase 4 continues), then the TS packages (phase 6:
   done in plan 13 stage 4, 2026-10-27).
