// SPDX-License-Identifier: Apache-2.0
//! PowerPoint import (IM-060): .pptx → editable design — port of services/api/src/importers/pptx.ts.
//!
//! Shapes (presets and custom geometry), text boxes and placeholders (runs, fonts, sizes, colours, alignment,
//! bullets, spacing), pictures (crops), connectors (arrowheads), groups (flattened), tables, charts, backgrounds,
//! hidden slides and speaker notes. Theme colours are resolved.
//!
//! The arithmetic follows the TS expression by expression (same operation order, JS `Number()` parsing of
//! attributes, NaN-propagating `Math.max`/`Math.min`), so the output matches the TS DesignDoc up to generated ids.
//!
//! Deviation: whitespace-only text nodes between elements are skipped. fast-xml-parser (`trimValues: false`)
//! keeps them as children, and the TS then throws when one sits in a theme's colour scheme (or a custom-geometry
//! path); pretty-printed parts therefore fail to import in TS and import normally here. Office, Google Slides
//! and LibreOffice write parts without such whitespace.

mod xml;

use std::collections::HashMap;
use std::io::Cursor;

use indexmap::IndexMap;
use oc_schema::color::hex;
use oc_schema::elements::{
    CellSpan, ChartKind, DropShadowEffect, Effect, Element, LineMarker, SemanticRole, ShapeKind, TableCell,
};
use oc_schema::factory::{
    ChartOpts, LineOpts, MetaInit, SeriesInit, ShapeOpts, TableOpts, TextOpts, base_init, empty_design, make_chart, make_image,
    make_line, make_shape, make_table, make_text, place_all_elements,
};
use oc_schema::js;
use oc_schema::primitives::{
    ColorValue, Extra, GradientStop, ImageFit, ImagePaint, LinearPaint, Paint, RadialPaint, SolidPaint, StoredRect, Stroke,
    StrokeAlign, StrokeCap, StrokeJoin, Xf,
};
use oc_schema::text::{ListKind, ListSpec, Paragraph, Run, RunStyle, RunStyleOverride, TextAlign, TextBody, CaseTransform, apply_override};
use oc_schema::tree::sorted_pages;
use oc_schema::DesignDoc;

use crate::common::{ImportError, ImportOptions, MAX_PART_BYTES, MAX_XML_PART_BYTES, MediaFn, map_font, read_limited};
use xml::{Node, OptNode, root_of};

const EMU: f64 = 9525.0;

/// JS `Number(v)` of an attribute (`undefined` → NaN).
fn num(v: Option<&str>) -> f64 {
    v.map_or(f64::NAN, js::string_to_number)
}
/// `Number(v ?? d)`.
fn num_or(v: Option<&str>, d: f64) -> f64 {
    v.map_or(d, js::string_to_number)
}
/// `px(v)` = `Number(v ?? 0) / EMU`.
fn px(v: Option<&str>) -> f64 {
    num_or(v, 0.0) / EMU
}
/// `px(v ?? d)`.
fn px_or(v: Option<&str>, d: f64) -> f64 {
    num_or(v, d) / EMU
}
/// JS truthiness of a number.
fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}
/// `Math.max(a, b)` (NaN wins).
fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}
/// `Math.min(a, b)` (NaN wins).
fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn solid(color: ColorValue) -> Paint {
    Paint::Solid(SolidPaint { color, extra: Extra::new() })
}

fn hexs(s: &str) -> Result<ColorValue, ImportError> {
    hex(s).map_err(ImportError::invalid)
}

