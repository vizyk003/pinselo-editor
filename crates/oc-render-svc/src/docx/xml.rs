// SPDX-License-Identifier: Apache-2.0
//! A minimal XML tree with the serialisation rules of the `xml` npm package the `docx` package uses: attributes in
//! insertion order, `&<>"'` escaped in text and attribute values, childless elements self-closed (`<w:p/>`).

use quick_xml::escape::escape;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    El(El),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct El {
    pub name: &'static str,
    pub attrs: Vec<(&'static str, String)>,
    pub kids: Vec<Node>,
}

impl El {
    pub fn new(name: &'static str) -> El {
        El {
            name,
            attrs: Vec::new(),
            kids: Vec::new(),
        }
    }

    /// Add an attribute.
    pub fn a(mut self, k: &'static str, v: impl Into<String>) -> El {
        self.attrs.push((k, v.into()));
        self
    }

    /// Add a child element.
    pub fn c(mut self, e: El) -> El {
        self.kids.push(Node::El(e));
        self
    }

    /// Add text content.
    pub fn t(mut self, s: impl Into<String>) -> El {
        self.kids.push(Node::Text(s.into()));
        self
    }

    pub fn push(&mut self, e: El) {
        self.kids.push(Node::El(e));
    }

    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty() && self.kids.is_empty()
    }

    pub fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.name);
        for (k, v) in &self.attrs {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape(v.as_str()));
            out.push('"');
        }
        if self.kids.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for k in &self.kids {
            match k {
                Node::El(e) => e.write(out),
                Node::Text(s) => out.push_str(&escape(s.as_str())),
            }
        }
        out.push_str("</");
        out.push_str(self.name);
        out.push('>');
    }

    pub fn to_xml(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

/// An element holding a single `w:val` attribute.
pub fn val(name: &'static str, v: impl Into<String>) -> El {
    El::new(name).a("w:val", v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_like_the_xml_package() {
        let e = El::new("w:p").c(El::new("w:r").c(El::new("w:t")
            .a("xml:space", "preserve")
            .t("a < b & \"c\" 'd'")));
        assert_eq!(
            e.to_xml(),
            r#"<w:p><w:r><w:t xml:space="preserve">a &lt; b &amp; &quot;c&quot; &apos;d&apos;</w:t></w:r></w:p>"#
        );
        assert_eq!(El::new("w:p").to_xml(), "<w:p/>");
        assert_eq!(val("w:b", "x&y").to_xml(), r#"<w:b w:val="x&amp;y"/>"#);
    }
}
