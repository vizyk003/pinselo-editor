# Vendored krilla 0.8.2 (OpenCanvas patches)

Upstream: https://github.com/LaurenzV/krilla, crate `krilla` 0.8.2 from crates.io, MIT OR Apache-2.0
(`LICENSE_MIT`, `LICENSE_APACHE`, `NOTICE.md` are upstream's). Wired in through `[patch.crates-io]` in the
workspace `Cargo.toml`; the crate is excluded from the workspace members. `examples/` and the crates.io
`Cargo.lock` were dropped; `publish = false` was added.

## Why a fork

OpenCanvas print export needs PDF/X-4 and PDF/X-1a:2003. krilla writes an `/OutputIntents` entry only for PDF/A
(sRGB, `/S /GTS_PDFA1`), has no PDF/X document-info keys and does not support overprint. Post-processing
krilla's output could add the catalog-level entries, but overprint has to be set in the graphics state of the
exact drawing operations (K-only black text and hairlines), which only the content writer knows. A small,
documented patch is more correct than rewriting content streams after the fact.

## Patches (all marked `OpenCanvas patch` in the source)

1. `src/configure/pdfx.rs` (new) and `src/configure/mod.rs`: `PdfX`, `PdfXVersion` (`X4`, `X1a2003`),
   `PdfXOutputIntent` (ICC bytes, components, `OutputConditionIdentifier`, `OutputCondition`, `RegistryName`,
   `Info`).
2. `src/serialize.rs`: `SerializeSettings::pdfx: Option<PdfX>` (default `None`); `serialize_destination_profiles`
   writes a `/S /GTS_PDFX` output intent with the embedded `/DestOutputProfile` when it is set.
3. `src/interchange/metadata.rs`, `src/chunk_container.rs`: with `pdfx` set, the document information dictionary
   is always written and gets `/GTS_PDFXVersion`, `/GTS_PDFXConformance` (X-1a) and `/Trapped`; the XMP packet
   gets `pdfxid:GTS_PDFXVersion`, `pdf:Trapped`, `xmpMM:VersionID` and `xmpMM:RenditionClass default`.
4. `src/graphics/graphics_state.rs`, `src/content.rs`, `src/surface.rs`: `Surface::push_overprint()` (popped
   with `Surface::pop`) sets `/OP true /op true /OPM 1` on everything drawn inside.
5. `src/graphics/xobject.rs`: with a CMYK PDF/X output intent, transparency groups blend in `/DeviceCMYK` (the
   output space, like InDesign's document-CMYK blend space) instead of sRGB.
6. `src/lib.rs`: `#![allow(deprecated)]` (upstream's `std::f32::INFINITY`; path dependencies don't cap lints).

Behaviour with `pdfx: None` and without `push_overprint` is byte-for-byte upstream's.

## Updating

Copy the new upstream release over `src/`, re-apply the five patches (grep for `OpenCanvas patch`), and run
`cargo test -p oc-render` (the PDF/X structure tests cover all of them).