/// `{ kind, params }` of a preset geometry (`PRESETS`).
fn preset(prst: &str) -> Option<(&'static str, &'static [(&'static str, f64)])> {
    Some(match prst {
        "rect" => ("rect", &[]),
        "ellipse" => ("ellipse", &[]),
        "triangle" => ("triangle", &[]),
        "rtTriangle" => ("right-triangle", &[]),
        "diamond" => ("diamond", &[]),
        "pentagon" => ("pentagon", &[]),
        "hexagon" => ("hexagon", &[]),
        "octagon" => ("octagon", &[]),
        "heart" => ("heart", &[]),
        "rightArrow" => ("arrow", &[]),
        "leftRightArrow" => ("double-arrow", &[]),
        "chevron" => ("chevron", &[]),
        "plus" | "mathPlus" => ("plus", &[]),
        "cloud" => ("cloud", &[]),
        "cloudCallout" => ("thought-bubble", &[]),
        "parallelogram" => ("parallelogram", &[]),
        "trapezoid" => ("trapezoid", &[]),
        "donut" => ("ring", &[]),
        "moon" => ("moon", &[]),
        "lightningBolt" => ("lightning", &[]),
        "wedgeRectCallout" => ("speech", &[]),
        "wedgeRoundRectCallout" => ("callout-rect", &[]),
        "teardrop" => ("drop", &[]),
        "irregularSeal1" | "irregularSeal2" => ("burst", &[]),
        "blockArc" => ("block-arc", &[]),
        "flowChartProcess" => ("rect", &[]),
        "flowChartDecision" => ("diamond", &[]),
        "mathMultiply" => ("cross", &[]),
        "star4" => ("star", &[("points", 4.0), ("innerRatio", 0.4)]),
        "star5" => ("star", &[("points", 5.0), ("innerRatio", 0.45)]),
        "star6" => ("star", &[("points", 6.0), ("innerRatio", 0.55)]),
        "star7" => ("star", &[("points", 7.0), ("innerRatio", 0.55)]),
        "star8" => ("star", &[("points", 8.0), ("innerRatio", 0.6)]),
        "star10" => ("star", &[("points", 10.0), ("innerRatio", 0.65)]),
        "star12" => ("star", &[("points", 12.0), ("innerRatio", 0.7)]),
        "star16" | "star24" | "star32" => ("seal", &[]),
        "ribbon2" => ("ribbon", &[]),
        "flowChartConnector" => ("ellipse", &[]),
        "snip1Rect" => ("snip-corner", &[]),
        "snip2SameRect" => ("snip-two", &[]),
        "round1Rect" => ("round-one", &[]),
        "foldedCorner" => ("folded-corner", &[]),
        "plaque" => ("plaque", &[]),
        "frame" => ("frame", &[]),
        "can" => ("cylinder", &[]),
        "cube" => ("cube", &[]),
        "pie" => ("pie", &[]),
        "sun" => ("sun", &[]),
        "wave" | "doubleWave" => ("wave", &[]),
        "gear6" => ("gear", &[("teeth", 6.0)]),
        "gear9" => ("gear", &[]),
        "uturnArrow" => ("u-turn-arrow", &[]),
        "circularArrow" => ("circular-arrow", &[]),
        "curvedRightArrow" => ("curved-arrow", &[]),
        "wedgeEllipseCallout" => ("callout-round", &[]),
        "flowChartTerminator" => ("pill", &[]),
        "ribbon" => ("banner", &[]),
        _ => return None,
    })
}

fn head(t: Option<&str>) -> LineMarker {
    match t.unwrap_or("") {
        "triangle" => LineMarker::Triangle,
        "arrow" => LineMarker::Arrow,
        "stealth" => LineMarker::Stealth,
        "oval" => LineMarker::Circle,
        "diamond" => LineMarker::Diamond,
        _ => LineMarker::None,
    }
}

fn prst_color(v: &str) -> &'static str {
    match v {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "gray" => "808080",
        "orange" => "FFA500",
        "purple" => "800080",
        _ => "000000",
    }
}

#[derive(Debug, Clone, Copy)]
struct Xfrm {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_x: bool,
    flip_y: bool,
}

fn read_xfrm(n: Option<&Node>) -> Option<Xfrm> {
    let n = n?;
    let off = n.child("a:off")?;
    let ext = n.child("a:ext")?;
    Some(Xfrm {
        x: px(off.attr("x")),
        y: px(off.attr("y")),
        w: px(ext.attr("cx")),
        h: px(ext.attr("cy")),
        rot: num_or(n.attr("rot"), 0.0) / 60000.0,
        flip_x: n.attr("flipH") == Some("1"),
        flip_y: n.attr("flipV") == Some("1"),
    })
}

/// A group's child-space → parent-space mapping.
#[derive(Debug, Clone, Copy)]
struct GroupTx {
    off_x: f64,
    off_y: f64,
    ch_x: f64,
    ch_y: f64,
    sx: f64,
    sy: f64,
}

/// Apply nested group transforms (innermost first, like the composed TS closures).
fn apply_tx(stack: &[GroupTx], r: Xfrm) -> Xfrm {
    stack.iter().rev().fold(r, |r, g| Xfrm { x: g.off_x + (r.x - g.ch_x) * g.sx, y: g.off_y + (r.y - g.ch_y) * g.sy, w: r.w * g.sx, h: r.h * g.sy, ..r })
}

/// `base(r)`.
fn base_xf(r: &Xfrm) -> Xf {
    let mut xf = Xf::new(r.x, r.y, jmax(1.0, r.w), jmax(1.0, r.h), r.rot);
    if r.flip_x {
        xf.flip_x = Some(true);
    }
    if r.flip_y {
        xf.flip_y = Some(true);
    }
    xf
}

fn normalize_path(dir: &str, target: &str) -> String {
    if let Some(t) = target.strip_prefix('/') {
        return t.to_owned();
    }
    let joined = format!("{dir}/{target}");
    let mut out: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        if p == ".." {
            out.pop();
        } else if !p.is_empty() && p != "." {
            out.push(p);
        }
    }
    out.join("/")
}

/// `part.slice(0, part.lastIndexOf('/'))` (JS `slice(0, -1)` when there is no slash).
fn dir_of(part: &str) -> &str {
    match part.rfind('/') {
        Some(i) => &part[..i],
        None => {
            let mut end = part.len();
            if let Some((i, _)) = part.char_indices().last() {
                end = i;
            }
            &part[..end]
        }
    }
}

type Rels = IndexMap<String, String>;

struct Pkg<'z> {
    zip: zip::ZipArchive<Cursor<&'z [u8]>>,
}

impl Pkg<'_> {
    fn bytes(&mut self, p: &str) -> Option<Vec<u8>> {
        self.bytes_limited(p, MAX_PART_BYTES)
    }
    fn bytes_limited(&mut self, p: &str, limit: u64) -> Option<Vec<u8>> {
        let f = self.zip.by_name(p).ok()?;
        if f.is_dir() {
            return None;
        }
        read_limited(f, limit).ok()
    }
    fn read(&mut self, p: &str) -> Option<String> {
        self.bytes_limited(p, MAX_XML_PART_BYTES).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    fn parse(&mut self, p: &str) -> Result<Option<Vec<xml::Child>>, ImportError> {
        match self.read(p) {
            Some(s) => xml::parse(&s).map(Some).map_err(ImportError::invalid),
            None => Ok(None),
        }
    }
    fn rels_of(&mut self, part: &str) -> Result<Rels, ImportError> {
        let dir = dir_of(part);
        let name = part.rfind('/').map_or(part, |i| &part[i + 1..]);
        let file = format!("{dir}/_rels/{name}.rels");
        let mut m = Rels::new();
        let Some(doc) = self.parse(&file)? else { return Ok(m) };
        if let Some(root) = root_of(&doc, "Relationships") {
            for rel in root.els() {
                let target = rel.attr("Target").unwrap_or("");
                let id = rel.attr("Id").unwrap_or("").to_owned();
                if rel.attr("TargetMode") == Some("External") {
                    m.insert(id, format!("external:{target}"));
                } else {
                    m.insert(id, normalize_path(dir, target));
                }
            }
        }
        Ok(m)
    }
}

struct Importer<'a, 'z, 'm> {
    pkg: Pkg<'z>,
    font_available: &'a dyn Fn(&str) -> bool,
    media_fn: &'a mut MediaFn<'m>,
    media_cache: HashMap<String, Option<String>>,
    scheme: HashMap<String, String>,
}

/// Fill of an `spPr`-like node: `None` = no fill given, `Some(None)` = `a:noFill`.
type FillResult = Option<Option<Paint>>;

