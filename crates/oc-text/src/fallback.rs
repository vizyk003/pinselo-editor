// SPDX-License-Identifier: Apache-2.0
//! Glyph fallback by script (FT-031, `packages/text/src/fallback.ts`): text in any language or emoji renders
//! even when the chosen font lacks those characters. Each paragraph's font stack gets the Noto family for every
//! script it actually contains (loaded on demand from Google Fonts).

use icu_properties::props::{ExtendedPictographic, Script};
use icu_properties::{CodePointMapData, CodePointSetData};

enum Matcher {
    Pictographic,
    Scripts(&'static [Script]),
    Ranges(&'static [(u32, u32)]),
}

const SCRIPTS: &[(Matcher, &str)] = &[
    (Matcher::Pictographic, "Noto Color Emoji"),
    (Matcher::Scripts(&[Script::Hiragana, Script::Katakana]), "Noto Sans JP"),
    (Matcher::Scripts(&[Script::Hangul]), "Noto Sans KR"),
    (Matcher::Scripts(&[Script::Han]), "Noto Sans SC"),
    (Matcher::Scripts(&[Script::Arabic]), "Noto Sans Arabic"),
    (Matcher::Scripts(&[Script::Hebrew]), "Noto Sans Hebrew"),
    (Matcher::Scripts(&[Script::Devanagari]), "Noto Sans Devanagari"),
    (Matcher::Scripts(&[Script::Bengali]), "Noto Sans Bengali"),
    (Matcher::Scripts(&[Script::Tamil]), "Noto Sans Tamil"),
    (Matcher::Scripts(&[Script::Telugu]), "Noto Sans Telugu"),
    (Matcher::Scripts(&[Script::Gujarati]), "Noto Sans Gujarati"),
    (Matcher::Scripts(&[Script::Gurmukhi]), "Noto Sans Gurmukhi"),
    (Matcher::Scripts(&[Script::Kannada]), "Noto Sans Kannada"),
    (Matcher::Scripts(&[Script::Malayalam]), "Noto Sans Malayalam"),
    (Matcher::Scripts(&[Script::Sinhala]), "Noto Sans Sinhala"),
    (Matcher::Scripts(&[Script::Thai]), "Noto Sans Thai"),
    (Matcher::Scripts(&[Script::Lao]), "Noto Sans Lao"),
    (Matcher::Scripts(&[Script::Khmer]), "Noto Sans Khmer"),
    (Matcher::Scripts(&[Script::Myanmar]), "Noto Sans Myanmar"),
    (Matcher::Scripts(&[Script::Ethiopic]), "Noto Sans Ethiopic"),
    (Matcher::Scripts(&[Script::Armenian]), "Noto Sans Armenian"),
    (Matcher::Scripts(&[Script::Georgian]), "Noto Sans Georgian"),
    (Matcher::Ranges(&[(0x0370, 0x03FF), (0x0400, 0x04FF), (0x1E00, 0x1EFF)]), "Noto Sans"),
    (Matcher::Ranges(&[(0x2190, 0x23FF), (0x2460, 0x27BF), (0x2900, 0x2BFF)]), "Noto Sans Symbols 2"),
    (Matcher::Ranges(&[(0x2200, 0x22FF), (0x27C0, 0x27EF), (0x2980, 0x29FF)]), "Noto Sans Math"),
];

/// `/^[\x00-\x7f -ɏ -⁯₠-₿]*$/`: text that never needs a fallback family.
fn is_basic_latin(c: char) -> bool {
    matches!(c as u32, 0x00..=0x7F | 0xA0..=0x24F | 0x2000..=0x206F | 0x20A0..=0x20BF)
}

/// Fallback families needed by `text` (beyond Latin), in priority order (`fallbackFamilies`).
pub fn fallback_families(text: &str) -> Vec<&'static str> {
    if text.chars().all(is_basic_latin) {
        return Vec::new();
    }
    let script = CodePointMapData::<Script>::new();
    let pictographic = CodePointSetData::new::<ExtendedPictographic>();
    SCRIPTS
        .iter()
        .filter(|(m, _)| {
            text.chars().any(|c| match m {
                Matcher::Pictographic => pictographic.contains(c),
                Matcher::Scripts(s) => s.contains(&script.get(c)),
                Matcher::Ranges(r) => r.iter().any(|&(a, b)| (a..=b).contains(&(c as u32))),
            })
        })
        .map(|(_, f)| *f)
        .collect()
}
