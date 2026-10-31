// SPDX-License-Identifier: Apache-2.0
//! Raster painter behaviour: paints, stroke alignment, dashes, gradients, layers (opacity, blend, shadow, blur),
//! clips, images of every format (SVG recolouring, missing assets, video frames), QR codes, text and its effects,
//! transitions, encoders, and determinism over the hand-made scenes.

mod common;

use std::borrow::Cow;
use std::collections::HashMap;

use common::*;
use oc_render::displaylist::{DLItem, DisplayList, PathStroke, ResolvedPaint};
use oc_render::motion::transition_frame;
use oc_render::raster::transition::{Side, draw_transition};
use oc_render::tiny_skia::Pixmap;
use oc_render::{AssetProvider, Canvas, NoAssets, RenderOptions, ResolveOptions, VideoFrames, resolve_page};
use oc_schema::DesignDoc;
use oc_schema::design::TransitionKind;
use oc_schema::elements::{BlurEffect, DropShadowEffect, Effect, LineMarker};
use oc_schema::primitives::{BlendMode, ColorValue, Extra, GradientStop, ImageFit, Paint, Rect, StoredRect, Stroke, StrokeAlign, StrokeCap, StrokeJoin};
use oc_schema::text::{BackgroundTextEffect, HollowTextEffect, RunStyleOverride, TextAlign, TextEffect};

fn c(r: f64, g: f64, b: f64, a: f64) -> ColorValue {
    ColorValue::srgb(r, g, b, a)
}

fn solid(r: f64, g: f64, b: f64) -> ResolvedPaint {
    ResolvedPaint::Solid { color: c(r, g, b, 1.0) }
}

fn stroke(width: f64, align: StrokeAlign, dash: Option<Vec<f64>>) -> Stroke {
    Stroke { paint: Paint::solid_color(c(0.0, 0.0, 1.0, 1.0)), width, align, cap: StrokeCap::Butt, join: StrokeJoin::Miter, dash, extra: Extra::new() }
}

fn dl(w: f64, h: f64, items: Vec<DLItem>) -> DisplayList {
    let mut d = DisplayList::new(w, h);
    d.items = items;
    d
}

fn path(d: &str, fills: Vec<ResolvedPaint>, stroke: Option<(Stroke, ResolvedPaint)>) -> DLItem {
    DLItem::Path { path: d.into(), fills, stroke: stroke.map(|(stroke, paint)| PathStroke { stroke, paint }), src_el: "e".into() }
}

fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = p.pixel(x, y).expect("pixel").demultiply();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

fn render(items: Vec<DLItem>, w: f64, h: f64, assets: &dyn AssetProvider) -> Pixmap {
    let mut r = renderer();
    let d = dl(w, h, items);
    ensure_fonts(&mut r, &d);
    r.render(&d, assets, &RenderOptions::default()).expect("render")
}

const SQUARE: &str = "M20 20 L80 20 L80 80 L20 80 Z";

#[test]
fn fills_and_stroke_alignment() {
    let center = render(vec![path(SQUARE, vec![solid(1.0, 0.0, 0.0)], Some((stroke(10.0, StrokeAlign::Center, None), solid(0.0, 0.0, 1.0))))], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&center, 50, 50), [255, 0, 0, 255]);
    assert_eq!(px(&center, 17, 50), [0, 0, 255, 255]);
    assert_eq!(px(&center, 23, 50), [0, 0, 255, 255]);
    assert_eq!(px(&center, 10, 50)[3], 0);
    let inside = render(vec![path(SQUARE, vec![solid(1.0, 0.0, 0.0)], Some((stroke(10.0, StrokeAlign::Inside, None), solid(0.0, 0.0, 1.0))))], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&inside, 27, 50), [0, 0, 255, 255]);
    assert_eq!(px(&inside, 33, 50), [255, 0, 0, 255]);
    assert_eq!(px(&inside, 17, 50)[3], 0);
    let outside = render(vec![path(SQUARE, vec![solid(1.0, 0.0, 0.0)], Some((stroke(10.0, StrokeAlign::Outside, None), solid(0.0, 0.0, 1.0))))], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&outside, 13, 50), [0, 0, 255, 255]);
    assert_eq!(px(&outside, 23, 50), [255, 0, 0, 255]);
    assert_eq!(px(&outside, 5, 50)[3], 0);
}

