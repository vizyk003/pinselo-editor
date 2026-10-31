// SPDX-License-Identifier: Apache-2.0
//! Every `fixtures/templates-seed/*.json`: validation result, migration and serde round trip.

mod common;

use common::*;
use oc_schema::DesignDoc;
use serde_json::{Value, json};

/// Deserialize → serialize yields semantically identical JSON (strict `serde_json::Value` equality: same keys,
/// same numbers including integer/float form, same order-insensitive objects).
#[test]
fn template_round_trip_is_identical() {
    let files = fixture_files();
    assert!(files.len() > 100, "expected the template fixtures, found {}", files.len());
    for file in &files {
        let raw = fixture(file);
        let doc: DesignDoc = serde_json::from_value(raw.clone()).unwrap_or_else(|e| panic!("{file}: {e}"));
        let back = j(&doc);
        if back != raw {
            panic!("{file}: round trip differs: {}", diff(&back, &raw, "$").unwrap_or_else(|| "number form".into()));
        }
        // And through the string form.
        let s = doc.to_json().expect("to_json");
        let again: Value = serde_json::from_str(&s).expect("reparse");
        assert_eq!(again, raw, "{file}: string round trip differs");
    }
}

#[test]
fn template_goldens_validate_and_migrate() {
    let g = golden("templates");
    let cases = g.as_array().expect("array");
    assert_eq!(cases.len(), fixture_files().len());
    for c in cases {
        let file = s(&c["file"]);
        let raw = fixture(file);
        let v = match oc_schema::validate_doc(&raw) {
            Ok(()) => json!({ "ok": true }),
            Err(issues) => json!({ "ok": false, "issues": issues }),
        };
        assert_eq!(diff(&v, &c["validation"], "$"), None, "{file}: validation");
        let doc: DesignDoc = de(&raw);
        assert_eq!(oc_schema::migrate::needs_migration(&doc), c["needsMigration"].as_bool().unwrap(), "{file}");
        match oc_schema::migrate::migrate(doc) {
            Ok(m) => assert_eq!(diff(&j(&m), &c["migrated"], "$"), None, "{file}: migrated"),
            Err(e) => assert_eq!(Some(e.to_string().as_str()), c["migrateError"].as_str(), "{file}"),
        }
        // Typed parse path.
        assert!(oc_schema::parse_doc(&raw).is_ok(), "{file}: parse_doc");
    }
}

/// Unknown fields and unknown element types survive a round trip (ADR-0011).
#[test]
fn unknown_fields_and_types_are_preserved() {
    let mut raw = fixture(&fixture_files()[0]);
    let pid = raw["pages"].as_object().unwrap().keys().next().unwrap().clone();
    raw["meta"]["futureMeta"] = json!({ "a": [1, 2.5, "x"] });
    raw["pages"][&pid]["futurePage"] = json!(true);
    let eid = raw["elements"].as_object().unwrap().keys().next().unwrap().clone();
    raw["elements"][&eid]["futureField"] = json!({ "nested": null });
    raw["elements"][&eid]["xf"]["skew"] = json!(3);
    raw["elements"]["zzzzzzzzzzzzzzzz"] = json!({ "id": "zzzzzzzzzzzzzzzz", "type": "hologram", "parentId": pid, "z": "b0", "beam": [1, 2] });
    raw["futureTop"] = json!("kept");
    let doc: DesignDoc = de(&raw);
    assert!(matches!(doc.elements["zzzzzzzzzzzzzzzz"], oc_schema::Element::Unknown(_)));
    assert_eq!(j(&doc), raw);
}
