// SPDX-License-Identifier: Apache-2.0
//! SVG import as editable vectors (DS-100): every filled or stroked path of an SVG (shapes, arcs and transforms
//! resolved by usvg) becomes a path element — a `custom` shape with its fill (solid or gradient) and stroke —
//! placed where the SVG image showed it. Even-odd fills are resolved to nonzero outlines. Text without fonts,
//! embedded raster images, patterns, filters, clip paths and masks inside the SVG are not converted (a pattern
//! fill falls back to grey).

use oc_geometry::{PathNode, SubPath, VectorPath, even_odd_to_nonzero};
use resvg::usvg::{self, Node, Paint as UPaint, tiny_skia_path::PathSegment};
use serde_json::{Map, Value, json};

/// Where the SVG's own coordinates land: `x' = sx·x + tx`, `y' = sy·y + ty` (element pixel space).
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    pub sx: f64,
    pub sy: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Placement {
    /// An SVG drawn into a `w × h` box through a normalised source crop (`cx, cy, cw, ch` in 0..1), like an
    /// image element shows it.
    pub fn image_box(svg_w: f64, svg_h: f64, w: f64, h: f64, crop: (f64, f64, f64, f64)) -> Placement {
        let (cx, cy, cw, ch) = crop;
        let sx = w / (svg_w * cw.max(1e-9));
        let sy = h / (svg_h * ch.max(1e-9));
        Placement { sx, sy, tx: -cx * svg_w * sx, ty: -cy * svg_h * sy }
    }
}

fn color(c: usvg::Color, a: f64) -> Value {
    json!({ "space": "srgb", "r": f64::from(c.red) / 255.0, "g": f64::from(c.green) / 255.0, "b": f64::from(c.blue) / 255.0, "a": a })
}

type M = [f64; 6];

fn mat(t: usvg::Transform) -> M {
    [f64::from(t.sx), f64::from(t.ky), f64::from(t.kx), f64::from(t.sy), f64::from(t.tx), f64::from(t.ty)]
}

fn ap(m: &M, x: f64, y: f64) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