impl Importer<'_, '_, '_> {
    fn media(&mut self, part_path: &str) -> Option<String> {
        if let Some(hit) = self.media_cache.get(part_path) {
            return hit.clone();
        }
        let a = match self.pkg.bytes(part_path) {
            Some(b) => {
                let name = part_path.rsplit('/').next().unwrap_or(part_path);
                (self.media_fn)(&b, name)
            }
            None => None,
        };
        self.media_cache.insert(part_path.to_owned(), a.clone());
        a
    }

    fn color(&self, n: Option<&Node>) -> Result<Option<ColorValue>, ImportError> {
        let Some(n) = n else { return Ok(None) };
        let Some(c) = n.child("a:srgbClr").or_else(|| n.child("a:schemeClr")).or_else(|| n.child("a:prstClr")).or_else(|| n.child("a:sysClr")) else {
            return Ok(None);
        };
        let h: Option<String> = match c.tag.as_str() {
            "a:srgbClr" => c.attr("val").map(str::to_owned),
            "a:schemeClr" => self.scheme.get(c.attr("val").unwrap_or("")).cloned(),
            "a:sysClr" => c.attr("lastClr").map(str::to_owned),
            _ => Some(prst_color(c.attr("val").unwrap_or("")).to_owned()),
        };
        // `if (!h) h = '000000'`
        let h = h.filter(|s| !s.is_empty()).unwrap_or_else(|| "000000".into());
        let ColorValue::Srgb(v0) = hexs(&format!("#{h}"))? else { unreachable!("hex() answers sRGB") };
        let (mut r, mut g, mut b, mut a) = (v0.r, v0.g, v0.b, v0.a);
        for m in c.els() {
            let val = num_or(m.attr("val"), 0.0) / 100000.0;
            match m.tag.as_str() {
                "a:lumMod" | "a:shade" => {
                    r *= val;
                    g *= val;
                    b *= val;
                }
                "a:lumOff" => {
                    r = jmin(1.0, r + val);
                    g = jmin(1.0, g + val);
                    b = jmin(1.0, b + val);
                }
                "a:tint" => {
                    r += (1.0 - r) * (1.0 - val);
                    g += (1.0 - g) * (1.0 - val);
                    b += (1.0 - b) * (1.0 - val);
                }
                "a:alpha" => a = val,
                _ => {}
            }
        }
        Ok(Some(ColorValue::srgb(r, g, b, a)))
    }

    fn fill(&self, sp_pr: Option<&Node>) -> Result<FillResult, ImportError> {
        let Some(sp_pr) = sp_pr else { return Ok(None) };
        if sp_pr.child("a:noFill").is_some() {
            return Ok(Some(None));
        }
        if let Some(sf) = sp_pr.child("a:solidFill") {
            return Ok(self.color(Some(sf))?.map(|c| Some(solid(c))));
        }
        if let Some(gf) = sp_pr.child("a:gradFill") {
            let mut stops = Vec::new();
            for gs in gf.child("a:gsLst").children_vec("a:gs") {
                let t = num_or(gs.attr("pos"), 0.0) / 100000.0;
                let color = match self.color(Some(gs))? {
                    Some(c) => c,
                    None => hexs("#000000")?,
                };
                stops.push(GradientStop { t, color, extra: Extra::new() });
            }
            stops.sort_by(|a, b| (a.t - b.t).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
            if stops.is_empty() {
                return Ok(None);
            }
            if gf.child("a:path").is_some() {
                return Ok(Some(Some(Paint::Radial(RadialPaint { stops, extra: Extra::new() }))));
            }
            let angle = num_or(gf.child("a:lin").and_then(|l| l.attr("ang")), 0.0) / 60000.0;
            return Ok(Some(Some(Paint::Linear(LinearPaint { stops, angle, extra: Extra::new() }))));
        }
        Ok(None)
    }

    /// Placeholder geometry defaults from the layout and its master (`phInfo`).
    fn ph_info(&mut self, layout_part: Option<&str>) -> Result<(HashMap<String, Xfrm>, Option<String>), ImportError> {
        let mut map = HashMap::new();
        let mut collect = |pkg: &mut Pkg<'_>, part: Option<&str>| -> Result<(), ImportError> {
            let Some(part) = part else { return Ok(()) };
            let Some(doc) = pkg.parse(part)? else { return Ok(()) };
            let root = doc.iter().find_map(|c| match c {
                xml::Child::El(n) if n.tag == "p:sldLayout" || n.tag == "p:sldMaster" => Some(n),
                _ => None,
            });
            for sp in root.path(&["p:cSld", "p:spTree"]).children_vec("p:sp") {
                let ph = Some(sp).path(&["p:nvSpPr", "p:nvPr", "p:ph"]);
                let x = read_xfrm(Some(sp).path(&["p:spPr", "a:xfrm"]));
                let (Some(ph), Some(x)) = (ph, x) else { continue };
                let idx = ph.attr("idx").filter(|s| !s.is_empty()).map(|i| format!("idx:{i}"));
                let ty = format!("type:{}", ph.attr("type").unwrap_or("body"));
                for k in idx.into_iter().chain([ty]) {
                    map.entry(k).or_insert(x);
                }
            }
            Ok(())
        };
        collect(&mut self.pkg, layout_part)?;
        let layout_rels = match layout_part {
            Some(l) => self.pkg.rels_of(l)?,
            None => Rels::new(),
        };
        let master = layout_rels.values().find(|t| t.contains("slideMaster")).cloned();
        collect(&mut self.pkg, master.as_deref())?;
        Ok((map, master))
    }

    fn bg_of(&mut self, part: Option<&str>, rels: &Rels) -> Result<Option<Paint>, ImportError> {
        let Some(part) = part else { return Ok(None) };
        let Some(doc) = self.pkg.parse(part)? else { return Ok(None) };
        let root = doc.iter().find_map(|c| match c {
            xml::Child::El(n) if matches!(n.tag.as_str(), "p:sld" | "p:sldLayout" | "p:sldMaster") => Some(n),
            _ => None,
        });
        let Some(bg_pr) = root.path(&["p:cSld", "p:bg", "p:bgPr"]) else { return Ok(None) };
        if let Some(blip) = Some(bg_pr).path(&["a:blipFill", "a:blip"]) {
            let target = rels.get(blip.attr("r:embed").unwrap_or("")).cloned().unwrap_or_default();
            return Ok(self.media(&target).map(|asset| Paint::Image(ImagePaint { asset, fit: ImageFit::Fill, crop: None, extra: Extra::new() })));
        }
        Ok(self.fill(Some(bg_pr))?.flatten())
    }
}