#[test]
fn dashes_and_invalid_paths() {
    let dashed = render(vec![path("M0 50 L100 50", vec![], Some((stroke(4.0, StrokeAlign::Center, Some(vec![2.0, 2.0])), solid(0.0, 0.0, 0.0))))], 100.0, 100.0, &NoAssets);
    // Intervals scale with the width: 8 px on, 8 px off.
    assert_eq!(px(&dashed, 4, 50)[3], 255);
    assert_eq!(px(&dashed, 12, 50)[3], 0);
    assert_eq!(px(&dashed, 20, 50)[3], 255);
    // An odd interval count is no valid dash (Skia returns no path effect): the stroke is solid.
    let odd = render(vec![path("M0 50 L100 50", vec![], Some((stroke(4.0, StrokeAlign::Center, Some(vec![2.0])), solid(0.0, 0.0, 0.0))))], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&odd, 12, 50)[3], 255);
    // Malformed path data draws nothing (CanvasKit's MakeFromSVGString returns null).
    let bad = render(vec![path("M0 0 L100 0 L100 100 Z oops", vec![solid(0.0, 0.0, 0.0)], None)], 100.0, 100.0, &NoAssets);
    assert!(bad.pixels().iter().all(|p| p.alpha() == 0));
}

fn stop(t: f64, col: ColorValue) -> GradientStop {
    GradientStop { t, color: col, extra: Extra::new() }
}

#[test]
fn gradients() {
    let b = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
    let stops = vec![stop(0.0, c(1.0, 0.0, 0.0, 1.0)), stop(1.0, c(0.0, 0.0, 1.0, 1.0))];
    let lin = render(vec![path("M0 0 L100 0 L100 100 L0 100 Z", vec![ResolvedPaint::Linear { stops: stops.clone(), angle: 0.0, bbox: b }], None)], 100.0, 100.0, &NoAssets);
    let (l, rr) = (px(&lin, 2, 50), px(&lin, 97, 50));
    assert!(l[0] > 240 && l[2] < 15 && rr[2] > 240 && rr[0] < 15, "{l:?} {rr:?}");
    // 90°: top → bottom.
    let down = render(vec![path("M0 0 L100 0 L100 100 L0 100 Z", vec![ResolvedPaint::Linear { stops: stops.clone(), angle: 90.0, bbox: b }], None)], 100.0, 100.0, &NoAssets);
    assert!(px(&down, 50, 2)[0] > 240 && px(&down, 50, 97)[2] > 240);
    let rad = render(vec![path("M0 0 L100 0 L100 100 L0 100 Z", vec![ResolvedPaint::Radial { stops, bbox: b }], None)], 100.0, 100.0, &NoAssets);
    assert!(px(&rad, 50, 50)[0] > 240 && px(&rad, 99, 50)[2] > 240);
}

fn layer(opacity: f64, blend: BlendMode, effects: Vec<Effect>) -> DLItem {
    DLItem::Layer { opacity, blend, effects, src_el: "e".into() }
}

#[test]
fn layers_blend_and_effects() {
    let bg = path("M0 0 L100 0 L100 100 L0 100 Z", vec![solid(0.5, 0.5, 0.5)], None);
    let half = render(vec![bg.clone(), layer(0.5, BlendMode::Normal, vec![]), path(SQUARE, vec![solid(1.0, 1.0, 1.0)], None), DLItem::Restore], 100.0, 100.0, &NoAssets);
    let v = px(&half, 50, 50)[0];
    assert!((190..=193).contains(&v), "{v}");
    let mult = render(vec![bg.clone(), layer(1.0, BlendMode::Multiply, vec![]), path(SQUARE, vec![solid(1.0, 0.0, 0.0)], None), DLItem::Restore], 100.0, 100.0, &NoAssets);
    assert_eq!(&px(&mult, 50, 50)[..3], &[128, 0, 0]);
    let screen = render(vec![bg, layer(1.0, BlendMode::Screen, vec![]), path(SQUARE, vec![solid(1.0, 0.0, 0.0)], None), DLItem::Restore], 100.0, 100.0, &NoAssets);
    assert_eq!(&px(&screen, 50, 50)[..3], &[255, 128, 128]);
    let shadow = Effect::DropShadow(DropShadowEffect { dx: 10.0, dy: 10.0, blur: 0.0, color: c(0.0, 0.0, 0.0, 1.0), extra: Extra::new() });
    let sh = render(vec![layer(1.0, BlendMode::Normal, vec![shadow]), path("M10 10 L50 10 L50 50 L10 50 Z", vec![solid(1.0, 0.0, 0.0)], None), DLItem::Restore], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&sh, 30, 30), [255, 0, 0, 255]);
    assert_eq!(px(&sh, 55, 55), [0, 0, 0, 255]);
    assert_eq!(px(&sh, 5, 5)[3], 0);
    let blur = Effect::Blur(BlurEffect { radius: 8.0, extra: Extra::new() });
    let bl = render(vec![layer(1.0, BlendMode::Normal, vec![blur]), path(SQUARE, vec![solid(1.0, 0.0, 0.0)], None), DLItem::Restore], 100.0, 100.0, &NoAssets);
    let edge = px(&bl, 20, 50)[3];
    assert!((90..170).contains(&edge), "{edge}");
    assert!(px(&bl, 15, 50)[3] > 0 && px(&bl, 50, 50)[3] == 255);
}

