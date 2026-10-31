//! Parity with tests/golden/platform/audit.json: a chain written by the TS `audit()` (read back through the
//! TS verifier's query) verifies in Rust, every stored hash is reproduced, and tampering is detected.
use oc_audit::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

pub fn ts_rows() -> Vec<AuditRow> {
    serde_json::from_value(golden("audit.json")["rows"].clone()).unwrap()
}

#[test]
fn ts_chain_verifies_in_rust() {
    let g = golden("audit.json");
    assert!(g["tsVerify"].is_null());
    let rows = ts_rows();
    assert!(rows.len() >= 8);
    assert_eq!(verify_rows(&rows), None);
    let mut prev = String::new();
    for r in &rows {
        assert_eq!(digest(&prev, &r.fields), r.hash.clone().unwrap(), "row {}", r.id);
        prev = r.hash.clone().unwrap();
    }
}

#[test]
fn tampering_is_detected() {
    let rows = ts_rows();
    let mut edited = rows.clone();
    edited[2].fields.meta = Some(serde_json::json!({"f": 0.3}));
    assert_eq!(verify_rows(&edited), Some(rows[2].id));
    let mut removed = rows.clone();
    removed.remove(3);
    assert_eq!(verify_rows(&removed), Some(rows[4].id));
    let mut unchained = rows.clone();
    unchained[5].hash = None;
    assert_eq!(verify_rows(&unchained), Some(rows[5].id));
    let mut retimed = rows.clone();
    retimed[1].fields.created_at += chrono::Duration::milliseconds(1);
    assert_eq!(verify_rows(&retimed), Some(rows[1].id));
}
