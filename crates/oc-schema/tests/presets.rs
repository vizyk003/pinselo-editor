// SPDX-License-Identifier: Apache-2.0
//! Port of packages/schema/src/presets.test.ts (the golden replay is in goldens.rs).

use std::collections::HashSet;

use oc_schema::factory::empty_design;
use oc_schema::presets::{
    POPULAR_LAYOUTS, PRESET_CATEGORIES, PresetCategoryId, PresetDesignOverrides, PresetGroup, SIZE_PRESETS, SizePreset, SizeSide, find_preset,
    locked_side, preset_design_init, preset_platforms, presets_in_category, push_recent_preset, search_presets, search_presets_with, try_new_presets,
};
use oc_schema::{Surface, Unit};
use serde_json::json;

fn ids(list: &[&SizePreset]) -> Vec<&'static str> {
    list.iter().map(|p| p.id).collect()
}

// ---- size presets

#[test]
fn unique_ids_and_positive_sizes_within_the_api_limit() {
    let ids: HashSet<&str> = SIZE_PRESETS.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), SIZE_PRESETS.len());
    for p in SIZE_PRESETS.iter() {
        assert!(p.w > 0.0, "{}", p.id);
        assert!(p.h > 0.0, "{}", p.id);
        // POST /api/designs caps custom sizes at 20000 px.
        assert!(p.w.max(p.h) <= 20000.0, "{}", p.id);
        // The i18n extractor reads labels with /label: '([^']+)'/.
        assert!(!p.label.contains('\''), "{}", p.id);
    }
}

#[test]
fn keep_the_ids_that_designs_templates_and_shortcuts_refer_to() {
    for id in [
        "ig-post", "ig-portrait", "ig-story", "fb-post", "fb-cover", "x-post", "linkedin-post", "linkedin-banner", "pinterest-pin", "yt-thumb", "yt-banner",
        "presentation", "presentation-43", "a4", "letter", "resume", "poster", "flyer", "business-card", "invitation", "logo", "email-header", "infographic",
        "whiteboard", "doc", "sheet", "website", "link-in-bio", "email",
    ] {
        assert!(find_preset(id).is_some(), "{id}");
    }
}

#[test]
fn offer_video_presets_on_the_video_surface() {
    let v = find_preset("video").unwrap();
    assert_eq!((v.w, v.h, &v.surface), (1920.0, 1080.0, &Surface::Video));
    let m = find_preset("video-mobile").unwrap();
    assert_eq!((m.w, m.h, &m.surface), (1080.0, 1920.0, &Surface::Video));
    for id in ["tiktok-video", "yt-video", "yt-shorts", "fb-video", "ig-reel", "video-square"] {
        assert_eq!(find_preset(id).map(|p| &p.surface), Some(&Surface::Video), "{id}");
    }
    assert!(presets_in_category(&PresetCategoryId::Video).iter().all(|p| p.surface == Surface::Video));
}

#[test]
fn include_a3_and_us_legal_documents() {
    let a3 = find_preset("a3").unwrap();
    assert_eq!((&a3.unit, &a3.surface), (&Unit::Mm, &Surface::Print));
    assert_eq!(find_preset("legal").unwrap().h.round(), 14.0 * 96.0);
}

#[test]
fn builds_a_design_from_a_preset_with_custom_sizes_taking_precedence() {
    let v = preset_design_init(find_preset("video"), &PresetDesignOverrides::default());
    assert_eq!((v.w, v.h, &v.unit, &v.surface, v.title.as_str()), (1920.0, 1080.0, &Unit::Px, &Surface::Video, "Untitled – Video (Landscape)"));
    assert_eq!(empty_design(&v.meta_init()).meta.surface, Surface::Video);
    assert_eq!(empty_design(&v.into()).meta.surface, Surface::Video);
    let o = PresetDesignOverrides { w: Some(300.0), h: Some(200.0), unit: Some(Unit::Mm), ..Default::default() };
    let c = preset_design_init(None, &o);
    assert_eq!((c.w, c.h, &c.unit, &c.surface, c.title.as_str()), (300.0, 200.0, &Unit::Mm, &Surface::Graphic, "Untitled design"));
    let d = preset_design_init(find_preset("doc"), &PresetDesignOverrides { title: Some("Notes".into()), ..Default::default() });
    assert_eq!((&d.surface, d.title.as_str()), (&Surface::Doc, "Notes"));
}

