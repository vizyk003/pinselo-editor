// SPDX-License-Identifier: AGPL-3.0-only
//! Resolving linked-chart data from a sheet design's stored Yjs state (API and render service).

use std::collections::HashMap;

use oc_schema::{DesignDoc, Element};
use serde::Serialize;

use crate::a1::{Range, parse_range};
use crate::chart::{ChartData, chart_changed, chart_patch, range_to_chart_data};
use crate::engine::{EngineOpts, SheetEngine};
use crate::format::CellValue;
use crate::formula::workbook::Host;
use crate::model::find_tab;

/// A read-only engine over a stored state (`openSheetState`). Invalid updates leave an empty doc.
pub fn open_sheet_state(state: &[u8], host: Box<dyn Host>) -> SheetEngine {
    let doc = oc_crdt::codec::new_doc();
    let _ = oc_crdt::codec::apply_update(&doc, state, None);
    SheetEngine::new(&doc, EngineOpts { live: false, host, ..EngineOpts::default() })
}

const MAX_CELLS: i64 = 20_000;

/// Values of a range read through [`read_range`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeRead {
    pub tab_id: String,
    pub tab_name: String,
    pub values: Vec<Vec<CellValue>>,
}

/// Values of `range` on tab `sheet` (tab id or name; a "Tab!A1:B2" range overrides it), capped at 20,000 cells.
pub fn read_range(engine: &SheetEngine, sheet: &str, range: &str) -> Option<RangeRead> {
    let p = parse_range(range)?;
    let t = find_tab(&engine.tabs, p.sheet.as_deref().unwrap_or(sheet)).or_else(|| {
        if p.sheet.as_deref().is_some_and(|s| !s.is_empty()) { None } else { engine.tabs.first() }
    })?;
    let r0 = p.range.r0 as i64;
    let c0 = p.range.c0 as i64;
    let mut r1 = (p.range.r1 as i64).min(t.rows.len() as i64 - 1);
    let c1 = (p.range.c1 as i64).min(t.cols.len() as i64 - 1);
    let w = c1 - c0 + 1;
    if (r1 - r0 + 1) * w > MAX_CELLS {
        r1 = r0 + (MAX_CELLS as f64 / w as f64).floor() as i64 - 1;
    }
    let values = if r1 < r0 || c1 < c0 {
        (0..(r1 - r0 + 1).max(0)).map(|_| Vec::new()).collect()
    } else {
        engine.range_values(&t.id, &Range { r0: r0 as usize, c0: c0 as usize, r1: r1 as usize, c1: c1 as usize })
    };
    Some(RangeRead { tab_id: t.id.clone(), tab_name: t.name.clone(), values })
}

/// Refresh every linked chart in `doc` from its source sheet. `load` returns a sheet design's state (or
/// `None` when missing / not accessible — the chart then keeps its cached values). Returns the number
/// refreshed. `host` provides the clock for each opened sheet.
pub fn resolve_linked_charts(
    doc: &mut DesignDoc,
    mut load: impl FnMut(&str) -> Option<Vec<u8>>,
    mut host: impl FnMut() -> Box<dyn Host>,
) -> usize {
    let ids: Vec<String> = doc
        .elements
        .iter()
        .filter(|(_, e)| matches!(e, Element::Chart(c) if c.source.is_some()) && !e.is_deleted())
        .map(|(id, _)| id.to_string())
        .collect();
    if ids.is_empty() {
        return 0;
    }
    let mut engines: HashMap<String, Option<SheetEngine>> = HashMap::new();
    let mut n = 0;
    for id in ids {
        let Some(Element::Chart(el)) = doc.elements.get(id.as_str()) else { continue };
        let src = el.source.clone().expect("filtered on source");
        if !engines.contains_key(&src.design_id) {
            let st = load(&src.design_id);
            engines.insert(src.design_id.clone(), st.map(|s| open_sheet_state(&s, host())));
        }
        let Some(Some(eng)) = engines.get(&src.design_id) else { continue };
        let Some(got) = read_range(eng, &src.sheet, &src.range) else { continue };
        let data: ChartData = range_to_chart_data(&got.values);
        if !chart_changed(el, &data) {
            continue;
        }
        let patch = chart_patch(el, &data, None);
        if let Some(Element::Chart(el)) = doc.elements.get_mut(id.as_str()) {
            patch.apply(el);
        }
        n += 1;
    }
    n
}
