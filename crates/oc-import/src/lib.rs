// SPDX-License-Identifier: Apache-2.0
//! File importers — the Rust port of `services/api/src/importers/*` (PowerPoint and PDF → editable design) and of
//! the Word → HTML conversion behind `POST /api/docs/import-docx` (mammoth in the TS). See docs/adr/0001-rust-core.md;
//! parity: `tests/golden/import/` (tools/golden/import.ts).
//!
//! Importers are synchronous; images are handed to a `media` callback that stores them and answers an asset id
//! (`None` when the file cannot be used), like the TS `opts.media`.

pub mod common;
pub mod docx;
pub mod pdf;
pub mod pptx;

pub use common::{ImportError, ImportOptions, MediaFn, map_font};
pub use pdf::import_pdf;
pub use pptx::import_pptx;
