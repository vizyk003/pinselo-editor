// SPDX-License-Identifier: Apache-2.0
//! Typed views of a DocStore snapshot. The CRDT snapshot is JSON (`oc_crdt::DocSnapshot`); geometry and most
//! editor logic want `oc_schema` types. A [`DocView`] carries both: the raw JSON maps (used for clones, patches and
//! brand-lock checks, like the TS plain objects) and the parsed `DesignDoc`. Elements are parsed once and reused
//! while their snapshot `Arc` is unchanged.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use oc_crdt::DocSnapshot;
use oc_schema::elements::Element;
use oc_schema::factory::{MetaInit, make_meta};
use oc_schema::tree::{Elements, page_of};
use oc_schema::{DesignDoc, Extra};
use serde_json::{Map, Value};

/// A design snapshot as JSON + typed.
#[derive(Debug, Clone)]
pub struct DocView {
    pub snap: Arc<DocSnapshot>,
    /// Raw element JSON (overrides merged in for gesture views).
    pub raw: BTreeMap<String, Arc<Map<String, Value>>>,
    pub doc: DesignDoc,
}

impl DocView {
    pub fn els(&self) -> &Elements {
        &self.doc.elements
    }

    pub fn el(&self, id: &str) -> Option<&Element> {
        self.doc.elements.get(id)
    }

    /// `doc.elements[id]` when it exists and is not deleted.
    pub fn live(&self, id: &str) -> Option<&Element> {
        self.el(id).filter(|e| !e.is_deleted())
    }

    pub fn raw_el(&self, id: &str) -> Option<&Map<String, Value>> {
        self.raw.get(id).map(|a| &**a)
    }

    pub fn page_of(&self, id: &str) -> Option<String> {
        page_of(&self.doc.pages, &self.doc.elements, id).map(str::to_string)
    }

    pub fn has_page(&self, id: &str) -> bool {
        self.doc.pages.contains_key(id)
    }

    /// A locked page (g96): its elements can't be selected, moved or edited.
    pub fn page_locked(&self, page_id: &str) -> bool {
        self.doc.pages.get(page_id).is_some_and(|p| p.locked == Some(true))
    }

    /// Whether the element sits on a locked page.
    pub fn on_locked_page(&self, id: &str) -> bool {
        self.page_of(id).is_some_and(|p| self.page_locked(&p))
    }

    /// The JSON snapshot `{ meta, pages, elements, assets }` (elements include view overrides).
    pub fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert("meta".into(), Value::Object((*self.snap.meta).clone()));
        out.insert(
            "pages".into(),
            Value::Object(self.snap.pages.iter().map(|(k, v)| (k.clone(), Value::Object((**v).clone()))).collect()),
        );
        out.insert("elements".into(), Value::Object(self.raw.iter().map(|(k, v)| (k.clone(), Value::Object((**v).clone()))).collect()));
        out.insert("assets".into(), Value::Object((*self.snap.assets).clone()));
        Value::Object(out)
    }
}

fn parse_element(m: &Map<String, Value>) -> Element {
    serde_json::from_value(Value::Object(m.clone())).unwrap_or_else(|_| Element::Unknown(m.clone()))
}

/// Parses snapshots, reusing elements whose JSON `Arc` did not change.
#[derive(Debug, Default)]
pub struct ViewCache {
    last: Option<(Arc<DocSnapshot>, Arc<DocView>)>,
    els: HashMap<String, (Arc<Map<String, Value>>, Element)>,
}

impl ViewCache {
    pub fn view(&mut self, snap: &Arc<DocSnapshot>) -> Arc<DocView> {
        if let Some((s, v)) = &self.last
            && Arc::ptr_eq(s, snap)
        {
            return v.clone();
        }
        let meta = serde_json::from_value(Value::Object((*snap.meta).clone()))
            .unwrap_or_else(|_| make_meta(&MetaInit { w: 100.0, h: 100.0, ..Default::default() }));
        let pages = snap
            .pages
            .iter()
            .filter_map(|(k, v)| serde_json::from_value(Value::Object((**v).clone())).ok().map(|p| (k.clone(), p)))
            .collect();
        let assets = snap
            .assets
            .iter()
            .filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|a| (k.clone(), a)))
            .collect();
        let mut elements = Elements::new();
        let mut next_cache = HashMap::with_capacity(snap.elements.len());
        for (id, m) in &snap.elements {
            let el = match self.els.remove(id) {
                Some((prev, el)) if Arc::ptr_eq(&prev, m) => el,
                _ => parse_element(m),
            };
            elements.insert(id.clone(), el.clone());
            next_cache.insert(id.clone(), (m.clone(), el));
        }
        self.els = next_cache;
        let view = Arc::new(DocView {
            snap: snap.clone(),
            raw: snap.elements.clone(),
            doc: DesignDoc { meta, pages, elements, assets, extra: Extra::new() },
        });
        self.last = Some((snap.clone(), view.clone()));
        view
    }
}

/// `{ ...doc, elements: { ...doc.elements, [id]: { ...el, ...override } } }`.
pub fn with_overrides<'a>(base: &DocView, overrides: impl IntoIterator<Item = (&'a String, &'a Map<String, Value>)>) -> DocView {
    let mut v = base.clone();
    for (id, o) in overrides {
        let Some(raw) = base.raw.get(id) else { continue };
        let mut m = (**raw).clone();
        for (k, x) in o {
            m.insert(k.clone(), x.clone());
        }
        v.doc.elements.insert(id.clone(), parse_element(&m));
        v.raw.insert(id.clone(), Arc::new(m));
    }
    v
}
