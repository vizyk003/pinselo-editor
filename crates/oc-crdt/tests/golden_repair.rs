// SPDX-License-Identifier: Apache-2.0
//! Load-time repair against the TS results.

mod common;

use common::{assert_same, load, unb64};
use oc_crdt::repair::{repair_y, upgrade_y};
use oc_crdt::{apply_update, decode_doc_value, from_y_value, new_doc_with_client};
use serde_json::Value;

const NOW: f64 = 1_700_000_000_000.0;

#[test]
fn repair_matches_ts() {
    let cases = load("repair.json");
    for c in cases.as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let doc = new_doc_with_client(3);
        apply_update(&doc, &unb64(&c["before"]), None).unwrap();
        let res = if c["upgrade"].as_bool() == Some(true) { upgrade_y(&doc, NOW) } else { Ok(repair_y(&doc, NOW)) };
        match (&res, c.get("error")) {
            (Err(e), Some(Value::String(msg))) => assert_eq!(&e.to_string(), msg, "{name}: error"),
            (Ok(report), None) => {
                assert_same(&serde_json::to_value(report).unwrap(), &c["report"], &format!("{name}: report"));
                assert_same(&serde_json::to_value(repair_y(&doc, NOW)).unwrap(), &c["second"], &format!("{name}: second pass"));
            }
            other => panic!("{name}: result {other:?} vs golden error {:?}", c.get("error")),
        }
        assert_same(&from_y_value(&doc), &c["snapshot"], &format!("{name}: snapshot"));
        let ts_after = decode_doc_value(&unb64(&c["after"])).unwrap();
        assert_same(&ts_after, &c["snapshot"], &format!("{name}: TS repaired update decoded by yrs"));
    }
}
