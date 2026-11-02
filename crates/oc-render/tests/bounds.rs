// SPDX-License-Identifier: Apache-2.0
//! Display-list groups, dirty regions and tiled painting (`Canvas::new_region`): the editor paints its scene in
//! fixed tiles and, after an edit, repaints only the tiles that meet the changed area.

mod common;

use common::*;
use oc_render::bounds::Dirty;
use oc_render::tiny_skia::Pixmap;
use oc_render::tiny_skia::BlendMode;
use oc_render::{Canvas, NoAssets, RenderOptions, ResolveOptions, dirty_region, layering, resolve_page};
use oc_schema::DesignDoc;
use serde_json::Value;

/// Tile size and filter margin of the editor's tiled painting (apps/web Canvas via @oc/engine).
const TILE: i32 = 128;

/// The margin a tile is painted with so filters see their whole input: the largest group reach, rounded up.
fn margin(r: &mut oc_render::Renderer, dls: &[&oc_render::DisplayList], scale: f64) -> i32 {
    let reach = dls.iter().flat_map(|dl| r.groups(dl)).map(|g| g.reach).fold(0.0, f64::max);
    (((reach * scale).ceil() as i32 + 2 + 15) / 16) * 16
}

/// Paint the tiles of a `w × h` frame that meet `only` (page space; all when `None`) into `frame`, like the
/// editor: each tile is a region canvas (`Canvas::new_region`) of the frame's device space, grown by the margin,
/// and only the tile itself is copied.
fn paint_tiles(r: &mut oc_render::Renderer, frame: &mut Pixmap, dl: &oc_render::DisplayList, scale: f64, m: i32, only: Option<&oc_schema::primitives::Rect>) -> usize {
    let groups = r.groups(dl);
    let (fw, fh) = (frame.width() as i32, frame.height() as i32);
    let mut painted = 0;
    for ty in (0..fh).step_by(TILE as usize) {
        for tx in (0..fw).step_by(TILE as usize) {
            let (tw, th) = (TILE.min(fw - tx), TILE.min(fh - ty));
            if let Some(a) = only {
                let (x0, y0) = ((a.x * scale).floor() as i32 - 1, (a.y * scale).floor() as i32 - 1);
                let (x1, y1) = (((a.x + a.w) * scale).ceil() as i32 + 1, ((a.y + a.h) * scale).ceil() as i32 + 1);
                if x1 <= tx || y1 <= ty || x0 >= tx + tw || y0 >= ty + th {
                    continue;
                }
            }
            painted += 1;
            let mut c = Canvas::new_region(tx - m, ty - m, (TILE + 2 * m) as u32, (TILE + 2 * m) as u32).expect("canvas");
            c.scale(scale as f32, scale as f32);
            r.draw_culled(&mut c, dl, &groups, &NoAssets, true, true);
            let sub = c.into_pixmap();
            let sw = sub.width() as usize;
            let (src, dst) = (sub.data(), frame.data_mut());
            for y in 0..th {
                let so = (((y + m) as usize) * sw + m as usize) * 4;
                let d = (((ty + y) as usize) * fw as usize + tx as usize) * 4;
                let n = tw as usize * 4;
                dst[d..d + n].copy_from_slice(&src[so..so + n]);
            }
        }
    }
    painted
}

fn frame(dl: &oc_render::DisplayList, scale: f64) -> Pixmap {
    Pixmap::new((dl.width * scale).round() as u32, (dl.height * scale).round() as u32).expect("pixmap")
}

fn diff_count(a: &Pixmap, b: &Pixmap) -> usize {
    a.data().chunks(4).zip(b.data().chunks(4)).filter(|(x, y)| x != y).count()
}

