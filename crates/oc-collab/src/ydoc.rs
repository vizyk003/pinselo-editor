//! yrs helpers for the yjs functions the server hooks use.

use std::panic::AssertUnwindSafe;

use serde_json::{Map, Value};
use yrs::updates::decoder::Decode;
use yrs::{Doc, Map as _, Out, ReadTxn, StateVector, Transact, Update};

/// `Y.decodeUpdate(update)` as a validity check. yrs is guarded against panics on hostile input.
pub fn decode_update(update: &[u8]) -> Result<Update, String> {
    match std::panic::catch_unwind(AssertUnwindSafe(|| Update::decode_v1(update))) {
        Ok(Ok(u)) => Ok(u),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("malformed update".into()),
    }
}

/// `Y.snapshotContainsUpdate(Y.snapshot(doc), update)`: every struct of the update is already within the
/// document's state vector and its delete set is a subset of the document's.
pub fn contains_update(doc: &Doc, update: &[u8]) -> Result<bool, String> {
    let u = decode_update(update)?;
    let txn = doc.transact();
    let snap = txn.snapshot();
    let sv = &snap.state_map;
    for (client, range) in u.insertions(true).iter() {
        let clock = sv.get(client);
        for r in range.iter() {
            if clock < r.end {
                return Ok(false);
            }
        }
    }
    // mergeDeleteSets([snapshot.ds, update.ds]) equals snapshot.ds (yrs keeps id sets canonical).
    if snap.delete_set.merge(u.delete_set()) != snap.delete_set {
        return Ok(false);
    }
    Ok(true)
}

/// `Y.encodeStateVector(doc)`.
pub fn state_vector(doc: &Doc) -> Vec<u8> {
    oc_crdt::encode_state_vector(doc)
}

/// `Y.encodeStateAsUpdate(doc, sv)` (y-protocols `readSyncStep1`).
pub fn diff(doc: &Doc, sv: &[u8]) -> Result<Vec<u8>, String> {
    let sv = match std::panic::catch_unwind(|| StateVector::decode_v1(sv)) {
        Ok(Ok(sv)) => sv,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err("malformed state vector".into()),
    };
    Ok(doc.transact().encode_state_as_update_v1(&sv))
}

/// Elements carrying a brand lock, read from the live document (`brandLocked` in main.ts).
pub fn brand_locked(doc: &Doc) -> Map<String, Value> {
    let r = oc_crdt::roots(doc);
    let txn = doc.transact();
    let mut out = Map::new();
    for (id, v) in r.elements.iter(&txn) {
        let Out::YMap(m) = v else { continue };
        let el = oc_crdt::read_element(&txn, &m);
        let by_brand = el.get("lock").and_then(|l| l.get("by")).and_then(Value::as_str) == Some("brand");
        if by_brand && !oc_crdt::codec::is_deleted(&txn, &m) {
            out.insert(id.to_string(), Value::Object(el));
        }
    }
    out
}

/// Apply the update to a scratch copy and compare the brand-locked elements (only when the doc has any).
pub fn brand_violations(doc: &Doc, update: &[u8]) -> Option<String> {
    let before = brand_locked(doc);
    if before.is_empty() {
        return None;
    }
    let scratch = oc_crdt::new_doc();
    oc_crdt::apply_update(&scratch, &oc_crdt::encode_state(doc), None).ok()?;
    let _ = oc_crdt::apply_update(&scratch, update, None);
    let r = oc_crdt::roots(&scratch);
    let txn = scratch.transact();
    let mut after = Map::new();
    for id in before.keys() {
        if let Some(m) = oc_crdt::codec::element_map(&txn, &r, id) {
            after.insert(id.clone(), Value::Object(oc_crdt::read_element(&txn, &m)));
        }
    }
    let before_m: Map<String, Value> = before;
    let v = oc_crdt::brand_lock_violations(&before_m, &after);
    v.first().map(|x| format!("brand lock: element {} — {}", x.id, x.reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::Text as _;

    #[test]
    fn contains_update_semantics() {
        let a = oc_crdt::new_doc();
        let t = a.get_or_insert_text("t");
        let u1 = {
            let mut txn = a.transact_mut();
            t.insert(&mut txn, 0, "hello");
            txn.encode_update_v1()
        };
        let b = oc_crdt::new_doc();
        assert!(!contains_update(&b, &u1).unwrap());
        oc_crdt::apply_update(&b, &u1, None).unwrap();
        assert!(contains_update(&b, &u1).unwrap());
        // A deletion is new information even though no struct is new.
        let u2 = {
            let mut txn = a.transact_mut();
            t.remove_range(&mut txn, 0, 2);
            txn.encode_update_v1()
        };
        assert!(!contains_update(&b, &u2).unwrap());
        oc_crdt::apply_update(&b, &u2, None).unwrap();
        assert!(contains_update(&b, &u2).unwrap());
        // Empty update.
        assert!(contains_update(&b, &[0, 0]).unwrap());
        assert!(contains_update(&b, &[0xff, 0xff, 0xff]).is_err());
        let m = a.get_or_insert_map("m");
        let full = {
            let mut txn = a.transact_mut();
            m.insert(&mut txn, "k", 1);
            drop(txn);
            oc_crdt::encode_state(&a)
        };
        assert!(!contains_update(&b, &full).unwrap());
    }
}
