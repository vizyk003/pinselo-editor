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