/// Tiled painting draws what the plain painter draws (up to rasterisation at tile edges: curves are chopped where
/// they cross a tile, which moves anti-aliased edge pixels by a few levels).
#[test]
fn tiles_paint_the_page() {
    let mut r = renderer();
    let dir = root().join("fixtures/templates-seed");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    for path in files.iter().step_by(9) {
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let v = if v.get("doc").is_some() { v["doc"].clone() } else { v };
        let doc: DesignDoc = de(&v);
        let Some(page) = oc_schema::tree::sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else { continue };
        let dl = resolve_page(&doc, &page, &ResolveOptions::default());
        ensure_fonts(&mut r, &dl);
        let scale = (500.0 / dl.width.max(dl.height)).min(1.0);
        let want = r.render(&dl, &NoAssets, &RenderOptions { scale, background: None }).expect("pixmap");
        let m = margin(&mut r, &[&dl], scale);
        let mut got = frame(&dl, scale);
        paint_tiles(&mut r, &mut got, &dl, scale, m, None);
        let off = got.data().chunks(4).zip(want.data().chunks(4)).filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 8)).count();
        let total = (got.width() * got.height()) as usize;
        assert!(off * 200 < total, "{}: {off} of {total} pixels differ from the plain render", path.display());
    }
}

/// Partial repaint: for elements of the seed templates (every fourth) moved, made translucent or deleted, repainting only the
/// tiles that meet the [`dirty_region`] over the old frame gives exactly (bit for bit) the full repaint.
#[test]
fn partial_repaint_matches_full_repaint() {
    let mut r = renderer();
    let dir = root().join("fixtures/templates-seed");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let (mut checked, mut areas, mut full, mut tiles_total, mut tiles_painted) = (0, 0, 0, 0, 0);
    for path in files.iter().step_by(4) {
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let v = if v.get("doc").is_some() { v["doc"].clone() } else { v };
        let doc: DesignDoc = de(&v);
        let Some(page) = oc_schema::tree::sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else { continue };
        let old = resolve_page(&doc, &page, &ResolveOptions::default());
        ensure_fonts(&mut r, &old);
        let scale = (500.0 / old.width.max(old.height)).min(1.0);
        let old_groups = r.groups(&old);
        let ids: Vec<String> = oc_schema::tree::children_of(&doc.elements, &page).iter().filter_map(|e| e.id().map(str::to_owned)).collect();
        for (n, id) in ids.iter().enumerate().take(6) {
            let mut nv = v.clone();
            let el = &mut nv["elements"][id.as_str()];
            match n % 3 {
                0 => {
                    el["xf"]["x"] = Value::from(el["xf"]["x"].as_f64().unwrap_or(0.0) + 37.5);
                    el["xf"]["y"] = Value::from(el["xf"]["y"].as_f64().unwrap_or(0.0) - 23.25);
                }
                1 => el["opacity"] = Value::from(0.5),
                _ => {
                    nv["elements"].as_object_mut().unwrap().remove(id.as_str());
                }
            }
            let ndoc: DesignDoc = de(&nv);
            let new = resolve_page(&ndoc, &page, &ResolveOptions::default());
            ensure_fonts(&mut r, &new);
            let new_groups = r.groups(&new);
            let m = margin(&mut r, &[&old, &new], scale);
            let mut base = frame(&old, scale);
            paint_tiles(&mut r, &mut base, &old, scale, m, None);
            let mut want = frame(&new, scale);
            let all = paint_tiles(&mut r, &mut want, &new, scale, m, None);
            checked += 1;
            match dirty_region(&old, &old_groups, &new, &new_groups) {
                Dirty::Full => full += 1,
                Dirty::None => assert_eq!(diff_count(&base, &want), 0, "{}: {id} reported unchanged", path.display()),
                Dirty::Area(a) => {
                    areas += 1;
                    let mut got = base.clone();
                    tiles_painted += paint_tiles(&mut r, &mut got, &new, scale, m, Some(&a));
                    tiles_total += all;
                    let d = diff_count(&got, &want);
                    if d > 0 {
                        let _ = std::fs::create_dir_all(root().join("target/bounds"));
                        got.save_png(root().join("target/bounds/got.png")).unwrap();
                        want.save_png(root().join("target/bounds/want.png")).unwrap();
                        panic!("{}: {id} (change {}) patched area {a:?}: {d} pixels differ from the full repaint", path.display(), n % 3);
                    }
                }
            }
        }
    }
    eprintln!("{checked} edits: {areas} partial (repainting {tiles_painted} of {tiles_total} tiles), {full} full");
    assert!(areas > checked * 9 / 10, "single-element edits repaint partially");
}

