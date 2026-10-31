// SPDX-License-Identifier: AGPL-3.0-only
//! Initial stored state for a new sheet design (the API writes it so clients never race to create tab 1).

use oc_crdt::rng::RandomSource;
use oc_schema::DesignDoc;

use crate::model::{InitOpts, init_sheet};

/// `encodeSheetDesign(doc)`: the design's Y.Doc with one empty 1000×26 tab, as a v1 update.
pub fn encode_sheet_design(doc: &DesignDoc, rng: &mut dyn RandomSource) -> Result<Vec<u8>, oc_crdt::error::CrdtError> {
    let ydoc = oc_crdt::codec::new_doc();
    oc_crdt::codec::to_y(doc, &ydoc, None)?;
    init_sheet(&ydoc, InitOpts::default(), rng);
    Ok(oc_crdt::codec::encode_state(&ydoc))
}