#[test]
fn clips_nest_and_restore() {
    let items = vec![
        DLItem::Save { matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] },
        DLItem::Clip { path: "M0 0 L50 0 L50 100 L0 100 Z".into() },
        DLItem::Clip { path: "M0 0 L100 0 L100 50 L0 50 Z".into() },
        path("M0 0 L100 0 L100 100 L0 100 Z", vec![solid(1.0, 0.0, 0.0)], None),
        DLItem::Restore,
        path("M60 60 L100 60 L100 100 L60 100 Z", vec![solid(0.0, 1.0, 0.0)], None),
    ];
    let p = render(items, 100.0, 100.0, &NoAssets);
    assert_eq!(px(&p, 25, 25), [255, 0, 0, 255]);
    assert_eq!(px(&p, 75, 25)[3], 0);
    assert_eq!(px(&p, 25, 75)[3], 0);
    assert_eq!(px(&p, 80, 80), [0, 255, 0, 255]);
}

#[test]
fn lines_and_markers() {
    let s = Stroke { paint: Paint::solid_color(c(0.0, 0.0, 0.0, 1.0)), width: 4.0, align: StrokeAlign::Center, cap: StrokeCap::Butt, join: StrokeJoin::Miter, dash: None, extra: Extra::new() };
    let it = DLItem::Line { points: vec![[10.0, 50.0], [90.0, 50.0]], stroke: s, paint: solid(0.0, 0.0, 0.0), start_marker: LineMarker::None, end_marker: LineMarker::Triangle, src_el: "l".into() };
    let p = render(vec![it], 100.0, 100.0, &NoAssets);
    assert_eq!(px(&p, 30, 50)[3], 255);
    // The triangle head (12 px long, ±7.2 px wide) is filled at the end.
    assert_eq!(px(&p, 80, 54)[3], 255);
    assert_eq!(px(&p, 30, 53)[3], 0);
}

#[test]
fn qr_codes_are_crisp() {
    let q = oc_render::qr::create("https://example.com", oc_render::qr::Ecc::M).unwrap();
    let it = DLItem::Qr { modules: q.modules, size: q.size, bbox: Rect { x: 0.0, y: 0.0, w: 116.0, h: 116.0 }, fg: c(0.0, 0.0, 0.0, 1.0), bg: c(1.0, 1.0, 1.0, 1.0), src_el: "q".into() };
    let p = render(vec![it], 116.0, 116.0, &NoAssets);
    // 25 modules + 2 × 2 quiet modules → 4 px cells; the finder's dark ring starts at (8, 8).
    assert_eq!(px(&p, 9, 9), [0, 0, 0, 255]);
    assert_eq!(px(&p, 3, 3), [255, 255, 255, 255]);
    assert!(p.pixels().iter().all(|c| c.red() == 0 || c.red() == 255));
}

fn image(asset: &str, w: f64, h: f64) -> DLItem {
    DLItem::Image { asset: asset.into(), dst: Rect { x: 0.0, y: 0.0, w, h }, crop: StoredRect::from(Rect::UNIT), edits: None, clip: None, src_el: "i".into() }
}

#[test]
fn images_of_every_format() {
    let assets = scene_assets();
    for (key, w, h) in [("photo", 160.0, 100.0), ("tile", 64.0, 64.0), ("anim", 48.0, 48.0), ("pic", 96.0, 64.0), ("icon", 64.0, 64.0)] {
        let p = render(vec![image(key, w, h)], w, h, &assets);
        assert!(p.pixels().iter().any(|c| c.alpha() > 0), "{key} drew nothing");
        assert_ne!(px(&p, (w / 2.0) as u32, (h / 2.0) as u32), [224, 230, 237, 255], "{key} is the placeholder");
    }
    // The GIF's first frame is a white square on pink.
    let gif = render(vec![image("anim", 48.0, 48.0)], 48.0, 48.0, &assets);
    assert_eq!(px(&gif, 24, 24), [255, 255, 255, 255]);
    assert_eq!(px(&gif, 2, 2), [236, 72, 153, 255]);
    // Missing assets draw the light placeholder.
    let missing = render(vec![image("nope", 20.0, 20.0)], 20.0, 20.0, &assets);
    assert_eq!(px(&missing, 10, 10), [224, 230, 237, 255]);
}