/// Layered painting during a gesture: the background and the groups below the changing ones, then the changing
/// groups, then the groups above (painted onto transparent and composited) equal the list painted at once, up to
/// the 8-bit rounding of the cached layer.
#[test]
fn layered_painting_matches_plain_painting() {
    let mut r = renderer();
    let dir = root().join("fixtures/templates-seed");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let (mut checked, mut split, mut worst) = (0, 0, 0u8);
    for path in files.iter().step_by(5) {
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let doc: DesignDoc = de(&v);
        let Some(page) = oc_schema::tree::sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else { continue };
        let old = resolve_page(&doc, &page, &ResolveOptions::default());
        ensure_fonts(&mut r, &old);
        let scale = (300.0 / old.width.max(old.height)).min(1.0);
        let old_groups = r.groups(&old);
        let ids: Vec<String> = oc_schema::tree::children_of(&doc.elements, &page).iter().filter_map(|e| e.id().map(str::to_owned)).collect();
        for (n, id) in ids.iter().enumerate().take(4) {
            let mut nv = v.clone();
            let el = &mut nv["elements"][id.as_str()];
            if n % 2 == 0 {
                el["xf"]["x"] = Value::from(el["xf"]["x"].as_f64().unwrap_or(0.0) + 21.0);
            } else {
                el["opacity"] = Value::from(0.3);
            }
            let new = resolve_page(&de::<DesignDoc>(&nv), &page, &ResolveOptions::default());
            ensure_fonts(&mut r, &new);
            let ng = r.groups(&new);
            checked += 1;
            let Some(l) = layering(&old, &old_groups, &new, &ng) else { continue };
            split += 1;
            let (w, h) = ((new.width * scale).round() as u32, (new.height * scale).round() as u32);
            let want = r.render(&new, &NoAssets, &RenderOptions { scale, background: None }).unwrap();
            let mut below = Canvas::new(w, h).unwrap();
            below.scale(scale as f32, scale as f32);
            r.draw_groups(&mut below, &new, &ng, 0..l.from, &NoAssets, true, true);
            let mut above = Canvas::new(w, h).unwrap();
            above.scale(scale as f32, scale as f32);
            r.draw_groups(&mut above, &new, &ng, l.to..ng.len(), &NoAssets, false, true);
            let above = above.into_pixmap();
            let mut c = Canvas::from_pixmap(below.into_pixmap());
            c.scale(scale as f32, scale as f32);
            r.draw_groups(&mut c, &new, &ng, l.from..l.to, &NoAssets, false, true);
            c.set_transform(oc_render::tiny_skia::Transform::identity());
            c.draw_device_pixmap(0, 0, &above, 1.0, BlendMode::SourceOver);
            let got = c.into_pixmap();
            let d = got.data().iter().zip(want.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
            worst = worst.max(d);
            assert!(d <= 3, "{}: {id} layered differs by {d}", path.display());
        }
    }
    eprintln!("{checked} edits, {split} layered, worst channel difference {worst}");
    assert!(split * 10 > checked * 8, "most single-element edits are layerable");
}

/// A moved element is recognised as a translation of its groups; any other change is not.
#[test]
fn moves_are_translations() {
    let v: Value = serde_json::from_slice(&std::fs::read(root().join("fixtures/templates-seed/mw-web-services-greenleaf.json")).unwrap()).unwrap();
    let doc: DesignDoc = de(&v);
    let page = oc_schema::tree::sorted_pages(&doc.pages)[0].id.to_string();
    let mut r = renderer();
    let old = resolve_page(&doc, &page, &ResolveOptions::default());
    let og = r.groups(&old);
    let ids: Vec<String> = oc_schema::tree::children_of(&doc.elements, &page).iter().filter_map(|e| e.id().map(str::to_owned)).collect();
    let mut moved = 0;
    for id in &ids {
        for (dx, op) in [(12.5, "move"), (0.0, "recolour")] {
            let mut nv = v.clone();
            let el = &mut nv["elements"][id.as_str()];
            if op == "move" {
                el["xf"]["x"] = Value::from(el["xf"]["x"].as_f64().unwrap() + dx);
            } else {
                el["opacity"] = Value::from(0.25);
            }
            let new = resolve_page(&de::<DesignDoc>(&nv), &page, &ResolveOptions::default());
            let ng = r.groups(&new);
            let Some(l) = layering(&old, &og, &new, &ng) else { continue };
            let t = oc_render::translation(&old, &og, l.from..l.to, &new, &ng, l.from..l.to);
            if op == "move" {
                // Children of a moved group are located in the group's space: the whole run moves.
                if let Some((x, y)) = t {
                    assert!((x - dx).abs() < 1e-9 && y.abs() < 1e-9, "{id}: {x},{y}");
                    moved += 1;
                }
            } else {
                assert_eq!(t, None, "{id}: a recolour is not a move");
            }
        }
    }
    assert!(moved * 10 >= ids.len() * 8, "{moved} of {} moves recognised", ids.len());
}

/// Typing: appending to a text element repaints its last line (from the edited run), and the refined region
/// still covers every pixel that changed.
#[test]
fn typing_dirties_the_edited_line() {
    let mut r = renderer();
    let dir = root().join("fixtures/templates-seed");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let (mut checked, mut smaller) = (0, 0);
    for path in files.iter().step_by(6) {
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let doc: DesignDoc = de(&v);
        let Some(page) = oc_schema::tree::sorted_pages(&doc.pages).first().map(|p| p.id.to_string()) else { continue };
        let old = resolve_page(&doc, &page, &ResolveOptions::default());
        ensure_fonts(&mut r, &old);
        let scale = (400.0 / old.width.max(old.height)).min(1.0);
        let og = r.groups(&old);
        for el in oc_schema::tree::children_of(&doc.elements, &page) {
            let oc_schema::Element::Text(t) = el else { continue };
            let Some(id) = el.id() else { continue };
            let mut nv = v.clone();
            let runs = nv["elements"][id]["body"]["paragraphs"].as_array_mut().and_then(|p| p.last_mut()).and_then(|p| p["runs"].as_array_mut());
            let Some(run) = runs.and_then(|r| r.last_mut()) else { continue };
            let text = run["text"].as_str().unwrap_or("").to_owned();
            run["text"] = Value::from(format!("{text}x"));
            let _ = t;
            let new = resolve_page(&de::<DesignDoc>(&nv), &page, &ResolveOptions::default());
            let ng = r.groups(&new);
            let coarse = dirty_region(&old, &og, &new, &ng);
            let fine = r.dirty_region_fine(&old, &og, &new, &ng);
            let (Dirty::Area(a), Dirty::Area(c)) = (&fine, &coarse) else { continue };
            checked += 1;
            if a.w * a.h < c.w * c.h * 0.9 {
                smaller += 1;
            }
            // Every changed pixel lies inside the refined area.
            let m = margin(&mut r, &[&old, &new], scale);
            let mut base = frame(&old, scale);
            paint_tiles(&mut r, &mut base, &old, scale, m, None);
            let mut want = frame(&new, scale);
            paint_tiles(&mut r, &mut want, &new, scale, m, None);
            let mut got = base.clone();
            paint_tiles(&mut r, &mut got, &new, scale, m, Some(a));
            assert_eq!(diff_count(&got, &want), 0, "{}: typing in {id}: refined area {a:?} misses changed pixels", path.display());
        }
    }
    eprintln!("{checked} typed texts, {smaller} with a region at least 10% smaller than the whole box");
    // Centred and right-aligned lines move as a whole when a character is appended (their region is the line);
    // single-line boxes are their line. Measured: 131 of 224.
    assert!(checked > 10 && smaller * 2 > checked, "typing usually repaints less than the whole text box");
}
