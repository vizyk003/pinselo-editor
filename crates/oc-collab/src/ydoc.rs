//! yrs helpers for the yjs functions the server hooks use.

use std::panic::AssertUnwindSafe;

use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

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

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::Map as _;
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
