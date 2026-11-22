// SPDX-License-Identifier: Apache-2.0
//! Counts designs with animated elements inside groups, which play from the release that animates group children
//! (before it they were drawn still). Reads `design_id<TAB>hex state` lines on stdin, for example from
//! `psql -At -F $'\t' -c "select design_id, encode(state, 'hex') from doc_state"`, and prints one line per affected
//! design plus a total. It only reads.

use std::io::BufRead;

use oc_schema::elements::Element;

fn hex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("\\x").unwrap_or(s);
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect()
}

fn main() {
    let (mut seen, mut hit, mut bad) = (0usize, 0usize, 0usize);
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Some((id, state)) = line.split_once('\t') else { continue };
        seen += 1;
        let Some(doc) = hex(state.trim()).and_then(|b| oc_crdt::codec::decode_doc(&b).ok()) else {
            bad += 1;
            continue;
        };
        let groups: std::collections::HashSet<&str> =
            doc.elements.values().filter(|e| matches!(e, Element::Group(_)) && !e.is_deleted()).filter_map(|e| e.id()).collect();
        let n = doc
            .elements
            .values()
            .filter(|e| !e.is_deleted())
            .filter_map(|e| e.base())
            .filter(|b| groups.contains(b.parent_id.as_str()))
            .filter(|b| {
                b.anim.is_some()
                    || b.anim_out.is_some()
                    || b.emphasis.is_some()
                    || b.motion_path.is_some()
                    || b.timing.is_some()
                    || b.keyframes.as_ref().is_some_and(|k| [&k.x, &k.y, &k.scale, &k.rotate, &k.opacity, &k.blur].iter().any(|t| t.as_ref().is_some_and(|t| !t.is_empty())))
            })
            .count();
        if n > 0 {
            hit += 1;
            println!("{id}\t{n} animated element(s) inside groups\t{:?}", doc.meta.surface);
        }
    }
    println!("{hit} of {seen} designs affected ({bad} could not be read)");
}
