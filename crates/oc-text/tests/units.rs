// SPDX-License-Identifier: Apache-2.0
//! Pure helpers against tests/golden/text/units.json, plus registry/inspector behaviour.

mod common;

use oc_schema::text::{CaseTransform, TextBody};
use oc_text::{
    FaceRequest, FontError, FontInspector, FontRegistry, MagicBytesInspector, ParsingInspector, TextEngine, apply_case, fallback_families, inspect_font,
    scale_body,
};

#[test]
fn fallback_families_match_ts() {
    let u = common::golden("units.json");
    for c in u["fallbackFamilies"].as_array().unwrap() {
        let want: Vec<&str> = c["out"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(fallback_families(c["text"].as_str().unwrap()), want, "{:?}", c["text"]);
    }
}

#[test]
fn pick_face_matches_ts() {
    let u = common::golden("units.json");
    let reg = FontRegistry::new(common::manifest());
    for c in u["pickFace"].as_array().unwrap() {
        let (fam, face) = reg.pick_face(c["family"].as_str().unwrap(), c["weight"].as_f64().unwrap(), c["italic"].as_bool().unwrap()).unwrap();
        assert_eq!(fam.family, c["out"]["family"].as_str().unwrap(), "{c}");
        assert_eq!(face.file, c["out"]["file"].as_str().unwrap(), "{c}");
    }
    assert_eq!(reg.resolve_family("No Such Family"), "Inter");
    assert_eq!(reg.resolve_family("Lobster"), "Lobster");
}

#[test]
fn scale_body_matches_ts() {
    let u = common::golden("units.json");
    for c in u["scaleBody"].as_array().unwrap() {
        let body: TextBody = serde_json::from_value(c["body"].clone()).unwrap();
        let want: TextBody = serde_json::from_value(c["out"].clone()).unwrap();
        assert_eq!(scale_body(&body, c["s"].as_f64().unwrap()), want);
    }
}

#[test]
fn case_transforms() {
    assert_eq!(apply_case("straße and café", Some(&CaseTransform::Upper)), "STRASSE AND CAFÉ");
    assert_eq!(apply_case("MIXED Case ÉLAN ΟΔΟΣ", Some(&CaseTransform::Lower)), "mixed case élan οδος");
    // JS `\b` is ASCII-only (even with the `u` flag): a non-ASCII letter after an ASCII word character starts a
    // "word", and a non-ASCII letter at the start of the text or after a space does not (`ü` is not a word
    // character, so `\bü` never matches, but `üB` has a boundary before `B`). Expected values are Node's output,
    // and match `case-title` in tests/golden/text/hand.json.
    assert_eq!(apply_case("the café o'neil über-cool x_y 3d", Some(&CaseTransform::Title)), "The CafÉ O'Neil üBer-Cool X_y 3d");
    assert_eq!(apply_case("über ébc a_éb éa", Some(&CaseTransform::Title)), "üBer éBc A_ÉB éA");
    assert_eq!(apply_case("ǆemal ﬁne 1st", Some(&CaseTransform::Title)), "ǆEmal ﬁNe 1st");
    // Full (multi-character) case mappings, as `toUpperCase`.
    assert_eq!(apply_case("a_ﬁ xǆ", Some(&CaseTransform::Title)), "A_FI XǄ");
    assert_eq!(apply_case("ǆemal ﬁne 1st", Some(&CaseTransform::Upper)), "ǄEMAL FINE 1ST");
    // Final sigma and İ, as `toLowerCase`.
    assert_eq!(apply_case("ΟΔΟΣ. Σ ΑΣ", Some(&CaseTransform::Lower)), "οδος. σ ας");
    assert_eq!(apply_case("İstanbul", Some(&CaseTransform::Lower)), "i\u{307}stanbul");
    assert_eq!(apply_case("as is", Some(&CaseTransform::None)), "as is");
    assert_eq!(apply_case("as is", None), "as is");
}

#[test]
fn registry_loads_by_hash_and_rejects_bad_bytes() {
    let mut reg = FontRegistry::new(common::manifest());
    let req = FaceRequest::new("Lobster", 400.0, false);
    let missing = reg.missing([&req]);
    assert_eq!(missing, vec!["lobster/Lobster_400Regular.ttf".to_owned()]);
    assert!(!reg.is_loaded("Lobster", 400.0, false));
    let bytes = std::fs::read(common::fonts_dir().join("files").join(&missing[0])).unwrap();
    // Wrong bytes for a pinned file fail the hash check.
    let inter = std::fs::read(common::fonts_dir().join("files/inter/Inter_400Regular.ttf")).unwrap();
    assert!(matches!(reg.insert_face(&missing[0], inter), Err(FontError::HashMismatch { .. })));
    assert!(matches!(reg.insert_face("nope.ttf", bytes.clone()), Err(FontError::UnknownFile(_))));
    let sha = reg.pick_face("Lobster", 400.0, false).unwrap().1.sha256.clone();
    assert_eq!(reg.face_by_sha256(&sha).unwrap().file, missing[0]);
    let g0 = reg.generation();
    reg.insert_face(&missing[0], bytes).unwrap();
    assert_eq!(reg.generation(), g0 + 1);
    assert!(reg.is_loaded("Lobster", 700.0, true), "Lobster has one face: every request resolves to it");
    assert!(reg.missing([&req]).is_empty());
    let face = reg.face_by_file(&missing[0]).unwrap();
    assert!(face.covers('L') && !face.covers('語'));
    assert_eq!(face.info.family, "Lobster");
}

#[test]
fn inspector() {
    let inter = std::fs::read(common::fonts_dir().join("files/inter/Inter_700Bold.ttf")).unwrap();
    let info = inspect_font(&inter).unwrap();
    assert_eq!((info.family.as_str(), info.weight, info.italic), ("Inter", 700, false));
    assert!(info.glyph_count > 100 && info.units_per_em > 0);
    assert!((info.metrics.ascent - 0.96875).abs() < 1e-9);
    let italic = std::fs::read(common::fonts_dir().join("files/inter/Inter_400Regular_Italic.ttf")).unwrap();
    assert!(inspect_font(&italic).unwrap().italic);
    assert!(ParsingInspector.validate(&inter).is_ok());
    // Magic number alone passes the TS check but not a real parse.
    let fake = [0u8, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3];
    assert!(MagicBytesInspector.validate(&fake).is_ok());
    assert!(ParsingInspector.validate(&fake).is_err());
    assert_eq!(MagicBytesInspector.validate(b"<html>").unwrap_err(), "not a font file");
    assert!(ParsingInspector.validate(b"<html>").is_err());
}

fn body(text: &str, width_family: &str) -> TextBody {
    serde_json::from_value(serde_json::json!({
        "defaultRun": { "family": width_family, "weight": 400, "italic": false, "size": 20,
            "fill": { "k": "solid", "color": { "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }, "tracking": 0 },
        "paragraphs": [{ "runs": [{ "text": text, "style": {} }], "align": "start", "lineHeight": 1.3, "spaceAfter": 0 }]
    }))
    .unwrap()
}

#[test]
fn layout_cache_and_font_readiness() {
    let mut e = TextEngine::new(FontRegistry::new(common::manifest()));
    let b = body("Hello world", "Lobster");
    assert!(!e.fonts_ready(&b));
    common::ensure(&mut e, &b);
    assert!(e.fonts_ready(&b));
    let a = e.layout("k", &b, 300.0, None);
    let again = e.layout("k", &b, 300.0, None);
    assert!(std::sync::Arc::ptr_eq(&a, &again), "same inputs hit the cache");
    let wider = e.layout("k", &b, 400.0, None);
    assert!(!std::sync::Arc::ptr_eq(&a, &wider));
    assert_eq!(a.paras[0].lines.len(), 1);
    let run = &a.paras[0].lines[0].glyph_runs[0];
    assert_eq!(run.face, "lobster/Lobster_400Regular.ttf");
    assert_eq!(run.glyphs.len(), 11);
    // Glyphs advance left to right and stay within the line.
    let xs: Vec<f64> = run.glyphs.iter().map(|g| g.x).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]));
    assert!(a.paras[0].lines[0].width > 50.0 && a.paras[0].lines[0].width < 300.0);
}

#[test]
fn layout_without_fonts_does_not_panic() {
    let mut e = TextEngine::new(FontRegistry::new(common::manifest()));
    let l = e.layout_uncached(&body("No fonts loaded yet", "Inter"), 200.0, None);
    assert_eq!(l.paras.len(), 1);
    assert!(l.height >= 1.0);
}

#[test]
fn caret_basics() {
    let mut e = common::engine();
    let b = body("one two three", "Inter");
    common::ensure(&mut e, &b);
    let l = e.layout("k", &b, 60.0, None);
    assert!(l.paras[0].lines.len() >= 2);
    assert_eq!(l.word_at(5), (4, 7));
    let c0 = l.caret_rect(0);
    assert_eq!(c0.x, 0.0);
    let end = l.caret_rect(l.length);
    assert!(end.y > c0.y);
    assert_eq!(l.offset_at(-10.0, -10.0), 0);
    assert_eq!(l.offset_at(1e4, 1e4), l.length);
    assert_eq!(l.vertical_move(0, -1), 0);
    assert_eq!(l.vertical_move(l.length, 1), l.length);
    let sel = l.selection_rects(2, 9);
    assert!(sel.len() >= 2);
}

#[test]
fn missing_faces_are_reported_once_and_relayout_after_load() {
    let mut e = common::engine();
    let b = body("Hello world, a wider line of text", "Lobster");
    // Off by default: nothing is recorded.
    e.layout("off", &b, 120.0, None);
    assert!(e.take_missing_faces().is_empty());
    e.report_missing_faces(true);
    let before = e.layout("k", &b, 120.0, None);
    let missing = e.take_missing_faces();
    let want: Vec<String> = e.missing_fonts(&b);
    assert!(!want.is_empty());
    assert_eq!(missing.iter().map(|m| m.file.clone()).collect::<Vec<_>>(), want, "every missing file, in request order");
    assert_eq!((missing[0].family.as_str(), missing[0].weight, missing[0].italic), ("Lobster", 400.0, false));
    // Laid out again (other key, other width): already reported, not queued twice.
    e.layout("k2", &b, 200.0, None);
    e.curved("c", &b, 40.0);
    assert_eq!(e.pending_missing_faces(), 0);
    // A failed load may be retried.
    e.forget_missing_face(&want[0]);
    e.layout("k3", &b, 220.0, None);
    assert_eq!(e.take_missing_faces().len(), 1);
    // Loading the faces bumps the generation: the cached layout is redone with the real face.
    common::ensure(&mut e, &b);
    let after = e.layout("k", &b, 120.0, None);
    assert!(!std::sync::Arc::ptr_eq(&before, &after));
    let fresh = TextEngine::new(FontRegistry::new(common::manifest()));
    let mut fresh = fresh;
    common::ensure(&mut fresh, &b);
    let exact = fresh.layout("k", &b, 120.0, None);
    assert_eq!((after.height, after.paras[0].longest_line), (exact.height, exact.paras[0].longest_line));
    assert_ne!(before.paras[0].longest_line, after.paras[0].longest_line, "Lobster measures differently from the fallback face");
    assert!(e.take_missing_faces().is_empty());
}
