// SPDX-License-Identifier: Apache-2.0
//! Conversions against moxcms reference transforms and published FOGRA51 characterisation values. The ECI profiles
//! come from `pnpm icc:sync` (fixtures/icc); without them the profile tests print a note and pass.

use std::path::PathBuf;

use moxcms::{ColorProfile, Layout, RenderingIntent, TransformOptions};
use oc_schema::primitives::{CmykColor, ColorValue, SpotColor};

use super::*;

fn icc_dir() -> PathBuf {
    std::env::var_os("OC_ICC_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/icc"))
}

fn fogra51() -> Option<OutputProfile> {
    match registry::read_bundled("pso-coated-v3", &icc_dir()) {
        Ok(b) => Some(OutputProfile::new(b).expect("PSO Coated v3 loads")),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn de(a: [f64; 3], b: [f64; 3]) -> f64 {
    cie::delta_e_2000(a, b)
}

#[test]
fn inspect_classifies_profiles() {
    let s = inspect(registry::SRGB_ICC).expect("sRGB");
    assert_eq!((s.class.as_str(), s.color_space.as_str(), s.channels), ("mntr", "RGB", 3));
    assert!(inspect(b"not an icc profile at all").is_err());
    let mut broken = registry::SRGB_ICC.to_vec();
    broken.truncate(200);
    assert!(inspect(&broken).is_err(), "truncated profile");
    let Some(p) = fogra51() else { return };
    assert_eq!((p.info.class.as_str(), p.info.color_space.as_str(), p.info.channels), ("prtr", "CMYK", 4));
    assert!(p.info.description.contains("PSO Coated v3"), "{}", p.info.description);
}

#[test]
fn rgb_profile_has_no_cmyk_converter() {
    let p = OutputProfile::new(registry::SRGB_ICC.to_vec()).expect("sRGB as output");
    assert!(!p.is_cmyk());
    assert!(p.converter(Intent::Relative).is_err());
}

#[test]
fn matches_moxcms_reference_transform() {
    // Without BPC our path (exact sRGB matrix → Lab → profile B2A1) must agree with moxcms' own sRGB → CMYK.
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let raw = Converter { black: None, lut: OnceLock::new(), ..Converter::new(&p.profile, Intent::Relative).unwrap() };
    let srgb = ColorProfile::new_srgb();
    let opts = TransformOptions { rendering_intent: RenderingIntent::RelativeColorimetric, prefer_fixed_point: false, ..TransformOptions::default() };
    let reference = srgb.create_transform_f32(Layout::Rgb, &p.profile, Layout::Rgba, opts).unwrap();
    for rgb in [[0.8, 0.2, 0.2], [0.2, 0.5, 0.8], [0.5, 0.5, 0.5], [0.9, 0.85, 0.3], [0.1, 0.6, 0.3], [0.95, 0.95, 0.95]] {
        let mut want = [0f32; 4];
        reference.transform(&rgb.map(|v| v as f32), &mut want).unwrap();
        let want = want.map(f64::from);
        let got = raw.srgb_to_cmyk(rgb);
        let d = de(raw.cmyk_to_lab(want), raw.cmyk_to_lab(got));
        // Both are 33³ LUT interpolations, over different grids (gamma-encoded RGB vs ∛XYZ).
        assert!(d < 1.5, "{rgb:?}: moxcms {want:?} vs ours {got:?} (ΔE00 {d:.2})");
        // BPC lifts by at most the black point's share: small in the mid-tones, nothing at white.
        if rgb.iter().all(|v| *v >= 0.5) {
            let d = de(cv.cmyk_to_lab(cv.srgb_to_cmyk(rgb)), cv.cmyk_to_lab(got));
            assert!(d < 2.5, "{rgb:?}: BPC moved it by ΔE00 {d:.2}");
        }
    }
}

#[test]
fn fogra51_solids_match_the_characterisation_data() {
    // FOGRA51 (PSO Coated v3) characterisation, M1, absolute L*a*b* of paper and solids; through the profile's
    // relative colorimetric A2B the paper is white and solids are scaled to it, so compare media-relative values.
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let paper = [95.0, 1.0, -4.0];
    let rel = |abs: [f64; 3]| {
        let (x, w) = (cie::lab_to_xyz(abs), cie::lab_to_xyz(paper));
        cie::xyz_to_lab([0, 1, 2].map(|i| x[i] / w[i] * cie::D50[i]))
    };
    let white = cv.cmyk_to_lab([0.0; 4]);
    assert!(de(white, [100.0, 0.0, 0.0]) < 0.5, "paper → {white:?}");
    for (cmyk, abs) in [
        ([1.0, 0.0, 0.0, 0.0], [55.0, -37.0, -50.0]),
        ([0.0, 1.0, 0.0, 0.0], [48.0, 75.0, -4.0]),
        ([0.0, 0.0, 1.0, 0.0], [89.0, -4.0, 93.0]),
        ([0.0, 0.0, 0.0, 1.0], [16.0, 0.0, 0.0]),
    ] {
        let got = cv.cmyk_to_lab(cmyk);
        let want = rel(abs);
        assert!(de(got, want) < 3.0, "{cmyk:?}: {got:?} vs FOGRA51 {want:?} (ΔE00 {:.2})", de(got, want));
    }
}

#[test]
fn white_black_and_ink_limits() {
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let w = cv.srgb_to_cmyk([1.0, 1.0, 1.0]);
    assert!(w.iter().sum::<f64>() < 0.02, "white → {w:?}");
    let k = cv.srgb_to_cmyk([0.0, 0.0, 0.0]);
    assert!(k[3] > 0.8 && k.iter().sum::<f64>() <= 3.05, "black → {k:?} (TAC 300 %)");
    // BPC: source black lands on the profile's black point instead of clipping.
    let bp = cv.black_point_l().unwrap();
    assert!((3.0..20.0).contains(&bp), "black point L* {bp}");
    let kl = cv.cmyk_to_lab(k)[0];
    assert!((kl - bp).abs() < 2.0, "black prints at L* {kl}, black point {bp}");
    // Shadows keep their separation (no clipping into the same CMYK).
    let k2 = cv.srgb_to_cmyk([0.06, 0.06, 0.06]);
    assert!(cv.cmyk_to_lab(k2)[0] > cv.cmyk_to_lab(k)[0] + 0.5, "{k:?} vs {k2:?}");
    // Neutral greys stay neutral.
    let g = cv.cmyk_to_lab(cv.srgb_to_cmyk([0.5, 0.5, 0.5]));
    assert!(g[1].abs() < 2.0 && g[2].abs() < 2.0, "{g:?}");
}

#[test]
fn gamut_check() {
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let rgb = |r, g, b| ColorValue::srgb(r, g, b, 1.0);
    let green = cv.gamut(&rgb(0.0, 1.0, 0.0));
    assert!(!green.in_gamut && green.delta_e > 5.0, "{green:?}");
    let blue = cv.gamut(&rgb(0.0, 0.0, 1.0));
    assert!(!blue.in_gamut, "{blue:?}");
    for c in [rgb(0.5, 0.5, 0.5), rgb(0.85, 0.75, 0.6), rgb(0.0, 0.0, 0.0), rgb(1.0, 1.0, 1.0)] {
        assert!(cv.gamut(&c).in_gamut, "{c:?}: {:?}", cv.gamut(&c));
    }
    let p3 = ColorValue::DisplayP3(oc_schema::primitives::RgbColor { r: 0.0, g: 1.0, b: 0.0, a: 1.0, extra: Default::default() });
    assert!(cv.gamut(&p3).delta_e > green.delta_e, "P3 green is further out than sRGB green");
    let cmyk = ColorValue::Cmyk(CmykColor { c: 1.0, m: 0.0, y: 0.0, k: 0.0, a: 1.0, extra: Default::default() });
    assert!(cv.gamut(&cmyk).in_gamut);
}

#[test]
fn perceptual_differs_from_relative() {
    let Some(p) = fogra51() else { return };
    let r = p.converter(Intent::Relative).unwrap().srgb_to_cmyk([0.0, 0.8, 0.2]);
    let q = p.converter(Intent::Perceptual).unwrap().srgb_to_cmyk([0.0, 0.8, 0.2]);
    assert!(r.iter().zip(q).any(|(a, b)| (a - b).abs() > 0.01), "{r:?} vs {q:?}");
}

#[test]
fn image_lut_tracks_direct_conversion() {
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let px: Vec<u8> = vec![255, 255, 255, 0, 0, 0, 200, 40, 40, 30, 120, 220, 128, 128, 128, 250, 200, 10];
    let out = cv.srgb8_to_cmyk8(&px);
    assert_eq!(out.len(), 6 * 4);
    for (i, rgb) in px.chunks(3).enumerate() {
        let direct = cv.srgb_to_cmyk(rgb.iter().map(|v| f64::from(*v) / 255.0).collect::<Vec<_>>().try_into().unwrap());
        for k in 0..4 {
            let d = (f64::from(out[i * 4 + k]) / 255.0 - direct[k]).abs();
            assert!(d < 0.03, "pixel {i} channel {k}: lut {} direct {:.3}", out[i * 4 + k], direct[k]);
        }
    }
}

#[test]
fn inks_pass_cmyk_and_spot_through() {
    let spots: SpotAlternates = [("PANTONE 186 C".to_string(), [0.0, 1.0, 0.81, 0.04])].into_iter().collect();
    let c = ColorValue::Cmyk(CmykColor { c: 0.1, m: 0.2, y: 0.3, k: 0.4, a: 1.0, extra: Default::default() });
    assert_eq!(ink(&c, None, &spots), Ink::Cmyk([0.1, 0.2, 0.3, 0.4]));
    let s = ColorValue::Spot(SpotColor { swatch: "PANTONE 186 C".into(), tint: 0.6, a: 1.0, extra: Default::default() });
    assert_eq!(ink(&s, None, &spots), Ink::Spot { name: "PANTONE 186 C".into(), tint: 0.6, alternate: [0.0, 1.0, 0.81, 0.04] });
    let u = ColorValue::Spot(SpotColor { swatch: "Gold".into(), tint: 1.0, a: 1.0, extra: Default::default() });
    assert!(matches!(ink(&u, None, &spots), Ink::Spot { alternate: SPOT_FALLBACK, .. }));
    if let Some(p) = fogra51() {
        let cv = p.converter(Intent::Relative).unwrap();
        // Authored CMYK is never round-tripped through the profile.
        assert_eq!(ink(&c, Some(&cv), &spots), Ink::Cmyk([0.1, 0.2, 0.3, 0.4]));
    }
}

#[test]
fn naive_fallback_inverts_the_display_conversion() {
    for rgb in [[0.2, 0.4, 0.6], [1.0, 0.0, 0.5], [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]] {
        let [c, m, y, k] = naive_cmyk(rgb);
        let back = oc_schema::color::to_rgba(&ColorValue::Cmyk(CmykColor { c, m, y, k, a: 1.0, extra: Default::default() }));
        assert!((back.r - rgb[0]).abs() < 1e-9 && (back.g - rgb[1]).abs() < 1e-9 && (back.b - rgb[2]).abs() < 1e-9);
    }
}

#[test]
fn pure_black_detection() {
    assert!(is_pure_black(&ColorValue::srgb(0.0, 0.0, 0.0, 1.0)));
    assert!(!is_pure_black(&ColorValue::srgb(0.0, 0.0, 0.0, 0.5)));
    assert!(!is_pure_black(&ColorValue::srgb(0.1, 0.1, 0.1, 1.0)));
    assert!(is_pure_black(&ColorValue::Cmyk(CmykColor { c: 0.0, m: 0.0, y: 0.0, k: 1.0, a: 1.0, extra: Default::default() })));
    assert!(!is_pure_black(&ColorValue::Cmyk(CmykColor { c: 0.6, m: 0.4, y: 0.4, k: 1.0, a: 1.0, extra: Default::default() })));
}


#[test]
fn grey_ramp_is_monotonic_and_neutral() {
    let Some(p) = fogra51() else { return };
    let cv = p.converter(Intent::Relative).unwrap();
    let mut last = -1.0;
    for i in 0..=40 {
        let v = f64::from(i) / 40.0;
        let lab = cv.cmyk_to_lab(cv.srgb_to_cmyk([v, v, v]));
        assert!(lab[0] >= last - 0.3, "grey {v}: L* {} after {last}", lab[0]);
        assert!(lab[1].abs() < 3.0 && lab[2].abs() < 4.0, "grey {v}: {lab:?}");
        last = lab[0];
    }
}

#[test]
fn every_bundled_cmyk_profile_converts() {
    for b in registry::bundled().iter().filter(|b| b.color_space == "CMYK") {
        let Ok(bytes) = registry::read_bundled(&b.id, &icc_dir()) else {
            eprintln!("skipping {}: not synced", b.id);
            continue;
        };
        let p = OutputProfile::new(bytes).unwrap_or_else(|e| panic!("{}: {e}", b.id));
        assert_eq!((p.info.class.as_str(), p.info.channels), ("prtr", 4), "{}", b.id);
        for intent in [Intent::Relative, Intent::Perceptual] {
            let cv = p.converter(intent).unwrap();
            let w = cv.srgb_to_cmyk([1.0, 1.0, 1.0]);
            assert!(w.iter().sum::<f64>() < 0.02, "{} {intent:?}: white → {w:?}", b.id);
            let k = cv.srgb_to_cmyk([0.0, 0.0, 0.0]);
            assert!(k[3] > 0.75, "{} {intent:?}: black → {k:?}", b.id);
        }
    }
}
