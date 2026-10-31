// SPDX-License-Identifier: Apache-2.0
//! Resize a stored design document (JSON) to a new page size: the server-side twin of [`crate::Editor::resize_design`]
//! for "Copy and resize" (g101), which makes resized copies without opening them in an editor.

use oc_schema::primitives::Xf;
use oc_schema::tree::{children_of, sorted_pages};
use oc_schema::{DesignDoc, Unit};
use serde_json::{Map, Value};

use crate::transform::scale_patches;
use crate::util::{jv, min};

/// Set the page size of a design document to `w × h` (and `unit` when given). With `scale_content`, every page's
/// elements scale uniformly to fit and centre, exactly as the editor's Resize with "Scale content" does; without it
/// they keep their positions. Unknown fields are kept as they are.
pub fn resize_doc(doc: &mut Value, w: f64, h: f64, unit: Option<Unit>, scale_content: bool) -> Result<(), serde_json::Error> {
    let parsed: DesignDoc = serde_json::from_value(doc.clone())?;
    let (ow, oh) = (parsed.meta.dims.w, parsed.meta.dims.h);
    if let Some(dims) = doc.pointer_mut("/meta/dims").and_then(Value::as_object_mut) {
        dims.insert("w".into(), jv(&w));
        dims.insert("h".into(), jv(&h));
        if let Some(u) = unit {
            dims.insert("unit".into(), jv(&u));
        }
    }
    if !scale_content || ow <= 0.0 || oh <= 0.0 {
        return Ok(());
    }
    let sc = min(w / ow, h / oh);
    let dx = (w - ow * sc) / 2.0;
    let dy = (h - oh * sc) / 2.0;
    let mut patches: Vec<(String, Map<String, Value>)> = Vec::new();
    for p in sorted_pages(&parsed.pages) {
        for el in children_of(&parsed.elements, &p.id) {
            let Some(x) = el.xf() else { continue };
            let xf = Xf { x: x.x * sc + dx, y: x.y * sc + dy, w: x.w * sc, h: x.h * sc, ..x.clone() };
            let id = el.id().unwrap_or("").to_string();
            let mut own = Map::new();
            own.insert("xf".into(), jv(&xf));
            patches.push((id, own));
            for (id, patch) in scale_patches(&parsed.elements, el, sc, sc) {
                patches.push((id, patch));
            }
        }
    }
    if let Some(els) = doc.get_mut("elements").and_then(Value::as_object_mut) {
        for (id, patch) in patches {
            let Some(e) = els.get_mut(&id).and_then(Value::as_object_mut) else { continue };
            for (k, v) in patch {
                if v.is_null() {
                    e.remove(&k);
                } else {
                    e.insert(k, v);
                }
            }
        }
    }
    Ok(())
}
