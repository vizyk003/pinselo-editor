// SPDX-License-Identifier: Apache-2.0
//! Shared importer types and the Office font map (`mapFont` of importers/pptx.ts).

/// Store a media file (bytes, file name) as an asset; answers its id, or `None` when it can't be used.
pub type MediaFn<'a> = dyn FnMut(&[u8], &str) -> Option<String> + 'a;

/// `PptxImportOptions` / `PdfImportOptions` without `media` (passed separately).
pub struct ImportOptions<'a> {
    pub font_available: &'a dyn Fn(&str) -> bool,
    pub title: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a PowerPoint presentation")]
    NotPresentation,
    #[error("{0}")]
    Invalid(String),
}

impl ImportError {
    pub fn invalid(e: impl std::fmt::Display) -> Self {
        ImportError::Invalid(e.to_string())
    }
}

const FONT_MAP: &[(&str, &str)] = &[
    ("calibri", "Lato"), ("calibri light", "Lato"), ("arial", "Inter"), ("helvetica", "Inter"), ("helvetica neue", "Inter"),
    ("segoe ui", "Open Sans"), ("verdana", "Open Sans"), ("tahoma", "Open Sans"), ("times new roman", "PT Serif"), ("times", "PT Serif"),
    ("georgia", "Lora"), ("cambria", "Lora"), ("garamond", "EB Garamond"), ("book antiqua", "EB Garamond"), ("palatino", "EB Garamond"),
    ("century gothic", "Montserrat"), ("gill sans", "Lato"), ("futura", "Jost"), ("impact", "Anton"), ("arial black", "Archivo Black"),
    ("trebuchet ms", "Fira Sans"), ("consolas", "JetBrains Mono"), ("courier new", "Courier Prime"), ("courier", "Courier Prime"),
    ("comic sans ms", "Patrick Hand"), ("franklin gothic", "Libre Franklin"),
];

/// `\b\w` → upper case (JS `\w` = ASCII word characters).
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_word = false;
    for c in s.chars() {
        let word = c.is_ascii_alphanumeric() || c == '_';
        out.push(if word && !prev_word { c.to_ascii_uppercase() } else { c });
        prev_word = word;
    }
    out
}

/// JS `toLowerCase` then `.replace(/\s+(regular|bold|light|semibold|medium)$/, '')`.
fn strip_style_suffix(k: &str) -> &str {
    for suf in ["regular", "bold", "light", "semibold", "medium"] {
        if let Some(head) = k.strip_suffix(suf) {
            let trimmed = head.trim_end_matches(|c: char| c.is_whitespace());
            if trimmed.len() < head.len() {
                return trimmed;
            }
        }
    }
    k
}

/// Office fonts → families we ship (`mapFont`).
pub fn map_font(name: Option<&str>, available: &dyn Fn(&str) -> bool) -> String {
    let Some(name) = name.filter(|n| !n.is_empty()) else { return "Inter".into() };
    if available(name) {
        return name.into();
    }
    let lower = name.to_lowercase();
    let k = strip_style_suffix(&lower);
    if let Some((_, exact)) = FONT_MAP.iter().find(|(n, _)| *n == k)
        && available(exact)
    {
        return (*exact).into();
    }
    let t = title_case(k);
    if available(&t) { t } else { "Inter".into() }
}

/// Upper bounds for one decompressed package part: the sizes in a zip's headers are the uploader's claim, and a few
/// KB can inflate to gigabytes. XML is the tighter one because a parsed tree is many times larger than its text.
pub(crate) const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const MAX_XML_PART_BYTES: u64 = 64 * 1024 * 1024;

/// `read_to_end`, failing once more than `limit` bytes come out.
pub(crate) fn read_limited(r: impl std::io::Read, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    r.take(limit + 1).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "package part too large"));
    }
    Ok(out)
}

#[cfg(test)]
mod read_limited_tests {
    use super::read_limited;

    #[test]
    fn stops_at_the_limit() {
        assert_eq!(read_limited(&b"12345"[..], 5).unwrap(), b"12345");
        assert!(read_limited(&b"123456"[..], 5).is_err());
        assert!(read_limited(std::io::repeat(0), 1024).is_err()); // ends instead of filling memory
    }
}
