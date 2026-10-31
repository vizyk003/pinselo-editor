// SPDX-License-Identifier: Apache-2.0
//! `caseTransform` exactly as the TS applies it (`String.prototype.toUpperCase/toLowerCase` and the title regex
//! `/\b(\p{L})/gu`, whose `\b` is the ASCII word boundary even in Unicode mode).

use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};
use oc_schema::text::CaseTransform;

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `applyCase(text, mode)`.
pub fn apply_case(text: &str, mode: Option<&CaseTransform>) -> String {
    match mode {
        Some(CaseTransform::Upper) => text.to_uppercase(),
        Some(CaseTransform::Lower) => text.to_lowercase(),
        Some(CaseTransform::Title) => {
            let gc = CodePointMapData::<GeneralCategory>::new();
            let mut out = String::with_capacity(text.len());
            let mut prev_word = false;
            for c in text.chars() {
                let word = is_ascii_word(c);
                if word != prev_word && GeneralCategoryGroup::Letter.contains(gc.get(c)) {
                    out.extend(c.to_uppercase());
                } else {
                    out.push(c);
                }
                prev_word = word;
            }
            out
        }
        _ => text.to_owned(),
    }
}