/// Per-slide state.
struct Slide<'r> {
    rels: &'r Rels,
    ph_map: &'r HashMap<String, Xfrm>,
    els: Vec<Element>,
}

fn default_pt(ph_type: Option<&str>) -> f64 {
    match ph_type {
        Some("title" | "ctrTitle") => 40.0,
        Some("subTitle") => 24.0,
        _ => 18.0,
    }
}

fn default_run(size: f64) -> RunStyle {
    RunStyle {
        family: "Inter".into(),
        weight: 400.0,
        italic: false,
        size,
        fill: solid(hex("#1f2328").expect("literal")),
        tracking: 0.0,
        underline: None,
        strike: None,
        case_transform: None,
        highlight: None,
        baseline: None,
        extra: Extra::new(),
    }
}

impl Importer<'_, '_, '_> {
    fn text_body(&self, tx_body: Option<&Node>, ph_type: Option<&str>) -> Result<Option<TextBody>, ImportError> {
        let Some(tx_body) = tx_body else { return Ok(None) };
        let body_pr = tx_body.child("a:bodyPr");
        let scale = num_or(body_pr.child("a:normAutofit").attr("fontScale"), 100000.0) / 100000.0;
        let dpt = default_pt(ph_type);
        let mut paragraphs: Vec<Paragraph> = Vec::new();
        let mut def: Option<RunStyle> = None;
        for p in tx_body.children("a:p") {
            let p_pr = p.child("a:pPr");
            let algn = p_pr.attr("algn");
            let mut runs: Vec<Option<Run>> = Vec::new(); // None = line break
            for r in p.els() {
                let t = r.tag.as_str();
                if t != "a:r" && t != "a:br" && t != "a:fld" {
                    continue;
                }
                if t == "a:br" {
                    runs.push(None);
                    continue;
                }
                let r_pr = r.child("a:rPr");
                let pt = num_or(r_pr.attr("sz"), dpt * 100.0) / 100.0;
                let c = self.color(r_pr.child("a:solidFill"))?;
                let u = r_pr.attr("u");
                let strike = r_pr.attr("strike");
                let style = RunStyleOverride {
                    family: Some(map_font(r_pr.child("a:latin").attr("typeface"), self.font_available)),
                    size: Some(jmax(1.0, (pt / 0.75) * scale)),
                    weight: Some(if r_pr.attr("b") == Some("1") { 700.0 } else { 400.0 }),
                    italic: Some(r_pr.attr("i") == Some("1")),
                    fill: c.map(solid),
                    underline: (u.is_some_and(|u| !u.is_empty() && u != "none")).then_some(true),
                    strike: (strike.is_some_and(|s| !s.is_empty() && s != "noStrike")).then_some(true),
                    case_transform: (r_pr.attr("cap") == Some("all")).then_some(CaseTransform::Upper),
                    ..Default::default()
                };
                let text = r.child("a:t").text();
                if !text.is_empty() {
                    runs.push(Some(Run { text, style, link: None, extra: Extra::new() }));
                }
            }
            // Line breaks inside a paragraph become separate paragraphs (our model has no soft breaks).
            let mut segments: Vec<Vec<Run>> = vec![vec![]];
            for r in runs {
                match r {
                    None => segments.push(vec![]),
                    Some(r) => segments.last_mut().expect("segment").push(r),
                }
            }
            let ln_pct = num_or(p_pr.path(&["a:lnSpc", "a:spcPct"]).attr("val"), 0.0);
            let after = num_or(p_pr.path(&["a:spcAft", "a:spcPts"]).attr("val"), 0.0) / 100.0 / 0.75;
            let list = if p_pr.child("a:buChar").is_some() {
                Some(ListSpec { kind: ListKind::Bullet, level: num_or(p_pr.attr("lvl"), 0.0), extra: Extra::new() })
            } else if p_pr.child("a:buAutoNum").is_some() {
                Some(ListSpec { kind: ListKind::Number, level: num_or(p_pr.attr("lvl"), 0.0), extra: Extra::new() })
            } else {
                None
            };
            let align = match algn {
                Some("ctr") => TextAlign::Center,
                Some("r") => TextAlign::End,
                Some("just") => TextAlign::Justify,
                _ => TextAlign::Start,
            };
            for seg in segments {
                if def.is_none()
                    && let Some(first) = seg.first()
                {
                    def = Some(apply_override(&default_run((dpt / 0.75) * scale), &first.style));
                }
                paragraphs.push(Paragraph {
                    runs: if seg.is_empty() { vec![Run::plain("")] } else { seg },
                    align: align.clone(),
                    line_height: if truthy(ln_pct) { jmax(0.6, (ln_pct / 100000.0) * 1.2) } else { 1.2 },
                    space_after: after,
                    list: list.clone(),
                    extra: Extra::new(),
                });
            }
        }
        if !paragraphs.iter().any(|p| p.runs.iter().any(|r| !js::trim(&r.text).is_empty())) {
            return Ok(None);
        }
        Ok(Some(TextBody { paragraphs, default_run: def.unwrap_or_else(|| default_run(dpt / 0.75)), extra: Extra::new() }))
    }

    fn walk(&mut self, s: &mut Slide<'_>, tree: Option<&Node>, tx: &[GroupTx]) -> Result<(), ImportError> {
        let Some(tree) = tree else { return Ok(()) };
        for n in tree.els() {
            match n.tag.as_str() {
                "p:grpSp" => {
                    let gx = Some(n).path(&["p:grpSpPr", "a:xfrm"]);
                    let off = gx.child("a:off");
                    let ext = gx.child("a:ext");
                    let ch_off = gx.child("a:chOff");
                    let ch_ext = gx.child("a:chExt");
                    let sx = if ch_ext.is_some() && truthy(num(ch_ext.attr("cx"))) { num(ext.attr("cx")) / num(ch_ext.attr("cx")) } else { 1.0 };
                    let sy = if ch_ext.is_some() && truthy(num(ch_ext.attr("cy"))) { num(ext.attr("cy")) / num(ch_ext.attr("cy")) } else { 1.0 };
                    let g = GroupTx { off_x: px(off.attr("x")), off_y: px(off.attr("y")), ch_x: px(ch_off.attr("x")), ch_y: px(ch_off.attr("y")), sx, sy };
                    let mut inner = tx.to_vec();
                    inner.push(g);
                    self.walk(s, Some(n), &inner)?;
                }
                "p:sp" => self.shape(s, n, tx)?,
                "p:pic" => self.picture(s, n, tx),
                "p:cxnSp" => self.connector(s, n, tx)?,
                "p:graphicFrame" => self.frame(s, n, tx)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn shape(&mut self, s: &mut Slide<'_>, n: &Node, tx: &[GroupTx]) -> Result<(), ImportError> {
        let ph = Some(n).path(&["p:nvSpPr", "p:nvPr", "p:ph"]);
        let ph_type: Option<&str> = ph.map(|p| p.attr("type").unwrap_or("body"));
        let sp_pr = n.child("p:spPr");
        let mut r = read_xfrm(sp_pr.child("a:xfrm"));
        if r.is_none()
            && let Some(ph) = ph
        {
            // `phMap.get(`idx:${attr(ph, 'idx')}`)`: an absent idx looks up "idx:undefined", never stored.
            let idx_key = format!("idx:{}", ph.attr("idx").unwrap_or("undefined"));
            r = s.ph_map.get(&idx_key).or_else(|| s.ph_map.get(&format!("type:{}", ph_type.unwrap_or("body")))).copied();
        }
        let Some(r) = r else { return Ok(()) };
        let r = apply_tx(tx, r);
        let cnv = Some(n).path(&["p:nvSpPr", "p:cNvPr"]);
        let name = cnv.attr("name").filter(|s| !s.is_empty());
        let descr = cnv.attr("descr").filter(|s| !s.is_empty());
        let prst = sp_pr.child("a:prstGeom").attr("prst");
        let f = self.fill(sp_pr)?;
        let ln = sp_pr.child("a:ln");
        let ln_fill = match ln {
            Some(l) if l.child("a:noFill").is_none() => self.color(l.child("a:solidFill"))?,
            _ => None,
        };
        let ln_w = if ln.is_some() { px_or(ln.attr("w"), 12700.0) } else { 0.0 };
        let fill_paint: Option<Paint> = f.clone().flatten();
        let has_fill = fill_paint.is_some();
        if prst == Some("line") {
            let mut l = make_line(base_init(base_xf(&r)), LineOpts { color: Some(hexs("#000000")?), width: Some(jmax(1.0, ln_w)), ..Default::default() });
            if let Some(c) = ln_fill {
                l.stroke.paint = solid(c);
            }
            l.points = vec![[if r.flip_x { r.w } else { 0.0 }, if r.flip_y { r.h } else { 0.0 }], [if r.flip_x { 0.0 } else { r.w }, if r.flip_y { 0.0 } else { r.h }]];
            l.base.xf.flip_x = None;
            l.base.xf.flip_y = None;
            s.els.push(Element::Line(l));
            return Ok(());
        }
        if has_fill || ln_fill.is_some() {
            let cust = sp_pr.child("a:custGeom");
            let (kind, pparams) = prst.and_then(preset).unwrap_or(("rect", &[]));
            let mut params: IndexMap<String, f64> = pparams.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
            if matches!(prst, Some("roundRect" | "flowChartAlternateProcess")) {
                let fmla = sp_pr.path(&["a:prstGeom", "a:avLst"]).child("a:gd").attr("fmla");
                let adj = fmla.map_or(16667.0, |f| js::string_to_number(&f.replacen("val ", "", 1)));
                params.insert("cornerRadius".into(), (adj / 100000.0) * jmin(r.w, r.h));
            }
            if prst == Some("flowChartTerminator") {
                params.insert("cornerRadius".into(), jmin(r.w, r.h) / 2.0);
            }
            let d = cust.and_then(custom);
            let mut init = base_init(base_xf(&r));
            if let Some(nm) = name {
                init.name = Some(nm.to_owned());
            }
            let mut sh = make_shape(
                init,
                ShapeOpts {
                    kind: Some(if d.is_some() { ShapeKind::Custom } else { ShapeKind::parse(kind) }),
                    fill: Some(match &fill_paint {
                        Some(p) => p.clone(),
                        None => solid(hexs("#00000000")?),
                    }),
                    params: Some(params),
                    ..Default::default()
                },
            );
            if d.is_some() {
                sh.d = d;
            }
            if !has_fill {
                sh.fills = vec![];
            }
            if let Some(c) = &ln_fill
                && ln_w > 0.0
            {
                let dash = ln.child("a:prstDash").attr("val").filter(|v| !v.is_empty() && *v != "solid").map(|_| vec![3.0, 2.0]);
                sh.strokes = vec![Stroke { paint: solid(c.clone()), width: ln_w, align: StrokeAlign::Center, cap: StrokeCap::Butt, join: StrokeJoin::Miter, dash, extra: Extra::new() }];
            }
            if let Some(d) = descr {
                sh.base.alt_text = Some(d.to_owned());
            }
            if let Some(shd) = sp_pr.path(&["a:effectLst", "a:outerShdw"]) {
                let dist = px_or(shd.attr("dist"), 0.0);
                let dir = (num_or(shd.attr("dir"), 0.0) / 60000.0) * (std::f64::consts::PI / 180.0);
                let color = match self.color(Some(shd))? {
                    Some(c) => c,
                    None => hexs("#00000040")?,
                };
                sh.base.effects = Some(vec![Effect::DropShadow(DropShadowEffect {
                    dx: dir.cos() * dist,
                    dy: dir.sin() * dist,
                    blur: px_or(shd.attr("blurRad"), 0.0),
                    color,
                    extra: Extra::new(),
                })]);
            }
            s.els.push(Element::Shape(sh));
        }
        if let Some(body) = self.text_body(n.child("p:txBody"), ph_type)? {
            let body_pr = Some(n).path(&["p:txBody", "a:bodyPr"]);
            let l = px_or(body_pr.attr("lIns"), 91440.0);
            let t = px_or(body_pr.attr("tIns"), 45720.0);
            let rr = px_or(body_pr.attr("rIns"), 91440.0);
            let b = px_or(body_pr.attr("bIns"), 45720.0);
            let est = body.paragraphs.iter().fold(0.0, |sum, p| {
                let mx = p.runs.iter().map(|x| x.style.size.unwrap_or(body.default_run.size)).fold(f64::NEG_INFINITY, jmax);
                sum + mx * p.line_height + p.space_after
            });
            let anchor = body_pr.attr("anchor");
            let inner_h = jmax(1.0, r.h - t - b);
            let y = match anchor {
                Some("ctr") => r.y + t + jmax(0.0, (inner_h - est) / 2.0),
                Some("b") => r.y + t + jmax(0.0, inner_h - est),
                _ => r.y + t,
            };
            let mut te = make_text(base_init(Xf::new(r.x + l, y, jmax(10.0, r.w - l - rr), jmax(10.0, est), r.rot)), TextOpts { body: Some(body), ..Default::default() });
            if matches!(ph_type, Some("title" | "ctrTitle")) {
                te.base.role = Some(SemanticRole::Headline);
            }
            // WordArt arches → curved text.
            match body_pr.child("a:prstTxWarp").attr("prst") {
                Some("textArchUp" | "textCircle") => te.curve = Some(50.0),
                Some("textArchDown") => te.curve = Some(-50.0),
                _ => {}
            }
            if body_pr.attr("vert") == Some("vert") {
                te.base.xf.rot = 90.0;
            }
            s.els.push(Element::Text(te));
        }
        Ok(())
    }

    fn picture(&mut self, s: &mut Slide<'_>, n: &Node, tx: &[GroupTx]) {
        let sp_pr = n.child("p:spPr");
        let Some(x) = read_xfrm(sp_pr.child("a:xfrm")) else { return };
        let r = apply_tx(tx, x);
        let blip = Some(n).path(&["p:blipFill", "a:blip"]);
        let Some(target) = s.rels.get(blip.attr("r:embed").unwrap_or("")) else { return };
        if target.is_empty() || target.starts_with("external:") {
            return;
        }
        let Some(a) = self.media(target) else { return };
        let src = Some(n).path(&["p:blipFill", "a:srcRect"]);
        let cl = num_or(src.attr("l"), 0.0) / 100000.0;
        let ct = num_or(src.attr("t"), 0.0) / 100000.0;
        let cr = num_or(src.attr("r"), 0.0) / 100000.0;
        let cb = num_or(src.attr("b"), 0.0) / 100000.0;
        let mut img = make_image(base_init(base_xf(&r)), &a);
        img.crop = StoredRect {
            x: jmax(0.0, cl),
            y: jmax(0.0, ct),
            w: jmin(1.0, jmax(0.01, 1.0 - cl - cr)),
            h: jmin(1.0, jmax(0.01, 1.0 - ct - cb)),
            extra: Extra::new(),
        };
        if let Some(d) = Some(n).path(&["p:nvPicPr", "p:cNvPr"]).attr("descr").filter(|d| !d.is_empty()) {
            img.base.alt_text = Some(d.to_owned());
        }
        let geom = sp_pr.child("a:prstGeom").attr("prst");
        if geom == Some("roundRect") {
            img.corner_radius = Some(jmin(r.w, r.h) * 0.16);
        }
        if geom == Some("ellipse") {
            img.corner_radius = Some(jmin(r.w, r.h) / 2.0);
        }
        s.els.push(Element::Image(img));
    }

    fn connector(&mut self, s: &mut Slide<'_>, n: &Node, tx: &[GroupTx]) -> Result<(), ImportError> {
        let sp_pr = n.child("p:spPr");
        let Some(x) = read_xfrm(sp_pr.child("a:xfrm")) else { return Ok(()) };
        let r = apply_tx(tx, x);
        let ln = sp_pr.child("a:ln");
        let c = match self.color(ln.child("a:solidFill"))? {
            Some(c) => c,
            None => hexs("#1f2328")?,
        };
        let mut l = make_line(
            base_init(Xf::new(r.x, r.y, jmax(1.0, r.w), jmax(1.0, r.h), r.rot)),
            LineOpts { color: Some(hexs("#000000")?), width: Some(jmax(1.0, px_or(ln.attr("w"), 12700.0))), ..Default::default() },
        );
        l.stroke.paint = solid(c);
        l.points = vec![[if r.flip_x { r.w } else { 0.0 }, if r.flip_y { r.h } else { 0.0 }], [if r.flip_x { 0.0 } else { r.w }, if r.flip_y { 0.0 } else { r.h }]];
        l.start_marker = head(ln.child("a:headEnd").attr("type"));
        l.end_marker = head(ln.child("a:tailEnd").attr("type"));
        s.els.push(Element::Line(l));
        Ok(())
    }

    fn frame(&mut self, s: &mut Slide<'_>, n: &Node, tx: &[GroupTx]) -> Result<(), ImportError> {
        let Some(x) = read_xfrm(n.child("p:xfrm")) else { return Ok(()) };
        let r = apply_tx(tx, x);
        let data = Some(n).path(&["a:graphic", "a:graphicData"]);
        if let Some(tbl) = data.child("a:tbl") {
            let cols: Vec<f64> = tbl.child("a:tblGrid").children_vec("a:gridCol").iter().map(|c| px(c.attr("w"))).collect();
            let rows: Vec<&Node> = tbl.children("a:tr").collect();
            let grid: Vec<Vec<String>> = rows
                .iter()
                .map(|tr| {
                    tr.children("a:tc")
                        .map(|tc| {
                            tc.child("a:txBody")
                                .children_vec("a:p")
                                .iter()
                                .map(|p| p.children("a:r").map(|rr| rr.child("a:t").text()).collect::<String>())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .collect()
                })
                .collect();
            let mut t = make_table(base_init(base_xf(&r)), TableOpts { data: Some(grid), ..Default::default() });
            let tw = cols.iter().fold(0.0, |a, b| a + b);
            let tw = if truthy(tw) { tw } else { 1.0 };
            t.cols = cols.iter().map(|c| c / tw).collect();
            let rh: Vec<f64> = rows.iter().map(|tr| px(tr.attr("h"))).collect();
            let th = rh.iter().fold(0.0, |a, b| a + b);
            let th = if truthy(th) { th } else { 1.0 };
            t.rows = rh.iter().map(|h| h / th).collect();
            // Merged cells (gridSpan/rowSpan on the anchor; hMerge/vMerge cells are covered and carry nothing).
            for (ri, tr) in rows.iter().enumerate() {
                for (ci, tc) in tr.children("a:tc").enumerate() {
                    let gs = num_or(tc.attr("gridSpan"), 1.0);
                    let rs = num_or(tc.attr("rowSpan"), 1.0);
                    let k = format!("{ri}:{ci}");
                    if tc.attr("hMerge") == Some("1") || tc.attr("vMerge") == Some("1") {
                        t.cells.shift_remove(&k);
                    } else if gs > 1.0 || rs > 1.0 {
                        let span = CellSpan { rows: jmax(1.0, rs), cols: jmax(1.0, gs), extra: Extra::new() };
                        match t.cells.get_mut(&k) {
                            Some(c) => c.span = Some(span),
                            None => {
                                let mut c = TableCell::text("");
                                c.span = Some(span);
                                t.cells.insert(k, c);
                            }
                        }
                    }
                }
            }
            let first_run = rows.first().copied().path(&["a:tc", "a:txBody", "a:p", "a:r", "a:rPr"]);
            if let Some(sz) = first_run.attr("sz").filter(|s| !s.is_empty()) {
                t.style.size = js::string_to_number(sz) / 100.0 / 0.75;
            }
            s.els.push(Element::Table(t));
            return Ok(());
        }
        if let Some(chart_ref) = data.child("c:chart") {
            let part = s.rels.get(chart_ref.attr("r:id").unwrap_or("")).cloned();
            let cx = match part.filter(|p| !p.is_empty()) {
                Some(p) => self.pkg.read(&p),
                None => None,
            };
            let chart = match cx {
                Some(x) => parse_chart(&x)?,
                None => None,
            };
            if let Some((kind, labels, series)) = chart {
                s.els.push(Element::Chart(make_chart(
                    base_init(base_xf(&r)),
                    ChartOpts {
                        kind: Some(kind),
                        labels: Some(labels),
                        series: Some(series.into_iter().map(|(name, values)| SeriesInit { name, values, color: None }).collect()),
                    },
                )));
            }
        }
        Ok(())
    }
}

/// Custom geometry → a unit-box SVG path.
fn custom(geom: &Node) -> Option<String> {
    let p = geom.child("a:pathLst").child("a:path")?;
    let pw = num_or(p.attr("w"), 1.0);
    let pw = if truthy(pw) { pw } else { 1.0 };
    let ph = num_or(p.attr("h"), 1.0);
    let ph = if truthy(ph) { ph } else { 1.0 };
    // `pt(undefined)` (a moveTo without a point) formats NaN, like the TS.
    let pt = |n: Option<&Node>| format!("{} {}", js::to_fixed(num(n.attr("x")) / pw, 4), js::to_fixed(num(n.attr("y")) / ph, 4));
    let mut out: Vec<String> = Vec::new();
    for c in p.els() {
        let pts: Vec<&Node> = c.children("a:pt").collect();
        let all = || pts.iter().map(|n| pt(Some(n))).collect::<Vec<_>>().join(" ");
        match c.tag.as_str() {
            "a:moveTo" => out.push(format!("M{}", pt(pts.first().copied()))),
            "a:lnTo" => out.push(format!("L{}", pt(pts.first().copied()))),
            "a:cubicBezTo" => out.push(format!("C{}", all())),
            "a:quadBezTo" => out.push(format!("Q{}", all())),
            "a:close" => out.push("Z".into()),
            _ => {}
        }
    }
    if out.is_empty() { None } else { Some(out.join(" ")) }
}

type Chart = (ChartKind, Vec<String>, Vec<(String, Vec<f64>)>);

/// Bar/column/line/area/pie/doughnut charts → our chart element (`parseChart`).
fn parse_chart(xml_s: &str) -> Result<Option<Chart>, ImportError> {
    let doc = xml::parse(xml_s).map_err(ImportError::invalid)?;
    let root = root_of(&doc, "c:chartSpace");
    let plot = root.path(&["c:chart", "c:plotArea"]);
    const TYPES: &[&str] = &["c:barChart", "c:lineChart", "c:areaChart", "c:pieChart", "c:doughnutChart", "c:pie3DChart", "c:bar3DChart", "c:line3DChart"];
    let Some(type_node) = plot.and_then(|p| p.els().find(|n| TYPES.contains(&n.tag.as_str()))) else { return Ok(None) };
    let t = type_node.tag.as_str();
    let kind = if t.starts_with("c:bar") {
        if type_node.child("c:barDir").attr("val") == Some("bar") { ChartKind::Bar } else { ChartKind::Column }
    } else if t.starts_with("c:line") {
        ChartKind::Line
    } else if t == "c:areaChart" {
        ChartKind::Area
    } else if t == "c:doughnutChart" {
        ChartKind::Donut
    } else {
        ChartKind::Pie
    };
    let pts = |n: Option<&Node>| -> Vec<String> {
        let cache = if n.child("c:strRef").is_some() {
            n.path(&["c:strRef", "c:strCache"])
        } else if n.child("c:numRef").is_some() {
            n.path(&["c:numRef", "c:numCache"])
        } else {
            n.child("c:strLit").or_else(|| n.child("c:numLit"))
        };
        let mut v = cache.children_vec("c:pt");
        v.sort_by(|a, b| (num(a.attr("idx")) - num(b.attr("idx"))).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
        v.iter().map(|p| p.child("c:v").text()).collect()
    };
    let mut series: Vec<(String, Vec<f64>)> = Vec::new();
    let mut labels0: Option<Vec<String>> = None;
    for s in type_node.children("c:ser") {
        let name = pts(s.child("c:tx")).into_iter().next().unwrap_or_else(|| Some(s).path(&["c:tx", "c:v"]).text());
        let values = pts(s.child("c:val")).iter().map(|v| js::string_to_number(v)).map(|v| if v.is_finite() { v } else { 0.0 }).collect();
        let labels = pts(s.child("c:cat"));
        labels0.get_or_insert(labels);
        series.push((name, values));
    }
    let Some(labels) = labels0 else { return Ok(None) };
    Ok(Some((kind, labels, series)))
}

/// `importPptx(bytes, opts)`.
pub fn import_pptx(bytes: &[u8], opts: &ImportOptions<'_>, media: &mut MediaFn<'_>) -> Result<DesignDoc, ImportError> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(ImportError::invalid)?;
    let mut im = Importer { pkg: Pkg { zip }, font_available: opts.font_available, media_fn: media, media_cache: HashMap::new(), scheme: HashMap::new() };

    let Some(pres_doc) = im.pkg.parse("ppt/presentation.xml")? else { return Err(ImportError::NotPresentation) };
    let pres = root_of(&pres_doc, "p:presentation");
    let sz = pres.child("p:sldSz");
    let w = js::round(px_or(sz.attr("cx"), 12192000.0));
    let h = js::round(px_or(sz.attr("cy"), 6858000.0));
    let pres_rels = im.pkg.rels_of("ppt/presentation.xml")?;
    let slide_parts: Vec<String> = pres
        .child("p:sldIdLst")
        .children_vec("p:sldId")
        .iter()
        .filter_map(|s| pres_rels.get(s.attr("r:id").unwrap_or("")).filter(|x| !x.is_empty()).cloned())
        .collect();

    // Theme colours (first theme).
    let theme_part = pres_rels.values().find(|t| t.contains("theme/")).cloned().unwrap_or_else(|| "ppt/theme/theme1.xml".into());
    if let Some(theme) = im.pkg.parse(&theme_part)? {
        let cs = root_of(&theme, "a:theme").path(&["a:themeElements", "a:clrScheme"]);
        for c in cs.into_iter().flat_map(Node::els) {
            let name = c.tag.replacen("a:", "", 1);
            let v = match c.child("a:srgbClr") {
                Some(s) => s.attr("val"),
                None => c.child("a:sysClr").attr("lastClr"),
            };
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                im.scheme.insert(name, v.to_owned());
            }
        }
        for (k, from, d) in [("tx1", "dk1", "000000"), ("bg1", "lt1", "FFFFFF"), ("tx2", "dk2", "44546A"), ("bg2", "lt2", "E7E6E6")] {
            let v = im.scheme.get(from).cloned().unwrap_or_else(|| d.into());
            im.scheme.insert(k.into(), v);
        }
    }

    let mut doc = empty_design(&MetaInit {
        w,
        h,
        pages: Some(slide_parts.len().max(1)),
        title: Some(opts.title.clone().unwrap_or_else(|| "Imported presentation".into())),
        ..Default::default()
    });
    let page_ids: Vec<String> = sorted_pages(&doc.pages).iter().map(|p| p.id.clone()).collect();

    for (si, part) in slide_parts.iter().enumerate() {
        let page_id = &page_ids[si];
        let Some(sld_doc) = im.pkg.parse(part)? else { continue };
        let sld = root_of(&sld_doc, "p:sld");
        let rels = im.pkg.rels_of(part)?;
        let layout_part = rels.values().find(|t| t.contains("slideLayout")).cloned();
        let (ph_map, master_part) = im.ph_info(layout_part.as_deref())?;
        let mut slide = Slide { rels: &rels, ph_map: &ph_map, els: Vec::new() };
        im.walk(&mut slide, sld.path(&["p:cSld", "p:spTree"]), &[])?;
        let mut els = std::mem::take(&mut slide.els);

        let mut bg = im.bg_of(Some(part), &rels)?;
        if bg.is_none()
            && let Some(lp) = &layout_part
        {
            let lr = im.pkg.rels_of(lp)?;
            bg = im.bg_of(Some(lp), &lr)?;
        }
        if bg.is_none()
            && let Some(mp) = &master_part
        {
            let mr = im.pkg.rels_of(mp)?;
            bg = im.bg_of(Some(mp), &mr)?;
        }
        // Speaker notes.
        let mut notes = String::new();
        if let Some(np) = rels.values().find(|t| t.contains("notesSlide")).cloned()
            && let Some(nx) = im.pkg.parse(&np)?
        {
            let root = root_of(&nx, "p:notes");
            for sp in root.path(&["p:cSld", "p:spTree"]).children_vec("p:sp") {
                if Some(sp).path(&["p:nvSpPr", "p:nvPr", "p:ph"]).attr("type") != Some("body") {
                    continue;
                }
                let text = sp
                    .child("p:txBody")
                    .children_vec("a:p")
                    .iter()
                    .map(|p| p.children("a:r").map(|r| r.child("a:t").text()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                notes = js::trim(&text).to_owned();
            }
        }
        let page = doc.pages.get_mut(page_id).expect("page");
        if let Some(bg) = bg {
            page.background = vec![bg];
        }
        if sld.attr("show") == Some("0") {
            page.hidden = Some(true);
        }
        if !notes.is_empty() {
            page.notes = Some(notes);
        }
        place_all_elements(page_id, &mut els, None);
        for e in els {
            let id = e.id().unwrap_or_default().to_owned();
            doc.elements.insert(id, e);
        }
    }
    Ok(doc)
}