#[test]
fn svg_recolor_is_derived_from_the_source() {
    let assets = scene_assets();
    let key = oc_schema::svg::recolor_key("icon", Some(&indexmap::IndexMap::from([("#ffffff".to_string(), "#22c55e".to_string())])));
    let p = render(vec![image(&key, 128.0, 128.0)], 128.0, 128.0, &assets);
    // The white circle (centre 32, 28 of 64) is now green.
    assert_eq!(px(&p, 64, 56), [0x22, 0xc5, 0x5e, 255]);
    let plain = render(vec![image("icon", 128.0, 128.0)], 128.0, 128.0, &assets);
    assert_eq!(px(&plain, 64, 56), [255, 255, 255, 255]);
}

struct RedFrames;

impl VideoFrames for RedFrames {
    fn frame(&self, _asset: &str, _t: f64, key: &str) -> Option<std::sync::Arc<Pixmap>> {
        assert_eq!(key, "v1");
        let mut p = Pixmap::new(8, 8)?;
        p.fill(oc_render::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        Some(std::sync::Arc::new(p))
    }
}

#[test]
fn video_frames_and_posters() {
    let assets = scene_assets();
    let video = |t: Option<f64>| DLItem::Video {
        asset: "clip".into(),
        dst: Rect { x: 0.0, y: 0.0, w: 160.0, h: 90.0 },
        crop: StoredRect::from(Rect::UNIT),
        source_time_ms: t,
        clip: None,
        key: Some("v1".into()),
        src_el: "v".into(),
        edits: None,
    };
    let mut r = renderer();
    r.video_frames = Some(Box::new(RedFrames));
    let frame = r.render(&dl(160.0, 90.0, vec![video(Some(500.0))]), &assets, &RenderOptions::default()).unwrap();
    assert_eq!(px(&frame, 80, 45), [255, 0, 0, 255]);
    // Without a source time (static renders) the poster shows: its play triangle is white.
    let poster = r.render(&dl(160.0, 90.0, vec![video(None)]), &assets, &RenderOptions::default()).unwrap();
    assert_eq!(px(&poster, 75, 45), [255, 255, 255, 255]);
    // No poster: the dark placeholder.
    let none = render(vec![video(None)], 160.0, 90.0, &NoAssets);
    assert_eq!(px(&none, 80, 45), [26, 26, 31, 255]);
}

fn text(s: &str, effect: Option<TextEffect>, style: RunStyleOverride) -> DLItem {
    DLItem::Text {
        key: format!("t-{s}-{}", effect.is_some()),
        body: oc_schema::factory::text_body(s, &style, TextAlign::Start),
        bbox: Rect { x: 10.0, y: 10.0, w: 380.0, h: 80.0 },
        effect,
        valign: None,
        curve: None,
        fit: None,
        src_el: "t".into(),
    }
}

fn ink(p: &Pixmap) -> usize {
    p.pixels().iter().filter(|c| c.alpha() > 128).count()
}

#[test]
fn text_and_effects() {
    let style = RunStyleOverride { size: Some(40.0), weight: Some(700.0), fill: Some(Paint::solid_color(c(0.0, 0.0, 0.0, 1.0))), ..Default::default() };
    let plain = render(vec![text("Hello", None, style.clone())], 400.0, 100.0, &NoAssets);
    let n = ink(&plain);
    assert!(n > 800, "glyphs drawn: {n}");
    let hollow = render(vec![text("Hello", Some(TextEffect::Hollow(HollowTextEffect { width: 4.0, extra: Extra::new() })), style.clone())], 400.0, 100.0, &NoAssets);
    assert!(ink(&hollow) < n, "hollow text is outlined");
    let bg = TextEffect::Background(BackgroundTextEffect { color: c(1.0, 1.0, 0.0, 1.0), padding: 10.0, radius: 4.0, extra: Extra::new() });
    let boxed = render(vec![text("Hello", Some(bg), style.clone())], 400.0, 100.0, &NoAssets);
    assert_eq!(px(&boxed, 3, 40), [255, 255, 0, 255]);
    let underlined = render(vec![text("Hello", None, RunStyleOverride { underline: Some(true), ..style.clone() })], 400.0, 100.0, &NoAssets);
    assert!(ink(&underlined) > n);
    // Deterministic.
    let again = render(vec![text("Hello", None, style)], 400.0, 100.0, &NoAssets);
    assert_eq!(plain.data(), again.data());
}

#[test]
fn transitions_draw_both_sides() {
    let mut r = renderer();
    let red = dl(100.0, 100.0, vec![path("M0 0 L100 0 L100 100 L0 100 Z", vec![solid(1.0, 0.0, 0.0)], None)]);
    let blue = dl(100.0, 100.0, vec![path("M0 0 L100 0 L100 100 L0 100 Z", vec![solid(0.0, 0.0, 1.0)], None)]);
    let mut c = Canvas::new(100, 100).unwrap();
    let frame = transition_frame(&TransitionKind::Push, 0.5, None);
    draw_transition(&mut c, 100.0, 100.0, &frame, |side, cv| r.draw(cv, if side == Side::Prev { &red } else { &blue }, &NoAssets));
    let p = c.into_pixmap();
    // Push from the right at half way: the previous page fills the left half, the next one the right half.
    assert_eq!(px(&p, 25, 50), [255, 0, 0, 255]);
    assert_eq!(px(&p, 75, 50), [0, 0, 255, 255]);
    let mut c = Canvas::new(100, 100).unwrap();
    draw_transition(&mut c, 100.0, 100.0, &transition_frame(&TransitionKind::Iris, 0.2, None), |side, cv| r.draw(cv, if side == Side::Prev { &red } else { &blue }, &NoAssets));
    let p = c.into_pixmap();
    assert_eq!(px(&p, 50, 50), [0, 0, 255, 255]);
    assert_eq!(px(&p, 2, 2), [255, 0, 0, 255]);
}

#[test]
fn encoders() {
    let p = render(vec![path(SQUARE, vec![ResolvedPaint::Solid { color: c(1.0, 0.0, 0.0, 0.5) }], None)], 100.0, 100.0, &NoAssets);
    let png = oc_render::encode_png(&p);
    let back = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(back.dimensions(), (100, 100));
    assert_eq!(back.get_pixel(50, 50).0, [255, 0, 0, 128]);
    let jpg = oc_render::encode_jpeg(&p, 90);
    let back = image::load_from_memory(&jpg).unwrap().into_rgb8();
    let px = back.get_pixel(50, 50).0;
    assert!(px[0] > 245 && px[1] > 115 && px[1] < 140, "{px:?}");
}

/// Every hand-made scene renders (fonts and committed assets), twice with identical pixels.
#[test]
fn scenes_render_deterministically() {
    let scenes = golden("scenes");
    let assets = scene_assets();
    for sc in scenes.as_array().expect("scenes") {
        let doc: DesignDoc = de(&sc["doc"]);
        let first = sc["cases"].as_array().unwrap()[0].clone();
        let page = s(&first["page"]);
        let opts: ResolveOptions = de(&first["opts"]);
        let d = resolve_page(&doc, page, &opts);
        let mut r = renderer();
        ensure_fonts(&mut r, &d);
        let a = r.render(&d, &assets, &RenderOptions { scale: 0.5, background: None }).unwrap();
        let b = r.render(&d, &assets, &RenderOptions { scale: 0.5, background: None }).unwrap();
        assert_eq!(a.data(), b.data(), "{} not deterministic", s(&sc["name"]));
        assert!(a.pixels().iter().any(|p| p.alpha() > 0), "{} drew nothing", s(&sc["name"]));
    }
}

/// Servers move renderers into worker threads.
#[test]
fn renderer_is_send() {
    fn send<T: Send>() {}
    send::<oc_render::Renderer>();
}

#[test]
fn asset_provider_impls() {
    let mut m: HashMap<String, Vec<u8>> = HashMap::new();
    m.insert("a".into(), vec![1, 2]);
    assert_eq!(m.asset("a"), Some(Cow::Borrowed(&[1u8, 2][..])));
    let f = oc_render::FnAssets(|k: &str| (k == "b").then(|| vec![3]));
    assert_eq!(f.asset("b").as_deref(), Some(&[3u8][..]));
    assert!(f.asset("c").is_none());
    let _ = ImageFit::Fill;
}
