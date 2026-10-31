// SPDX-License-Identifier: Apache-2.0
//! Load-time repair and brand-lock checks against the TS results.

mod common;

use common::{assert_same, load, unb64};
use oc_crdt::brandlock::{BrandAction, body_change, brand_lock_error, brand_lock_violations};
use oc_crdt::repair::{repair_y, upgrade_y};
use oc_crdt::{apply_update, decode_doc_value, from_y_value, new_doc_with_client};
use serde_json::{Value, json};

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

fn opt(v: &Value) -> Option<&Value> {
    if v.is_null() || v == "__undefined__" { None } else { Some(v) }
}

fn action(a: &Value) -> BrandAction<'_> {
    match a["kind"].as_str().unwrap() {
        "move" => BrandAction::Move,
        "delete" => BrandAction::Delete,
        "content" => BrandAction::Content,
        "style" => BrandAction::Style,
        "xf" => BrandAction::Xf { from: opt(&a["from"]), to: opt(&a["to"]) },
        "patch" => BrandAction::Patch {
            keys: a["keys"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect(),
            xf: a.get("xf").map(|x| (opt(&x["from"]), opt(&x["to"]))),
            body: a.get("body").map(|x| (opt(&x["from"]), opt(&x["to"]))),
        },
        k => panic!("unknown action {k}"),
    }
}

#[test]
fn brand_lock_errors_match_ts() {
    let g = load("brandlock.json");
    let errors = g["errors"].as_array().unwrap();
    assert!(errors.len() > 200);
    for (i, c) in errors.iter().enumerate() {
        let lock = c.get("lock").filter(|l| !l.is_null());
        let out = brand_lock_error(lock, &action(&c["action"]));
        assert_eq!(json!(out), c["out"], "case {i}: {c}");
    }
}

#[test]
fn body_change_matches_ts() {
    let g = load("brandlock.json");
    for (i, c) in g["bodyChanges"].as_array().unwrap().iter().enumerate() {
        let out = body_change(opt(&c["a"]), opt(&c["b"]));
        assert_eq!(serde_json::to_value(out).unwrap(), c["out"], "case {i}: {c}");
    }
}

#[test]
fn brand_lock_violations_match_ts() {
    let g = load("brandlock.json");
    for (i, c) in g["violations"].as_array().unwrap().iter().enumerate() {
        let out = brand_lock_violations(c["before"].as_object().unwrap(), c["after"].as_object().unwrap());
        assert_eq!(serde_json::to_value(out).unwrap(), c["out"], "case {i}");
    }
}
