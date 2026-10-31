// SPDX-License-Identifier: Apache-2.0
//! mammoth's XML reading (xml/reader.js + docx/office-xml-reader.js): a small DOM with namespace URIs mapped to
//! fixed prefixes (`w:`, `r:`, … ; unknown namespaces as `{uri}local`), text nodes as xmldom produces them
//! (entity references merged, comments and CDATA dropped), and `mc:AlternateContent` collapsed to its fallback.

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

#[derive(Debug, Clone)]
pub enum XNode {
    Element(XElement),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct XElement {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<XNode>,
}

static EMPTY: XElement = XElement { name: String::new(), attributes: Vec::new(), children: Vec::new() };

impl XElement {
    /// `attributes[name]` (`None` = undefined).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn first(&self, name: &str) -> Option<&XElement> {
        self.children.iter().find_map(|c| match c {
            XNode::Element(e) if e.name == name => Some(e),
            _ => None,
        })
    }

    pub fn first_or_empty(&self, name: &str) -> &XElement {
        self.first(name).unwrap_or(&EMPTY)
    }

    /// Direct children with this name (`getElementsByTagName`).
    pub fn elements<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XElement> + 'a {
        self.children.iter().filter_map(move |c| match c {
            XNode::Element(e) if e.name == name => Some(e),
            _ => None,
        })
    }

    /// `Element.text()`: "" without children, the text of a single text child, otherwise an error.
    pub fn text(&self) -> Result<String, String> {
        match self.children.as_slice() {
            [] => Ok(String::new()),
            [XNode::Text(t)] => Ok(t.clone()),
            _ => Err("Not implemented".into()),
        }
    }
}

fn prefix_for(uri: &str) -> Option<&'static str> {
    Some(match uri {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main" | "http://purl.oclc.org/ooxml/wordprocessingml/main" => "w",
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships" | "http://purl.oclc.org/ooxml/officeDocument/relationships" => "r",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" | "http://purl.oclc.org/ooxml/drawingml/wordprocessingDrawing" => "wp",
        "http://schemas.openxmlformats.org/drawingml/2006/main" | "http://purl.oclc.org/ooxml/drawingml/main" => "a",
        "http://schemas.openxmlformats.org/drawingml/2006/picture" | "http://purl.oclc.org/ooxml/drawingml/picture" => "pic",
        "http://schemas.openxmlformats.org/package/2006/content-types" => "content-types",
        "http://schemas.openxmlformats.org/package/2006/relationships" => "relationships",
        "http://schemas.openxmlformats.org/markup-compatibility/2006" => "mc",
        "urn:schemas-microsoft-com:vml" => "v",
        "urn:schemas-microsoft-com:office:word" => "office-word",
        "http://schemas.microsoft.com/office/word/2010/wordml" => "wordml",
        _ => return None,
    })
}

fn convert_name(ns: &ResolveResult<'_>, local: &[u8]) -> Result<String, String> {
    let local = std::str::from_utf8(local).map_err(|e| e.to_string())?;
    match ns {
        ResolveResult::Unbound => Ok(local.to_owned()),
        ResolveResult::Bound(n) => {
            let uri = std::str::from_utf8(n.0).map_err(|e| e.to_string())?;
            Ok(match prefix_for(uri) {
                Some(p) => format!("{p}:{local}"),
                None => format!("{{{uri}}}{local}"),
            })
        }
        ResolveResult::Unknown(p) => Err(format!("unbound namespace prefix: {}", String::from_utf8_lossy(p))),
    }
}

