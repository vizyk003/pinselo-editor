// SPDX-License-Identifier: Apache-2.0
//! Pinselo CRDT layer — the Rust port of `@oc/crdt` (see docs/adr/0001-rust-core.md).
//!
//! The browser keeps a yjs `Y.Doc`; the server (and the WASM core) use yrs. Both sides read and write the same
//! Y.Doc layout — root maps `meta`, `pages`, `elements`, `assets` (+ `board`, `votes` for whiteboards, the
//! `docBody` XmlFragment for Docs), element maps with `props: Y.Map` and `text: Y.XmlFragment` — and the same
//! lib0 value encodings, so updates produced by one decode to the same snapshot in the other (verified against
//! goldens generated from the TS package in `tests/golden/crdt/`).
//!
//! * Snapshots are JSON (`serde_json::Value`) like the TS plain objects; typed `oc_schema::DesignDoc` wrappers
//!   are provided. Object keys read back from yrs are sorted (yrs does not keep insertion order).
//! * Text offsets are UTF-16 code units; create documents with [`new_doc`] (`OffsetKind::Utf16`).
//! * No clock, filesystem or threads: tombstone times are passed in, random ids come from a [`rng::RandomSource`].

pub mod board;
pub mod body;
pub mod brandlock;
pub mod codec;
pub mod doc_body;
pub mod error;
pub mod ops;
pub mod origins;
pub mod repair;
pub mod rng;
pub mod store;
pub mod text;
pub mod value;

pub use brandlock::{BodyChange, BrandAction, Violation, body_change, brand_lock_error, brand_lock_violations, is_brand_lock};
pub use codec::{
    Roots, apply_update, decode_doc, decode_doc_value, encode_diff, encode_doc, encode_doc_value, encode_state,
    encode_state_vector, from_y, from_y_value, new_doc, new_doc_with_client, read_element, read_meta, read_page,
    replace_content, roots, to_y, to_y_value, write_element, write_page,
};
pub use error::CrdtError;
pub use ops::{ApplyOptions, DocOp, DocOpError, DocOpErrorCode, DocOpResult, PageOp, apply_ops};
pub use origins::{BOARD_ORIGIN, OcOrigin, REPAIR_ORIGIN};
pub use store::{Change, DocSnapshot, DocStore};

