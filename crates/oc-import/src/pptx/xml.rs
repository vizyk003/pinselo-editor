// SPDX-License-Identifier: Apache-2.0
//! A small ordered XML tree with fast-xml-parser's semantics as the TS importer configures it
//! (`preserveOrder`, `trimValues: false`, `parseTagValue: false`): text kept verbatim apart from the entity
//! references fast-xml-parser decodes (`&apos;` `&gt;` `&lt;` `&quot;` and their `#39`/`#x27`-style spellings, then
//! `&amp;`); other references (`&#65;`) stay as written. Attribute values get the same treatment.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

#[derive(Debug, Clone)]
pub enum Child {
    El(Node),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Child>,
}

/// fast-xml-parser `replaceEntitiesValue` (processEntities on, htmlEntities off).
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut v = s.to_owned();
    for (alts, val) in [
        (["&apos;", "&#39;", "&#x27;"], "'"),
        (["&gt;", "&#62;", "&#x3E;"], ">"),
        (["&lt;", "&#60;", "&#x3C;"], "<"),
        (["&quot;", "&#34;", "&#x22;"], "\""),
        (["&amp;", "&#38;", "&#x26;"], "&"),
    ] {
        // One regex pass over the alternatives (`/&(amp|#38|#x26);/g`): no replacement can form a new match of the
        // same entity, so sequential replaces are equivalent.
        for a in alts {
            if v.contains(a) {
                v = v.replace(a, val);
            }
        }
    }
    v
}

fn start_node(e: &quick_xml::events::BytesStart<'_>) -> Node {
    let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for a in e.html_attributes().with_checks(false).flatten() {
        let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let v = unescape(&String::from_utf8_lossy(&a.value));
        // A repeated attribute: the later value wins (object assignment).
        if let Some(slot) = attrs.iter_mut().find(|(n, _): &&mut (String, String)| *n == k) {
            slot.1 = v;
        } else {
            attrs.push((k, v));
        }
    }
    Node { tag, attrs, kids: Vec::new() }
}

/// Parse a document into its top-level children.
pub fn parse(xml: &str) -> Result<Vec<Child>, String> {
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let mut r = Reader::from_str(xml);
    {
        let c = r.config_mut();
        c.trim_text_start = false;
        c.trim_text_end = false;
        c.check_end_names = false;
        c.expand_empty_elements = false;
    }
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut text = String::new();
    let flush = |text: &mut String, stack: &mut Vec<Node>| {
        if !text.is_empty() {
            let t = unescape(text);
            if let Some(top) = stack.last_mut() {
                top.kids.push(Child::Text(t));
            }
            text.clear();
        }
    };
    loop {
        match r.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => {
                flush(&mut text, &mut stack);
                stack.push(start_node(&e));
            }
            Event::Empty(e) => {
                flush(&mut text, &mut stack);
                let n = start_node(&e);
                if let Some(top) = stack.last_mut() {
                    top.kids.push(Child::El(n));
                }
            }
            Event::End(_) => {
                flush(&mut text, &mut stack);
                if stack.len() > 1 {
                    let n = stack.pop().expect("open element");
                    if let Some(top) = stack.last_mut() {
                        top.kids.push(Child::El(n));
                    }
                }
            }
            Event::Text(t) => text.push_str(&t.decode().map_err(|e| e.to_string())?),
            Event::GeneralRef(g) => {
                text.push('&');
                text.push_str(&g.decode().map_err(|e| e.to_string())?);
                text.push(';');
            }
            Event::CData(c) => {
                flush(&mut text, &mut stack);
                let s = String::from_utf8_lossy(&c).into_owned();
                if !s.is_empty()
                    && let Some(top) = stack.last_mut()
                {
                    top.kids.push(Child::Text(s));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    flush(&mut text, &mut stack);
    // Unclosed elements: attach them to their parents.
    while stack.len() > 1 {
        let n = stack.pop().expect("open element");
        if let Some(top) = stack.last_mut() {
            top.kids.push(Child::El(n));
        }
    }
    Ok(stack.pop().map(|n| n.kids).unwrap_or_default())
}

/// The first top-level element with this tag (`rootOf`).
pub fn root_of<'a>(doc: &'a [Child], tag: &str) -> Option<&'a Node> {
    doc.iter().find_map(|c| match c {
        Child::El(n) if n.tag == tag => Some(n),
        _ => None,
    })
}

impl Node {
    /// Element children (text nodes skipped).
    pub fn els(&self) -> impl Iterator<Item = &Node> {
        self.kids.iter().filter_map(|c| match c {
            Child::El(n) => Some(n),
            Child::Text(_) => None,
        })
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn child(&self, tag: &str) -> Option<&Node> {
        self.els().find(|n| n.tag == tag)
    }
    pub fn children<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.els().filter(move |n| n.tag == tag)
    }
    /// Concatenated text children (`textOf`).
    pub fn text(&self) -> String {
        self.kids
            .iter()
            .filter_map(|c| match c {
                Child::Text(t) => Some(t.as_str()),
                Child::El(_) => None,
            })
            .collect()
    }
}

/// Helpers over `Option<&Node>` (the TS helpers accept `undefined`).
pub trait OptNode<'a> {
    fn attr(self, name: &str) -> Option<&'a str>;
    fn child(self, tag: &str) -> Option<&'a Node>;
    fn path(self, tags: &[&str]) -> Option<&'a Node>;
    fn children_vec(self, tag: &str) -> Vec<&'a Node>;
    fn text(self) -> String;
}

impl<'a> OptNode<'a> for Option<&'a Node> {
    fn attr(self, name: &str) -> Option<&'a str> {
        self?.attr(name)
    }
    fn child(self, tag: &str) -> Option<&'a Node> {
        self?.child(tag)
    }
    fn path(self, tags: &[&str]) -> Option<&'a Node> {
        tags.iter().try_fold(self?, |cur, t| cur.child(t))
    }
    fn children_vec(self, tag: &str) -> Vec<&'a Node> {
        self.map(|n| n.els().filter(|c| c.tag == tag).collect()).unwrap_or_default()
    }
    fn text(self) -> String {
        self.map(Node::text).unwrap_or_default()
    }
}