// ---- create dialog helpers

#[test]
fn lists_every_grouped_preset_under_exactly_its_category() {
    let listed: HashSet<&str> = PRESET_CATEGORIES
        .iter()
        .flat_map(|c| presets_in_category(&c.id).into_iter().filter(|p| p.group != PresetGroup::Video || c.id == PresetCategoryId::Video).map(|p| p.id))
        .collect();
    assert_eq!(listed.len(), SIZE_PRESETS.len());
    let social = presets_in_category(&PresetCategoryId::Social);
    assert!(social.iter().any(|p| p.id == "ig-reel"));
    assert!(!social.iter().any(|p| p.id == "video"));
    let platforms = preset_platforms(&social);
    for name in ["Instagram", "TikTok", "YouTube", "WhatsApp"] {
        assert!(platforms.contains(&name), "{name}");
    }
    assert!(presets_in_category(&PresetCategoryId::Custom).is_empty());
}

#[test]
fn searches_labels_translations_platforms_keywords_and_sizes() {
    assert!(search_presets("").is_empty());
    let tiktok = ids(&search_presets("tiktok"));
    assert!(tiktok.contains(&"tiktok-video") && tiktok.contains(&"tiktok-photo"));
    assert_eq!(ids(&search_presets("twitter header")), ["x-header"]);
    assert!(ids(&search_presets("shorts")).contains(&"yt-shorts"));
    assert_eq!(ids(&search_presets("pageless")), ["doc"]);
    assert!(search_presets("1080x1920").len() > 3);
    let plakat = search_presets_with("Plakat", |s| if s.starts_with("Poster") { format!("Plakat {s}") } else { s.to_string() });
    assert!(plakat.iter().all(|p| p.label.starts_with("Poster")));
}

#[test]
fn keeps_a_capped_de_duplicated_most_recent_first_list() {
    assert_eq!(push_recent_preset(&json!(null), "a4", 8), ["a4"]);
    assert_eq!(push_recent_preset(&json!(["ig-post", "a4", "nope", 3]), "a4", 8), ["a4", "ig-post"]);
    assert_eq!(push_recent_preset(&json!(["ig-post"]), "unknown", 8), ["ig-post"]);
    let many: Vec<&str> = SIZE_PRESETS.iter().take(20).map(|p| p.id).collect();
    let mut expected = vec!["video"];
    expected.extend(&many[..4]);
    assert_eq!(push_recent_preset(&json!(many), "video", 5), expected);
}

#[test]
fn suggests_surfaces_the_person_has_not_used() {
    assert_eq!(try_new_presets(&[], 4).len(), 4);
    let after = try_new_presets(&["tiktok-video", "whiteboard"], 4);
    assert!(!after.iter().any(|p| p.surface == Surface::Video || p.surface == Surface::Whiteboard));
}

#[test]
fn keeps_the_aspect_ratio_of_a_locked_custom_size() {
    let (w, h) = (SizeSide::W, SizeSide::H);
    assert_eq!(locked_side(&w, 1920.0, 16.0 / 9.0, 2), 1080.0);
    assert_eq!(locked_side(&h, 1350.0, 1080.0 / 1350.0, 2), 1080.0);
    assert_eq!(locked_side(&w, 0.0, 1.0, 2), 0.0);
    // Whole pixels when asked (1920 × 1080 locked, width 1000).
    assert_eq!(locked_side(&w, 1000.0, 1920.0 / 1080.0, 2), 562.5);
    assert_eq!(locked_side(&w, 1000.0, 1920.0 / 1080.0, 0), 563.0);
    assert_eq!(locked_side(&h, 1000.0, 1920.0 / 1080.0, 0), 1778.0);
    assert_eq!(locked_side(&w, 1.0, 1000.0, 0), 1.0);
    assert!(POPULAR_LAYOUTS.iter().all(|l| l.w > 0.0 && l.h > 0.0));
}
