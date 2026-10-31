// SPDX-License-Identifier: AGPL-3.0-only
//! Recalc parity: the generator's edit sequences (cells, formats, row/column insert/delete, tab add/rename/
//! delete/reorder, merges, sizes, frozen panes, writeRows) replayed through the Rust engine leave the same tab
//! state (formula rewrites included) and the same computed values after every step.

mod common;

use common::*;
use serde_json::Value;

#[test]
fn recalc_sequences_match_hyperformula() {
    let g = load("recalc.json");
    let now = g["now"].as_f64().unwrap();
    let mut failures = Vec::new();
    let mut steps = 0;
    for case in g["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let seed = case["seed"].as_u64().unwrap() as u32;
        let (rows, cols) = (case["rows"].as_u64().unwrap() as usize, case["cols"].as_u64().unwrap() as usize);
        let (doc, rng) = sheet_doc(seed, rows, cols, 1);
        let mut e = engine_with(&doc, now, true, rng);
        for (i, st) in case["steps"].as_array().unwrap().iter().enumerate() {
            apply_step(&mut e, &st["step"]);
            steps += 1;
            let path = format!("{name}[{i}] {}", st["step"]["op"].as_str().unwrap());
            diffs(&format!("{path}.tabs"), &tabs_json(&e), &st["tabs"], &mut failures);
            diffs(&format!("{path}.values"), &eval_json(&e, &Value::Null), &st["values"], &mut failures);
        }
    }
    eprintln!("{steps} steps compared");
    assert_no_diffs("recalc", &failures);
}
