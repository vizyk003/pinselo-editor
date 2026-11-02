// SPDX-License-Identifier: Apache-2.0
//! Export + thumbnail service — the Rust port of `services/render` (ADR-0001, phase 5).
//!
//! A drop-in for the Fastify service: same port (`OC_RENDER_PORT`, bind host `OC_RENDER_HOST`), the same
//! internal routes behind the internal service token, the same `export` job queue, SQL and outputs.
//! See `PORTING.md` for the per-module status.
//!
//! Layout:
//! - [`renderer`]: the [`renderer::Renderer`] trait — every rasterising/vector-drawing/text-measuring call the
//!   exporters make — implemented on crates/oc-render by [`oc_renderer::OcRenderer`] ([`renderer::StubRenderer`]
//!   answers 501 everywhere, for builds without a painter).
//! - Pure ports with golden parity: [`doc_markdown`], [`docx`], [`site_reflow`], [`site_mobile`],
//!   [`doc_layout::blocks_of`].
//! - Writers over the renderer, verified against the TS twice (with the renderer's answers replayed, and with the
//!   real painter where the output is structural): [`site`], [`pptx`], [`doc_layout::doc_to_design`].
//! - Video: [`video`] (the timeline frame loop, MP4/GIF/M4A exports), [`media`] (ffmpeg decoding and the audio
//!   mix), [`gif`] (a port of the gifenc quantiser and encoder).
//! - The service: [`service`] (data access, document loading, Docs exports), [`app`] (routes, export job).
//!   The binary is `src/main.rs`.

pub mod app;
pub mod doc_layout;
pub mod doc_markdown;
pub mod docx;
pub mod geom;
pub mod gif;
pub mod lint;
pub mod media;
pub mod oc_renderer;
pub mod pm;
pub mod pptx;
pub mod print;
pub mod renderer;
pub mod service;
pub mod site;
pub mod site_mobile;
pub mod site_reflow;
pub mod video;
pub mod watermark;
