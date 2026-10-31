// SPDX-License-Identifier: AGPL-3.0-only
//! Formula parity: sheets built by the TS generator (yjs update) evaluate to the same values, kinds,
//! displayed strings and normalised formula texts as HyperFormula produced.

mod common;

use common::*;
use serde_json::Value;

#[test]
fn formulas_match_hyperformula() {
    let g = load("formulas.json");
    let now = g["now"].as_f64().unwrap();
    let mut failures = Vec::new();
    let mut total = 0;
    for case in g["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let doc = doc_from(&[unb64(&case["update"])]);
        let e = engine(&doc, now, false);
        let extra = case.get("extra").cloned().unwrap_or(Value::Null);
        let actual = eval_json(&e, &extra);
        let expected = &case["results"];
        for (tab, cells) in expected.as_object().unwrap() {
            total += cells.as_object().unwrap().len();
            let _ = tab;
        }
        let mut d = Vec::new();
        diffs(name, &actual, expected, &mut d);
        if !d.is_empty() {
            eprintln!("case {name}: {} difference(s)", d.len());
        }
        failures.extend(d);
    }
    eprintln!("{total} cells compared");
    assert_no_diffs("formulas", &failures);
}