fn mul(a: &M, b: &M) -> M {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn to_vector(p: &usvg::tiny_skia_path::Path, m: &M) -> VectorPath {
    let mut out = VectorPath::default();
    let mut cur: Option<SubPath> = None;
    let mut last = [0.0, 0.0];
    let push = |out: &mut VectorPath, sp: Option<SubPath>| {
        if let Some(sp) = sp
            && sp.nodes.len() >= 2
        {
            out.subpaths.push(sp);
        }
    };
    for seg in p.segments() {
        match seg {
            PathSegment::MoveTo(a) => {
                push(&mut out, cur.take());
                last = ap(m, f64::from(a.x), f64::from(a.y));
                cur = Some(SubPath { nodes: vec![PathNode::corner(last)], closed: false });
            }
            PathSegment::LineTo(a) => {
                let q = ap(m, f64::from(a.x), f64::from(a.y));
                cur.get_or_insert_with(|| SubPath { nodes: vec![PathNode::corner(last)], closed: false }).nodes.push(PathNode::corner(q));
                last = q;
            }
            PathSegment::QuadTo(c, a) => {
                let c = ap(m, f64::from(c.x), f64::from(c.y));
                let q = ap(m, f64::from(a.x), f64::from(a.y));
                let sp = cur.get_or_insert_with(|| SubPath { nodes: vec![PathNode::corner(last)], closed: false });
                if let Some(n) = sp.nodes.last_mut() {
                    n.cout = Some([last[0] + (c[0] - last[0]) * 2.0 / 3.0, last[1] + (c[1] - last[1]) * 2.0 / 3.0]);
                }
                sp.nodes.push(PathNode { p: q, cin: Some([q[0] + (c[0] - q[0]) * 2.0 / 3.0, q[1] + (c[1] - q[1]) * 2.0 / 3.0]), cout: None, smooth: false });
                last = q;
            }
            PathSegment::CubicTo(c1, c2, a) => {
                let c1 = ap(m, f64::from(c1.x), f64::from(c1.y));
                let c2 = ap(m, f64::from(c2.x), f64::from(c2.y));
                let q = ap(m, f64::from(a.x), f64::from(a.y));
                let sp = cur.get_or_insert_with(|| SubPath { nodes: vec![PathNode::corner(last)], closed: false });
                if let Some(n) = sp.nodes.last_mut() {
                    n.cout = Some(c1);
                }
                sp.nodes.push(PathNode { p: q, cin: Some(c2), cout: None, smooth: false });
                last = q;
            }
            PathSegment::Close => {
                if let Some(mut sp) = cur.take() {
                    // A closing node on the first node merges into it.
                    if sp.nodes.len() > 2 {
                        let (f, l) = (sp.nodes[0].p, sp.nodes[sp.nodes.len() - 1].p);
                        if (f[0] - l[0]).abs() < 1e-9 && (f[1] - l[1]).abs() < 1e-9 {
                            let lastn = sp.nodes.pop().expect("node");
                            sp.nodes[0].cin = lastn.cin;
                        }
                    }
                    sp.closed = true;
                    last = sp.nodes[0].p;
                    push(&mut out, Some(sp));
                }
            }
        }
    }
    push(&mut out, cur.take());
    // Re-parse through the writer so handles on anchors are dropped and smooth flags set like any path.
    VectorPath::parse(&out.to_d())
}

fn paint(p: &UPaint, opacity: f64, m: &M) -> Value {
    match p {
        UPaint::Color(c) => json!({ "k": "solid", "color": color(*c, opacity) }),
        UPaint::LinearGradient(g) => {
            let gm = mul(m, &mat(g.transform()));
            let a = ap(&gm, f64::from(g.x1()), f64::from(g.y1()));
            let b = ap(&gm, f64::from(g.x2()), f64::from(g.y2()));
            let angle = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
            let stops: Vec<Value> = g.stops().iter().map(|s| json!({ "t": f64::from(s.offset().get()), "color": color(s.color(), f64::from(s.opacity().get()) * opacity) })).collect();
            json!({ "k": "linear", "angle": (angle * 100.0).round() / 100.0, "stops": stops })
        }
        UPaint::RadialGradient(g) => {
            let stops: Vec<Value> = g.stops().iter().map(|s| json!({ "t": f64::from(s.offset().get()), "color": color(s.color(), f64::from(s.opacity().get()) * opacity) })).collect();
            json!({ "k": "radial", "stops": stops })
        }
        UPaint::Pattern(_) => json!({ "k": "solid", "color": { "space": "srgb", "r": 0.6, "g": 0.6, "b": 0.6, "a": opacity } }),
    }
}

struct Ctx {
    place: M,
    out: Vec<Map<String, Value>>,
    z: Option<String>,
}

impl Ctx {
    fn group(&mut self, g: &usvg::Group, opacity: f64) {
        let op = opacity * f64::from(g.opacity().get());
        for n in g.children() {
            match n {
                Node::Group(c) => self.group(c, op),
                Node::Path(p) => self.path(p, op),
                Node::Text(t) => self.group(t.flattened(), op),
                Node::Image(_) => {}
            }
        }
    }

    fn path(&mut self, p: &usvg::Path, opacity: f64) {
        if !p.is_visible() || (p.fill().is_none() && p.stroke().is_none()) {
            return;
        }
        let m = mul(&self.place, &mat(p.abs_transform()));
        let mut vp = to_vector(p.data(), &m);
        if vp.is_empty() {
            return;
        }
        let mut fills: Vec<Value> = vec![];
        if let Some(f) = p.fill() {
            if f.rule() == usvg::FillRule::EvenOdd {
                let nz = even_odd_to_nonzero(&vp);
                if !nz.is_empty() {
                    vp = nz;
                }
            }
            fills.push(paint(f.paint(), f64::from(f.opacity().get()), &m));
        }
        let mut strokes: Vec<Value> = vec![];
        if let Some(s) = p.stroke() {
            let scale = (m[0] * m[3] - m[1] * m[2]).abs().sqrt();
            let cap = match s.linecap() {
                usvg::LineCap::Butt => "butt",
                usvg::LineCap::Round => "round",
                usvg::LineCap::Square => "square",
            };
            let join = match s.linejoin() {
                usvg::LineJoin::Round => "round",
                usvg::LineJoin::Bevel => "bevel",
                _ => "miter",
            };
            strokes.push(json!({ "paint": paint(s.paint(), f64::from(s.opacity().get()), &m), "width": f64::from(s.width().get()) * scale,
                "align": "center", "cap": cap, "join": join }));
        }
        let Some((b, unit)) = vp.to_unit(0.5) else { return };
        let z = oc_schema::ids::key_between(self.z.as_deref(), None, false).unwrap_or_else(|_| "a0".into());
        self.z = Some(z.clone());
        let mut el = Map::new();
        el.insert("id".into(), json!(oc_schema::ids::new_id()));
        el.insert("type".into(), json!("shape"));
        el.insert("parentId".into(), json!(""));
        el.insert("z".into(), json!(z));
        el.insert("opacity".into(), json!((opacity * 1000.0).round() / 1000.0));
        el.insert("xf".into(), json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h, "rot": 0 }));
        if !p.id().is_empty() {
            el.insert("name".into(), json!(p.id()));
        }
        el.insert("kind".into(), json!("custom"));
        el.insert("params".into(), json!({}));
        el.insert("d".into(), json!(unit.to_d()));
        el.insert("fills".into(), Value::Array(fills));
        el.insert("strokes".into(), Value::Array(strokes));
        self.out.push(el);
    }
}