fn predefined(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Parse an XML part (UTF-8 with the BOM stripped); any well-formedness problem is an error (mammoth's xmldom
/// wrapper throws on every reported problem).
pub fn parse(bytes: &[u8]) -> Result<XElement, String> {
    let s = String::from_utf8_lossy(bytes);
    let s = s.strip_prefix('\u{feff}').unwrap_or(&s);
    let mut reader = NsReader::from_str(s);
    let mut stack: Vec<XElement> = vec![XElement::default()];
    let mut text = String::new();
    let mut in_text = false;
    let flush = |stack: &mut Vec<XElement>, text: &mut String, in_text: &mut bool| {
        if *in_text {
            if let Some(top) = stack.last_mut() {
                top.children.push(XNode::Text(std::mem::take(text)));
            }
            *in_text = false;
        }
    };
    loop {
        let (ns, ev) = reader.read_resolved_event().map_err(|e| e.to_string())?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let ns = ns.clone();
                let name = convert_name(&ns, e.local_name().as_ref())?;
                let mut attributes = Vec::new();
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    let key = a.key.as_ref();
                    if key == b"xmlns" || key.starts_with(b"xmlns:") {
                        continue;
                    }
                    let (ans, local) = reader.resolver().resolve_attribute(a.key);
                    let k = if key.starts_with(b"xml:") {
                        format!("{{http://www.w3.org/XML/1998/namespace}}{}", String::from_utf8_lossy(local.as_ref()))
                    } else {
                        convert_name(&ans, local.as_ref())?
                    };
                    let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| e.to_string())?;
                    attributes.push((k, v.into_owned()));
                }
                flush(&mut stack, &mut text, &mut in_text);
                let el = XElement { name, attributes, children: vec![] };
                if matches!(ev, Event::Empty(_)) {
                    stack.last_mut().ok_or("bad nesting")?.children.push(XNode::Element(el));
                } else {
                    stack.push(el);
                }
            }
            Event::End(_) => {
                flush(&mut stack, &mut text, &mut in_text);
                if stack.len() < 2 {
                    return Err("unexpected end tag".into());
                }
                let el = stack.pop().ok_or("bad nesting")?;
                stack.last_mut().ok_or("bad nesting")?.children.push(XNode::Element(el));
            }
            Event::Text(t) => {
                if stack.len() > 1 {
                    text.push_str(&t.xml10_content().map_err(|e| e.to_string())?);
                    in_text = true;
                } else if !t.decode().map_err(|e| e.to_string())?.trim().is_empty() {
                    return Err("text outside the root element".into());
                }
            }
            Event::GeneralRef(r) => {
                let s = if let Some(c) = r.resolve_char_ref().map_err(|e| e.to_string())? {
                    c.to_string()
                } else {
                    let name = r.decode().map_err(|e| e.to_string())?;
                    predefined(&name).ok_or_else(|| format!("entity not found: &{name};"))?.to_owned()
                };
                if stack.len() > 1 {
                    text.push_str(&s);
                    in_text = true;
                } else {
                    return Err("reference outside the root element".into());
                }
            }
            Event::Comment(_) | Event::CData(_) | Event::PI(_) => flush(&mut stack, &mut text, &mut in_text),
            Event::Decl(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    if stack.len() != 1 {
        return Err("unclosed element".into());
    }
    let doc = stack.pop().ok_or("empty document")?;
    let root = doc
        .children
        .into_iter()
        .find_map(|c| match c {
            XNode::Element(e) => Some(e),
            XNode::Text(_) => None,
        })
        .ok_or("no root element")?;
    Ok(collapse_root(root))
}

/// `collapseAlternateContent` (the root is never an `mc:AlternateContent` in a docx part).
fn collapse_root(mut el: XElement) -> XElement {
    el.children = collapse_children(std::mem::take(&mut el.children));
    el
}

fn collapse_children(children: Vec<XNode>) -> Vec<XNode> {
    let mut out = Vec::with_capacity(children.len());
    for c in children {
        match c {
            XNode::Element(e) if e.name == "mc:AlternateContent" => {
                out.extend(e.first_or_empty("mc:Fallback").children.iter().cloned());
            }
            XNode::Element(e) => out.push(XNode::Element(collapse_root(e))),
            t => out.push(t),
        }
    }
    out
}
