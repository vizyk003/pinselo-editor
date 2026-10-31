// SPDX-License-Identifier: AGPL-3.0-only
//! Pinselo sheets — the Rust port of `@oc/sheet`: the Y.Doc sheet model (same layout as the yjs one the
//! browser keeps), a HyperFormula-compatible formula engine, display formatting, CSV, XLSX import/export,
//! linked charts and sheet → table layout.
//!
//! # Parity
//!
//! The contract is the TS package as it runs (HyperFormula 3.4.0 with `@oc/sheet`'s configuration, exceljs
//! 4.4.0, yjs), pinned by the goldens in `tests/golden/sheet/` (`tools/golden/sheet.ts`, ADR-0001): formula
//! values, kinds, displays and normalised formula texts; recalculation after edits, row/column and tab
//! changes (formula rewrites written back included); yjs-built documents and concurrent merges; exceljs
//! workbooks; and the a1 / format / csv / chart / model / layout / linked helpers. Every function HyperFormula
//! registers (423) plus `CONCAT` is implemented.
//!
//! The engine is HyperFormula's model, not Excel's, down to its quirks: array formulas keep or lose their
//! spill area the stateful way HyperFormula does ([`formula::workbook::ArrayState`]); `SHEET()` numbers
//! sheets by creation; a live engine replays remote row/column inserts into its formulas without rewriting
//! their text, exactly like the TS engine.
//!
//! # Deviations
//!
//! - Clock and randomness are injected ([`formula::workbook::Host`]); the default host is fixed at the Unix
//!   epoch, the `native` feature adds a system host. `RAND()` uses that host's generator, not `Math.random`.
//! - `writeRows` always re-snapshots the tab after growing it (every TS caller passes a re-snapshot).
//! - XLSX: text holding a literal `_xHHHH_` sequence is escaped (`_x005F_xHHHH_`, the OOXML rule) and reads
//!   back verbatim; exceljs writes it raw, so readers decode it to the character. Import (new here: the TS
//!   package only exported) maps number formats back to `nf`/`dp`/`cur`, so HyperFormula's number kinds
//!   come back as formats with the same display. See [`xlsx`].
//! - Not modelled, both on cycles of array formulas: an array created without space while on a cycle that
//!   later leaves the cycle (HyperFormula shows its value in the anchor only, here it spills or is
//!   `#SPILL!`), and HyperFormula re-claiming such an array's area after a row/column deletion.

pub mod a1;
pub mod chart;
pub mod csv;
pub mod engine;
pub mod format;
pub mod formula;
pub mod jsnum;
pub mod linked;
pub mod model;
pub mod state;
pub mod table;
#[cfg(feature = "xlsx")]
pub mod xlsx;