/// Parse an SVG and turn its drawing into path elements (JSON, `parentId` `""`, bottom first) placed by
/// `place(svg_width, svg_height)`. `None` when the SVG does not parse or draws nothing convertible.
pub fn svg_to_elements(svg: &str, place: impl FnOnce(f64, f64) -> Placement) -> Option<Vec<Map<String, Value>>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let pl = place(f64::from(size.width()), f64::from(size.height()));
    let mut cx = Ctx { place: [pl.sx, 0.0, 0.0, pl.sy, pl.tx, pl.ty], out: vec![], z: None };
    cx.group(tree.root(), 1.0);
    (!cx.out.is_empty()).then_some(cx.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_shapes_with_their_paint_and_place() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50" width="100" height="50">
            <rect x="10" y="10" width="30" height="20" fill="#ff0000"/>
            <g transform="translate(50 0)"><circle cx="20" cy="25" r="20" fill="none" stroke="#0000ff" stroke-width="2"/></g>
            <path d="M0 0 L100 0 L100 50 L0 50 Z M25 12 L75 12 L75 38 L25 38 Z" fill-rule="evenodd" fill="#00ff00" opacity="0.5"/>
        </svg>"##;
        let els = svg_to_elements(svg, |w, h| Placement::image_box(w, h, 200.0, 100.0, (0.0, 0.0, 1.0, 1.0))).unwrap();
        assert_eq!(els.len(), 3);
        // The rect at 2× scale.
        assert_eq!(els[0]["xf"], json!({ "x": 20.0, "y": 20.0, "w": 60.0, "h": 40.0, "rot": 0 }));
        assert_eq!(els[0]["fills"][0]["color"]["r"], json!(1.0));
        assert!(els[0]["strokes"].as_array().unwrap().is_empty());
        // The circle: stroked, unfilled, moved by its group's transform, stroke scaled.
        assert!(els[1]["fills"].as_array().unwrap().is_empty());
        assert_eq!(els[1]["strokes"][0]["width"], json!(4.0));
        let x = els[1]["xf"]["x"].as_f64().unwrap();
        assert!((x - 100.0).abs() < 0.01, "{x}");
        // Even-odd ring resolved to an outer contour plus a counter-wound hole.
        let d = els[2]["d"].as_str().unwrap();
        assert_eq!(VectorPath::parse(d).subpaths.len(), 2, "{d}");
        assert_eq!(els[2]["opacity"], json!(0.5));
        // Order keys increase (paint order kept).
        assert!(els[0]["z"].as_str().unwrap() < els[1]["z"].as_str().unwrap());
    }

    #[test]
    fn crop_shifts_and_scales() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="50" y="50" width="10" height="10"/></svg>"##;
        let els = svg_to_elements(svg, |w, h| Placement::image_box(w, h, 100.0, 100.0, (0.5, 0.5, 0.5, 0.5))).unwrap();
        assert_eq!(els[0]["xf"], json!({ "x": 0.0, "y": 0.0, "w": 20.0, "h": 20.0, "rot": 0 }));
        assert!(svg_to_elements("<svg", |w, h| Placement::image_box(w, h, 1.0, 1.0, (0.0, 0.0, 1.0, 1.0))).is_none());
    }
}
