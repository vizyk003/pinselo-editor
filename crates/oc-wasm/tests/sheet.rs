// SPDX-License-Identifier: AGPL-3.0-only
//! Native tests of the sheet bridge: model writes on the mirror produce update bytes that rebuild the same sheet
//! in another mirror (the browser's yjs doc in production), remote updates recalculate, structural edits rewrite
//! formulas, and the structure revision moves only when a tab's shape changes.
#![cfg(feature = "sheet")]

use oc_wasm::sheet::SheetEngine;

fn pair() -> (SheetEngine, SheetEngine) {
    let mut a = SheetEngine::create(&[], Some(11), Some(1), Some(0.0), true).expect("a");
    a.init_sheet(None, Some(20), Some(6));
    let init: Vec<u8> = a.take_updates().concat_updates();
    let b = SheetEngine::create(&init, Some(12), Some(2), Some(0.0), true).expect("b");
    (a, b)
}

trait Concat {
    fn concat_updates(self) -> Vec<u8>;
}

impl Concat for Vec<Vec<u8>> {
    fn concat_updates(self) -> Vec<u8> {
        yrs::merge_updates_v1(self).expect("merge")
    }
}

fn forward(from: &mut SheetEngine, to: &mut SheetEngine) {
    for u in from.take_updates() {
        to.apply_update(&u).expect("update");
    }
}

fn tab(e: &SheetEngine) -> String {
    e.tab_ids()[0].clone()
}

#[test]
fn writes_travel_as_updates_and_recalculate() {
    let (mut a, mut b) = pair();
    assert_eq!(a.tab_ids(), b.tab_ids());
    let t = tab(&a);
    let v0 = a.engine().version;
    let st = a.set_raw(&t, 0, 0, "2").unwrap();
    assert_eq!(st % 2.0, 1.0, "updates pending");
    a.set_raw(&t, 0, 1, "=A1*21").unwrap();
    assert!(a.engine().version > v0);
    assert_eq!(a.engine().value(&t, 0, 1).to_json(), serde_json::json!(42));
    forward(&mut a, &mut b);
    assert_eq!(b.engine().value(&t, 0, 1).to_json(), serde_json::json!(42));
    assert_eq!(b.status() % 2.0, 0.0, "remote updates are not echoed");
}

#[test]
fn structural_edits_rewrite_formulas_on_both_sides() {
    let (mut a, mut b) = pair();
    let t = tab(&a);
    a.set_raw(&t, 0, 0, "5").unwrap();
    a.set_raw(&t, 1, 0, "=A1+1").unwrap();
    forward(&mut a, &mut b);
    let rev = b.structure_rev();
    b.insert(&t, "rows", 0, 2).unwrap();
    assert!(b.structure_rev() > rev, "rows changed");
    forward(&mut b, &mut a);
    assert_eq!(a.engine().tab(&t).unwrap().raw_at(3, 0), "=A3+1");
    assert_eq!(a.engine().value(&t, 3, 0).to_json(), serde_json::json!(6));
    let rev = a.structure_rev();
    a.set_raw(&t, 5, 5, "x").unwrap();
    assert_eq!(a.structure_rev(), rev, "a cell edit keeps the structure");
}

#[test]
fn tabs_cross_references_and_csv() {
    let (mut a, mut b) = pair();
    let t = tab(&a);
    let t2 = a.add_tab(Some("Data".into()), None);
    a.set_raw(&t2, 0, 0, "1,5").unwrap();
    a.set_raw(&t, 0, 0, "=Data!A1").unwrap();
    forward(&mut a, &mut b);
    assert_eq!(b.engine().tabs.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Sheet1", "Data"]);
    assert_eq!(b.engine().value(&t, 0, 0).to_json(), serde_json::json!("1,5"));
    assert_eq!(b.tab_to_csv(&t).unwrap(), "\"1,5\"");
    assert_eq!(b.unique_tab_name("data", None), "data 2");
}

#[test]
fn init_is_idempotent_and_has_sheet() {
    let mut a = SheetEngine::create(&[], Some(21), Some(3), Some(0.0), true).unwrap();
    assert!(!a.has_sheet());
    let id = a.init_sheet(None, None, None);
    assert!(a.has_sheet());
    assert_eq!(a.init_sheet(Some("Other".into()), None, None), id);
    assert_eq!(a.engine().tabs[0].rows.len(), 1000);
}
