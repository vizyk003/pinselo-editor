// SPDX-License-Identifier: Apache-2.0
//! PowerPoint export (EX-030), port of `services/render/src/pptx.ts`: an OOXML package built by hand.
//! Editable where PowerPoint has an equivalent — shapes (presets or exact custom geometry), text (runs, fonts,
//! sizes, colours, alignment, lists, spacing, curved text as WordArt arcs), images (with crops), lines and
//! connectors (arrowheads), tables, backgrounds, hidden slides and transitions. Charts, freehand drawings, QR
//! codes, photo grids and edited images are embedded as pictures ([`Renderer::render_rgba`]).
//!
//! All XML is built here; the renderer is asked for element pictures, PNG encoding, image transcoding and SVG
//! path parsing (CanvasKit's `Path.MakeFromSVGString`, which turns arcs into conics).
//!
//! Deviation: the TS builds a group's children concurrently (`Promise.all`), so when a group mixes children
//! that wait for asset bytes with ones that don't, the order in which shape ids, relationship ids and media
//! names are handed out follows promise timing (and, in production, database latency). Here children are built
//! in paint order. The XML is the same up to that numbering.

pub mod parts;

use std::io::Write as _;

use futures::future::BoxFuture;
use oc_geometry::jsmath::{atan2, hypot};
use oc_geometry::{frame_el_outline, frame_overlays, shape_path, xf_from_matrix};
use oc_schema::DesignDoc;
use oc_schema::color::to_rgba;
use oc_schema::elements::{Element, TextEl};
use oc_schema::js;
use oc_schema::primitives::{ColorValue, LinkTarget, Paint, Stroke, StrokeCap, Xf};
use oc_schema::table::{
    CellSide, anchor_of, cell_body, cell_fill, cell_key, cell_side_border, effective_span,
    geometry_of,
};
use oc_schema::text::{CaseTransform, ListKind, TextAlign, TextBody, resolve_run_style};
use oc_schema::tree::{children_of, sorted_pages};

use crate::renderer::{PathCmd, RasterOptions, RenderError, RenderHost, Renderer};

/// Why asset bytes are missing.
#[derive(Debug)]
pub enum AssetError {
    /// Not found / unreadable: the element is skipped (the TS `.catch(() => null)`).
    Missing(String),
    /// Aborts the export (e.g. a recoloured SVG needs the renderer).
    Fatal(RenderError),
}

/// `PptxDeps`.
pub trait PptxDeps: Send + Sync {
    fn asset_bytes<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>, AssetError>>;
}

#[derive(Debug, thiserror::Error)]
pub enum PptxError {
    #[error(transparent)]
    Renderer(#[from] RenderError),
    #[error("zip: {0}")]
    Zip(String),
}

/// The package as named parts.
#[derive(Debug, Clone, Default)]
pub struct PptxPackage {
    pub parts: Vec<(String, Vec<u8>)>,
}

impl PptxPackage {
    pub fn to_zip(&self) -> Result<Vec<u8>, PptxError> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in &self.parts {
            zip.start_file(name.as_str(), opts)
                .map_err(|e| PptxError::Zip(e.to_string()))?;
            zip.write_all(bytes)
                .map_err(|e| PptxError::Zip(e.to_string()))?;
        }
        Ok(zip
            .finish()
            .map_err(|e| PptxError::Zip(e.to_string()))?
            .into_inner())
    }
}

const EMU: f64 = 9525.0;

fn e(px: f64) -> f64 {
    js::round(px * EMU)
}

fn n(v: f64) -> String {
    js::num_to_string(v)
}

/// XML escape (`&<>"`).
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `RRGGBB` of a colour (channels clamped).
pub fn hex(c: &ColorValue) -> String {
    let x = to_rgba(c);
    [x.r, x.g, x.b]
        .iter()
        .map(|v| format!("{:02x}", js::round(v.clamp(0.0, 1.0) * 255.0) as i64))
        .collect::<String>()
        .to_uppercase()
}

fn alpha_tag(c: &ColorValue, extra: f64) -> String {
    let a = to_rgba(c).a * extra;
    if a < 0.999 {
        format!("<a:alpha val=\"{}\"/>", n(js::round(a * 100000.0)))
    } else {
        String::new()
    }
}

/// `((a % 360) + 360) % 360` in 60000ths of a degree.
fn angle(a: f64) -> String {
    n(js::round((((a % 360.0) + 360.0) % 360.0) * 60000.0))
}

/// Closest PowerPoint transition (PresentationML base set).
pub fn pptx_transition(kind: &str, dir: Option<&str>) -> String {
    let d = match dir.unwrap_or("left") {
        "up" => "u",
        "down" => "d",
        "left" => "l",
        "right" => "r",
        _ => "l",
    };
    match kind {
        "dissolve" => "<p:dissolve/>".into(),
        "slide" | "push" => format!("<p:push dir=\"{d}\"/>"),
        "cover" => format!("<p:cover dir=\"{d}\"/>"),
        "reveal" => format!("<p:pull dir=\"{d}\"/>"),
        "wipe" => format!(
            "<p:wipe dir=\"{}\"/>",
            if d == "l" {
                "r"
            } else if d == "r" {
                "l"
            } else {
                d
            }
        ),
        "zoom" => "<p:zoom/>".into(),
        "iris" => "<p:circle/>".into(),
        "split" => format!(
            "<p:split orient=\"{}\" dir=\"out\"/>",
            if dir == Some("up") || dir == Some("down") {
                "horz"
            } else {
                "vert"
            }
        ),
        _ => "<p:fade/>".into(),
    }
}

fn base(el: &Element) -> Option<&oc_schema::ElementBase> {
    el.base()
}

fn hidden(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") == Some(&serde_json::Value::Bool(false)),
        _ => base(el).is_some_and(|b| b.visible == Some(false)),
    }
}

fn opacity(el: &Element) -> f64 {
    match el {
        Element::Unknown(m) => m
            .get("opacity")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(f64::NAN),
        _ => base(el).map_or(1.0, |b| b.opacity),
    }
}

fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

/// The top-level element (child of the page) an element belongs to.
fn top_ancestor<'a>(doc: &'a DesignDoc, id: &'a str, page_id: &str) -> &'a str {
    let mut cur = doc.elements.get(id);
    while let Some(c) = cur {
        let pid = c.parent_id().unwrap_or("");
        if pid == page_id {
            break;
        }
        match doc.elements.get(pid) {
            Some(p) => cur = Some(p),
            None => break,
        }
    }
    cur.and_then(|c| c.id()).unwrap_or(id)
}

fn ext_of(b: &[u8]) -> Option<&'static str> {
    match b {
        [0x89, ..] => Some("png"),
        [0xff, 0xd8, ..] => Some("jpeg"),
        [0x47, 0x49, ..] => Some("gif"),
        _ => None,
    }
}

fn fill_xml(p: Option<&Paint>, opacity: f64) -> String {
    match p {
        None | Some(Paint::Image(_)) => "<a:noFill/>".into(),
        Some(Paint::Solid(s)) => format!(
            "<a:solidFill><a:srgbClr val=\"{}\">{}</a:srgbClr></a:solidFill>",
            hex(&s.color),
            alpha_tag(&s.color, opacity)
        ),
        Some(p @ (Paint::Linear(_) | Paint::Radial(_))) => {
            let (stops, lin) = match p {
                Paint::Linear(l) => (&l.stops, Some(l.angle)),
                Paint::Radial(r) => (&r.stops, None),
                _ => unreachable!(),
            };
            let gs: String = stops
                .iter()
                .map(|s| {
                    format!(
                        "<a:gs pos=\"{}\"><a:srgbClr val=\"{}\">{}</a:srgbClr></a:gs>",
                        n(js::round(s.t * 100000.0)),
                        hex(&s.color),
                        alpha_tag(&s.color, opacity)
                    )
                })
                .collect();
            let shade = match lin {
                Some(a) => format!("<a:lin ang=\"{}\" scaled=\"0\"/>", angle(a)),
                None => "<a:path path=\"circle\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path>".into(),
            };
            format!("<a:gradFill rotWithShape=\"1\"><a:gsLst>{gs}</a:gsLst>{shade}</a:gradFill>")
        }
    }
}

struct Deck<'a> {
    r: &'a dyn Renderer,
    host: &'a dyn RenderHost,
    doc: &'a DesignDoc,
    deps: &'a dyn PptxDeps,
    k: f64,
    media: Vec<(String, Vec<u8>)>,
    media_exts: Vec<&'static str>,
    rels: Vec<String>,
    sp_id: u32,
}

impl<'a> Deck<'a> {
    #[allow(non_snake_case)]
    fn E(&self, px: f64) -> f64 {
        js::round(e(px) * self.k)
    }

    fn en(&self, px: f64) -> String {
        n(self.E(px))
    }

    fn add_media(&mut self, bytes: Vec<u8>, ext: &'static str) -> String {
        let name = format!("image{}.{ext}", self.media.len() + 1);
        self.media.push((name.clone(), bytes));
        self.media_exts.push(ext);
        name
    }

    fn rel_id(&self) -> String {
        format!("rId{}", self.rels.len() + 2)
    }

    fn link_for(&mut self, url: &str) -> String {
        let rid = self.rel_id();
        self.rels.push(format!("<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>", esc(url)));
        format!("<a:hlinkClick r:id=\"{rid}\"/>")
    }

    fn image_rel(&mut self, name: &str) -> String {
        let rid = self.rel_id();
        self.rels.push(format!("<Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/{name}\"/>"));
        rid
    }

    async fn asset(&self, key: &str) -> Result<Option<Vec<u8>>, RenderError> {
        match self.deps.asset_bytes(key).await {
            Ok(b) => Ok(Some(b)),
            Err(AssetError::Missing(_)) => Ok(None),
            Err(AssetError::Fatal(e)) => Err(e),
        }
    }

    async fn to_png(&self, b: &[u8]) -> Result<Option<Vec<u8>>, RenderError> {
        Ok(self.r.transcode_png(b).await?.map(|d| d.png))
    }

    /// Render one element (and its children) to a transparent PNG cropped to its page-space bounds.
    async fn raster_element(
        &self,
        page_id: &str,
        el_id: &str,
        xf: &Xf,
    ) -> Result<Option<Vec<u8>>, RenderError> {
        let top = top_ancestor(self.doc, el_id, page_id);
        let skip: Vec<String> = children_of(&self.doc.elements, page_id)
            .into_iter()
            .filter_map(|x| x.id())
            .filter(|id| *id != top)
            .map(str::to_string)
            .collect();
        let scale = 2.0;
        let rgba = self
            .r
            .render_rgba(
                self.host,
                self.doc,
                page_id,
                &RasterOptions {
                    scale,
                    transparent: true,
                    skip,
                    ..RasterOptions::default()
                },
            )
            .await?;
        let x0 = (xf.x * scale).floor().max(0.0);
        let y0 = (xf.y * scale).floor().max(0.0);
        let cw = (f64::from(rgba.width) - x0)
            .min((xf.w * scale).ceil())
            .max(1.0);
        let ch = (f64::from(rgba.height) - y0)
            .min((xf.h * scale).ceil())
            .max(1.0);
        let cut = crate::email::crop_like_ts(&rgba, x0, y0, cw, ch);
        Ok(Some(self.r.encode_png(&cut).await?))
    }

    fn ln_xml(&self, s: Option<&Stroke>, ends: &str) -> String {
        let Some(s) = s.filter(|s| s.width > 0.0) else {
            return "<a:ln><a:noFill/></a:ln>".into();
        };
        let dash = match s.dash.as_deref() {
            Some([first, ..]) => format!(
                "<a:prstDash val=\"{}\"/>",
                if *first < 1.0 { "sysDot" } else { "dash" }
            ),
            _ => String::new(),
        };
        let cap = match s.cap {
            StrokeCap::Round => " cap=\"rnd\"",
            StrokeCap::Square => " cap=\"sq\"",
            _ => "",
        };
        format!(
            "<a:ln w=\"{}\"{cap}>{}{dash}{ends}</a:ln>",
            self.en(s.width),
            fill_xml(Some(&s.paint), 1.0)
        )
    }

    /// Custom geometry from our SVG path (shape-local px → path units in EMU).
    fn cust_geom(&self, d: &str, w: f64, h: f64, stroked: bool) -> Result<String, RenderError> {
        let Some(cmds) = self.r.path_commands(d)? else {
            return Ok("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".into());
        };
        let pt = |x: f64, y: f64| format!("<a:pt x=\"{}\" y=\"{}\"/>", self.en(x), self.en(y));
        let mut out = String::new();
        let (mut cx, mut cy) = (0.0, 0.0);
        for c in cmds {
            match c {
                PathCmd::Move(x, y) => {
                    (cx, cy) = (x, y);
                    out.push_str(&format!("<a:moveTo>{}</a:moveTo>", pt(cx, cy)));
                }
                PathCmd::Line(x, y) => {
                    (cx, cy) = (x, y);
                    out.push_str(&format!("<a:lnTo>{}</a:lnTo>", pt(cx, cy)));
                }
                PathCmd::Quad(qx, qy, x, y) => {
                    out.push_str(&format!(
                        "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                        pt(cx + (2.0 / 3.0) * (qx - cx), cy + (2.0 / 3.0) * (qy - cy)),
                        pt(x + (2.0 / 3.0) * (qx - x), y + (2.0 / 3.0) * (qy - y)),
                        pt(x, y)
                    ));
                    (cx, cy) = (x, y);
                }
                PathCmd::Conic(qx, qy, x, y, wt) => {
                    let kk = (4.0 * wt) / (3.0 * (1.0 + wt));
                    out.push_str(&format!(
                        "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                        pt(cx + kk * (qx - cx), cy + kk * (qy - cy)),
                        pt(x + kk * (qx - x), y + kk * (qy - y)),
                        pt(x, y)
                    ));
                    (cx, cy) = (x, y);
                }
                PathCmd::Cubic(x1, y1, x2, y2, x, y) => {
                    out.push_str(&format!(
                        "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                        pt(x1, y1),
                        pt(x2, y2),
                        pt(x, y)
                    ));
                    (cx, cy) = (x, y);
                }
                PathCmd::Close => out.push_str("<a:close/>"),
            }
        }
        Ok(format!(
            "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{}\" h=\"{}\"{}>{out}</a:path></a:pathLst></a:custGeom>",
            self.en(w),
            self.en(h),
            if stroked { " fill=\"none\"" } else { "" }
        ))
    }

    fn xfrm(&self, xf: &Xf, tag: &str) -> String {
        format!(
            "<{tag}{}{}{}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></{tag}>",
            if truthy(xf.rot) {
                format!(" rot=\"{}\"", angle(xf.rot))
            } else {
                String::new()
            },
            if xf.flip_x.unwrap_or(false) {
                " flipH=\"1\""
            } else {
                ""
            },
            if xf.flip_y.unwrap_or(false) {
                " flipV=\"1\""
            } else {
                ""
            },
            self.en(xf.x),
            self.en(xf.y),
            self.en(xf.w.max(1.0)),
            self.en(xf.h.max(1.0))
        )
    }

    fn effect_xml(&self, el: &Element) -> String {
        let sh = base(el).and_then(|b| b.effects.as_ref()).and_then(|fx| {
            fx.iter().find_map(|x| match x {
                oc_schema::elements::Effect::DropShadow(d) => Some(d),
                _ => None,
            })
        });
        let Some(sh) = sh else { return String::new() };
        let dist = hypot(sh.dx, sh.dy);
        let dir = ((atan2(sh.dy, sh.dx) * 180.0) / std::f64::consts::PI + 360.0) % 360.0;
        format!(
            "<a:effectLst><a:outerShdw blurRad=\"{}\" dist=\"{}\" dir=\"{}\" algn=\"ctr\" rotWithShape=\"0\"><a:srgbClr val=\"{}\">{}</a:srgbClr></a:outerShdw></a:effectLst>",
            self.en(sh.blur),
            self.en(dist),
            n(js::round(dir * 60000.0)),
            hex(&sh.color),
            alpha_tag(&sh.color, 1.0)
        )
    }

    /// DrawingML paragraphs with rich runs (shared by text boxes and table cells).
    fn paras_xml(&mut self, b: &TextBody) -> String {
        let mut out = String::new();
        for p in &b.paragraphs {
            let algn = match &p.align {
                TextAlign::Start => "l",
                TextAlign::Center => "ctr",
                TextAlign::End => "r",
                TextAlign::Justify => "just",
                _ => "undefined",
            };
            let bu = match &p.list {
                Some(l) if l.kind == ListKind::Bullet => {
                    "<a:buFont typeface=\"Arial\"/><a:buChar char=\"•\"/>"
                }
                Some(_) => "<a:buAutoNum type=\"arabicPeriod\"/>",
                None => "<a:buNone/>",
            };
            let indent = match &p.list {
                Some(l) => format!(
                    " marL=\"{}\" indent=\"{}\"",
                    self.en(b.default_run.size * 1.4 * (l.level + 1.0)),
                    n(-self.E(b.default_run.size * 1.1))
                ),
                None => String::new(),
            };
            let mut runs = String::new();
            for run in &p.runs {
                let s = resolve_run_style(b, run);
                let fill = match &s.fill {
                    Paint::Solid(_) => Some(s.fill.clone()),
                    Paint::Linear(l) => l
                        .stops
                        .first()
                        .map(|st| Paint::solid_color(st.color.clone())),
                    Paint::Radial(r) => r
                        .stops
                        .first()
                        .map(|st| Paint::solid_color(st.color.clone())),
                    Paint::Image(_) => None,
                };
                let lang = if self.doc.meta.lang.is_empty() {
                    "en-US".to_string()
                } else {
                    self.doc.meta.lang.clone()
                };
                let mut attrs: Vec<String> = vec![
                    format!("lang=\"{}\"", esc(&lang)),
                    format!("sz=\"{}\"", n(js::round(s.size * 0.75 * 100.0))),
                    if s.weight >= 600.0 {
                        "b=\"1\"".into()
                    } else {
                        "b=\"0\"".into()
                    },
                    if s.italic {
                        "i=\"1\"".into()
                    } else {
                        "i=\"0\"".into()
                    },
                ];
                if s.underline.unwrap_or(false) {
                    attrs.push("u=\"sng\"".into());
                }
                if s.strike.unwrap_or(false) {
                    attrs.push("strike=\"sngStrike\"".into());
                }
                if s.case_transform == Some(CaseTransform::Upper) {
                    attrs.push("cap=\"all\"".into());
                }
                if truthy(s.tracking) {
                    attrs.push(format!(
                        "spc=\"{}\"",
                        n(js::round(((s.tracking / 1000.0) * s.size * 0.75) * 100.0))
                    ));
                }
                attrs.push("dirty=\"0\"".into());
                let hl = s.highlight.as_ref().map_or_else(String::new, |h| {
                    format!("<a:highlight><a:srgbClr val=\"{}\"/></a:highlight>", hex(h))
                });
                let link = match &run.link {
                    Some(LinkTarget::Url(u)) => self.link_for(&u.url),
                    _ => String::new(),
                };
                runs.push_str(&format!(
                    "<a:r><a:rPr {}>{}{hl}<a:latin typeface=\"{}\"/><a:cs typeface=\"{}\"/>{link}</a:rPr><a:t>{}</a:t></a:r>",
                    attrs.join(" "),
                    fill_xml(fill.as_ref(), 1.0),
                    esc(&s.family),
                    esc(&s.family),
                    esc(&run.text)
                ));
            }
            out.push_str(&format!(
                "<a:p><a:pPr algn=\"{algn}\"{indent}><a:lnSpc><a:spcPct val=\"{}\"/></a:lnSpc><a:spcAft><a:spcPts val=\"{}\"/></a:spcAft>{bu}</a:pPr>{runs}<a:endParaRPr lang=\"en-US\" sz=\"{}\"/></a:p>",
                n(js::round(p.line_height * 100000.0 / 1.2)),
                n(js::round(p.space_after * 75.0)),
                n(js::round(b.default_run.size * 75.0))
            ));
        }
        out
    }

    fn text_body_xml(&mut self, el: &TextEl) -> String {
        let curve = el.curve.filter(|c| truthy(*c));
        let warp = curve.map_or_else(String::new, |c| {
            format!(
                "<a:prstTxWarp prst=\"{}\"><a:avLst/></a:prstTxWarp>",
                if c > 0.0 {
                    "textArchUp"
                } else {
                    "textArchDown"
                }
            )
        });
        let fit = if el.auto_fit.as_str() == "shrink" {
            "<a:normAutofit/>"
        } else if el.frame.mode.as_str() == "autoHeight" {
            "<a:spAutoFit/>"
        } else {
            ""
        };
        let paras = self.paras_xml(&el.body);
        format!(
            "<p:txBody><a:bodyPr wrap=\"{}\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"t\" rtlCol=\"0\">{warp}{fit}</a:bodyPr><a:lstStyle/>{paras}</p:txBody>",
            if curve.is_some() { "none" } else { "square" }
        )
    }

    fn nv(&mut self, tag: &str, el: &Element, extra: &str) -> String {
        let b = base(el);
        let name = esc(&b
            .and_then(|b| b.name.clone())
            .unwrap_or_else(|| format!("{} {}", el.type_name(), self.sp_id)));
        let descr = b
            .and_then(|b| b.alt_text.as_deref())
            .filter(|a| !a.is_empty())
            .map_or_else(String::new, |a| format!(" descr=\"{}\"", esc(a)));
        let link = match b.and_then(|b| b.link.as_ref()) {
            Some(LinkTarget::Url(u)) => self.link_for(&u.url),
            _ => String::new(),
        };
        self.sp_id += 1;
        let inner = format!(
            "<p:cNvPr id=\"{}\" name=\"{name}\"{descr}>{link}</p:cNvPr>",
            self.sp_id
        );
        match tag {
            "sp" => format!("<p:nvSpPr>{inner}<p:cNvSpPr{extra}/><p:nvPr/></p:nvSpPr>"),
            "pic" => format!(
                "<p:nvPicPr>{inner}<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>"
            ),
            "cxnSp" => format!("<p:nvCxnSpPr>{inner}<p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>"),
            _ => format!(
                "<p:nvGraphicFramePr>{inner}<p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>"
            ),
        }
    }

    fn picture_xml(
        &mut self,
        el: &Element,
        xf: &Xf,
        rid: &str,
        crop: Option<&oc_schema::primitives::StoredRect>,
        geom: Option<String>,
        ln: &str,
    ) -> String {
        let src = match crop {
            Some(c) if c.x > 0.0001 || c.y > 0.0001 || c.w < 0.9999 || c.h < 0.9999 => format!(
                "<a:srcRect l=\"{}\" t=\"{}\" r=\"{}\" b=\"{}\"/>",
                n(js::round(c.x * 100000.0)),
                n(js::round(c.y * 100000.0)),
                n(js::round((1.0 - c.x - c.w) * 100000.0)),
                n(js::round((1.0 - c.y - c.h) * 100000.0))
            ),
            _ => String::new(),
        };
        let op = opacity(el);
        let alpha = if op < 0.999 {
            format!("<a:alphaModFix amt=\"{}\"/>", n(js::round(op * 100000.0)))
        } else {
            String::new()
        };
        let geom =
            geom.unwrap_or_else(|| "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".into());
        let nv = self.nv("pic", el, "");
        format!(
            "<p:pic>{nv}<p:blipFill><a:blip r:embed=\"{rid}\">{alpha}</a:blip>{src}<a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}{geom}{ln}{}</p:spPr></p:pic>",
            self.xfrm(xf, "a:xfrm"),
            self.effect_xml(el)
        )
    }

    fn bounds_of(&self, el: &Element) -> Xf {
        let m = crate::geom::world_matrix(&self.doc.elements, el.id().unwrap_or(""));
        let (w, h) = crate::geom::xf_of(el).map_or((0.0, 0.0), |x| (x.w, x.h));
        let pts: Vec<[f64; 2]> = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]
            .iter()
            .map(|[x, y]| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]])
            .collect();
        let (x0, x1) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[0]), b.max(p[0]))
            });
        let (y0, y1) = pts
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[1]), b.max(p[1]))
            });
        Xf::new(x0, y0, x1 - x0, y1 - y0, 0.0)
    }

    fn element_xml<'s>(
        &'s mut self,
        page_id: &'s str,
        el: &'a Element,
    ) -> BoxFuture<'s, Result<String, RenderError>> {
        Box::pin(async move {
            if hidden(el) || el.is_deleted() {
                return Ok(String::new());
            }
            let id = el.id().unwrap_or("");
            let own = crate::geom::xf_of(el).unwrap_or_else(|| Xf::new(0.0, 0.0, 0.0, 0.0, 0.0));
            let xf = xf_from_matrix(
                &crate::geom::world_matrix(&self.doc.elements, id),
                own.w,
                own.h,
            );
            let mut full = xf.clone();
            full.flip_x = own.flip_x;
            full.flip_y = own.flip_y;
            match el {
                Element::Group(g)
                    if g.mask.as_ref().is_some_and(|m| {
                        children_of(&self.doc.elements, id).iter().any(|c| c.id() == Some(m.id.as_str()) && !hidden(c))
                    }) =>
                {
                    // PowerPoint has no masks (DS-107): a masked group goes in as a picture of what it shows.
                    let b = crate::geom::world_bounds(&self.doc.elements, id);
                    let x = Xf::new(b.x, b.y, b.w, b.h, 0.0);
                    let Some(png) = self.raster_element(page_id, id, &x).await? else {
                        return Ok(String::new());
                    };
                    let name = self.add_media(png, "png");
                    let rid = self.image_rel(&name);
                    Ok(self.picture_xml(el, &x, &rid, None, None, ""))
                }
                Element::Group(_) => {
                    let mut s = String::new();
                    for c in children_of(&self.doc.elements, id) {
                        s.push_str(&self.element_xml(page_id, c).await?);
                    }
                    Ok(s)
                }
                Element::Shape(s) => {
                    // Presets only where DrawingML's default geometry matches ours exactly; everything else is exact custGeom.
                    let kind = s.kind.as_str();
                    let cr = s.params.get("cornerRadius").copied();
                    let prst = match kind {
                        "rect" => Some(if cr.is_some_and(truthy) {
                            "roundRect"
                        } else {
                            "rect"
                        }),
                        "ellipse" => Some("ellipse"),
                        "triangle" => Some("triangle"),
                        "right-triangle" => Some("rtTriangle"),
                        "diamond" => Some("diamond"),
                        "pill" => Some("roundRect"),
                        _ => None,
                    };
                    let adj = if kind == "pill" {
                        "<a:gd name=\"adj\" fmla=\"val 50000\"/>".to_string()
                    } else if prst == Some("roundRect") {
                        format!(
                            "<a:gd name=\"adj\" fmla=\"val {}\"/>",
                            n(js::round(50000f64.min(
                                (cr.unwrap_or(f64::NAN) / own.w.min(own.h)) * 100000.0
                            )))
                        )
                    } else {
                        String::new()
                    };
                    let geom = match prst {
                        Some(p) => format!(
                            "<a:prstGeom prst=\"{p}\"><a:avLst>{adj}</a:avLst></a:prstGeom>"
                        ),
                        None => self.cust_geom(
                            &shape_path(kind, own.w, own.h, &s.params, s.d.as_deref()),
                            own.w,
                            own.h,
                            false,
                        )?,
                    };
                    // A photo filling the shape (dropped onto it): a picture cropped like the canvas shows it, clipped
                    // to the outline.
                    let photo = oc_render::resolve::shape_photo(s, self.doc);
                    let fill_image = photo.as_ref().map(|(_, p, _)| *p).or(match s.fills.first() {
                        Some(Paint::Image(img)) => Some(img),
                        _ => None,
                    });
                    if let Some(img) = fill_image {
                        let bytes = self.asset(&img.asset).await?;
                        if let Some(bytes) = bytes
                            && let Some(ext) = ext_of(&bytes)
                        {
                            let name = self.add_media(bytes, ext);
                            let rid = self.image_rel(&name);
                            let st = s.strokes.first().filter(|st| st.width > 0.0);
                            let ln = if st.is_some() { self.ln_xml(st, "") } else { String::new() };
                            return Ok(self.picture_xml(el, &full, &rid, photo.as_ref().map(|(_, _, c)| c), Some(geom), &ln));
                        }
                    }
                    let nv = self.nv("sp", el, "");
                    Ok(format!(
                        "<p:sp>{nv}<p:spPr>{}{geom}{}{}{}</p:spPr></p:sp>",
                        self.xfrm(&full, "a:xfrm"),
                        fill_xml(s.fills.first(), s.base.opacity),
                        self.ln_xml(s.strokes.first(), ""),
                        self.effect_xml(el)
                    ))
                }
                Element::Text(t) => {
                    let nv = self.nv("sp", el, " txBox=\"1\"");
                    let head = format!(
                        "<p:sp>{nv}<p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/>{}</p:spPr>",
                        self.xfrm(&full, "a:xfrm"),
                        self.effect_xml(el)
                    );
                    Ok(format!("{head}{}</p:sp>", self.text_body_xml(t)))
                }
                Element::Line(l) => {
                    let m = crate::geom::world_matrix(&self.doc.elements, id);
                    if l.points.len() == 2 {
                        let tr = |p: &[f64; 2]| {
                            [
                                m[0] * p[0] + m[2] * p[1] + m[4],
                                m[1] * p[0] + m[3] * p[1] + m[5],
                            ]
                        };
                        let (a, b) = (tr(&l.points[0]), tr(&l.points[1]));
                        let mut lx = Xf::new(
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            (b[0] - a[0]).abs(),
                            (b[1] - a[1]).abs(),
                            0.0,
                        );
                        lx.flip_x = Some(b[0] < a[0]);
                        lx.flip_y = Some(b[1] < a[1]);
                        // DrawingML has no hollow ends: outline markers map to their filled counterpart.
                        let head = |mk: &str| match mk {
                            "arrow" => "arrow",
                            "triangle" | "triangle-outline" => "triangle",
                            "stealth" | "bar" => "stealth",
                            "circle" | "circle-outline" => "oval",
                            "square" | "square-outline" | "diamond" | "diamond-outline" => {
                                "diamond"
                            }
                            _ => "",
                        };
                        let (hs, he) = (head(l.start_marker.as_str()), head(l.end_marker.as_str()));
                        let ends = format!(
                            "{}{}",
                            if hs.is_empty() {
                                String::new()
                            } else {
                                format!("<a:headEnd type=\"{hs}\"/>")
                            },
                            if he.is_empty() {
                                String::new()
                            } else {
                                format!("<a:tailEnd type=\"{he}\"/>")
                            }
                        );
                        let nv = self.nv("cxnSp", el, "");
                        return Ok(format!(
                            "<p:cxnSp>{nv}<p:spPr>{}<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom>{}</p:spPr></p:cxnSp>",
                            self.xfrm(&lx, "a:xfrm"),
                            self.ln_xml(Some(&l.stroke), &ends)
                        ));
                    }
                    // Polyline connectors (elbow/curved): stroke-only custom geometry.
                    let d = format!(
                        "M{}",
                        l.points
                            .iter()
                            .map(|p| format!("{} {}", n(p[0]), n(p[1])))
                            .collect::<Vec<_>>()
                            .join(" L")
                    );
                    let nv = self.nv("sp", el, "");
                    Ok(format!(
                        "<p:sp>{nv}<p:spPr>{}{}<a:noFill/>{}</p:spPr></p:sp>",
                        self.xfrm(&full, "a:xfrm"),
                        self.cust_geom(&d, own.w, own.h, true)?,
                        self.ln_xml(Some(&l.stroke), "")
                    ))
                }
                Element::Image(img) => {
                    if img.edits.is_some() {
                        let png = self.raster_element(page_id, id, &full).await?;
                        let Some(png) = png else {
                            return Ok(String::new());
                        };
                        let mut x = full.clone();
                        x.rot = 0.0;
                        x.flip_x = Some(false);
                        x.flip_y = Some(false);
                        let name = self.add_media(png, "png");
                        let rid = self.image_rel(&name);
                        return Ok(self.picture_xml(el, &x, &rid, None, None, ""));
                    }
                    // Recoloured SVGs (A4) embed their recoloured raster.
                    let key = match &img.svg_colors {
                        Some(map) => oc_schema::svg::recolor_key(&img.asset, Some(map)),
                        None => img.asset.to_string(),
                    };
                    let Some(bytes) = self.asset(&key).await? else {
                        return Ok(String::new());
                    };
                    let (data, ext) = match ext_of(&bytes) {
                        Some(ext) => (bytes, ext),
                        None => match self.to_png(&bytes).await? {
                            Some(png) => (png, "png"),
                            None => return Ok(String::new()),
                        },
                    };
                    let geom = img.corner_radius.filter(|r| truthy(*r)).map(|r| {
                        format!("<a:prstGeom prst=\"roundRect\"><a:avLst><a:gd name=\"adj\" fmla=\"val {}\"/></a:avLst></a:prstGeom>", n(js::round(50000f64.min((r / own.w.min(own.h)) * 100000.0))))
                    });
                    let name = self.add_media(data, ext);
                    let rid = self.image_rel(&name);
                    Ok(self.picture_xml(el, &full, &rid, Some(&img.crop), geom, ""))
                }
                Element::Frame(f) => {
                    let outline =
                        self.cust_geom(&frame_el_outline(f, own.w, own.h), own.w, own.h, false)?;
                    let has_overlay = f.overlay.as_ref().is_some_and(|o| !o.is_empty());
                    let mut flat = full.clone();
                    flat.rot = 0.0;
                    // Library chrome (device bezels, polaroid borders…) has no picture equivalent: export a faithful picture.
                    if has_overlay && f.content.is_some() {
                        let Some(png) = self.raster_element(page_id, id, &full).await? else {
                            return Ok(String::new());
                        };
                        let name = self.add_media(png, "png");
                        let rid = self.image_rel(&name);
                        return Ok(self.picture_xml(el, &flat, &rid, None, None, ""));
                    }
                    let Some(content) = &f.content else {
                        let nv = self.nv("sp", el, "");
                        let ph = format!(
                            "<p:sp>{nv}<p:spPr>{}{outline}{}</p:spPr></p:sp>",
                            self.xfrm(&full, "a:xfrm"),
                            fill_xml(Some(&f.placeholder_fill), 1.0)
                        );
                        if !has_overlay {
                            return Ok(ph);
                        }
                        let mut out = ph;
                        for o in frame_overlays(f.overlay.as_deref(), own.w, own.h) {
                            let nv = self.nv("sp", el, "");
                            out.push_str(&format!(
                                "<p:sp>{nv}<p:spPr>{}{}{}</p:spPr></p:sp>",
                                self.xfrm(&full, "a:xfrm"),
                                self.cust_geom(&o.path, own.w, own.h, false)?,
                                fill_xml(Some(&o.overlay.fill), 1.0)
                            ));
                        }
                        return Ok(out);
                    };
                    // Videos in frames export their poster frame.
                    let key = if content.video.is_some() {
                        format!("{}/poster", content.asset)
                    } else {
                        content.asset.to_string()
                    };
                    let bytes = self.asset(&key).await?;
                    match bytes.and_then(|b| ext_of(&b).map(|e| (b, e))) {
                        Some((bytes, ext)) if content.edits.is_none() => {
                            let name = self.add_media(bytes, ext);
                            let rid = self.image_rel(&name);
                            Ok(self.picture_xml(
                                el,
                                &full,
                                &rid,
                                Some(&content.crop),
                                Some(outline),
                                "",
                            ))
                        }
                        _ => {
                            let Some(png) = self.raster_element(page_id, id, &full).await? else {
                                return Ok(String::new());
                            };
                            let name = self.add_media(png, "png");
                            let rid = self.image_rel(&name);
                            Ok(self.picture_xml(el, &flat, &rid, None, None, ""))
                        }
                    }
                }
                Element::Table(t) => {
                    // Native table (A12): merged cells (gridSpan/rowSpan + hMerge/vMerge), fills, per-side borders, rich runs.
                    let geo = geometry_of(t);
                    let cols_w: Vec<f64> = (0..t.cols.len())
                        .map(|i| self.E(geo.xs[i + 1] - geo.xs[i]))
                        .collect();
                    let rows_h: Vec<f64> = (0..t.rows.len())
                        .map(|i| self.E(geo.ys[i + 1] - geo.ys[i]))
                        .collect();
                    let ln = |d: &Self, tag: &str, b: &oc_schema::elements::CellBorder| {
                        if b.width > 0.0 {
                            format!(
                                "<a:{tag} w=\"{}\" cap=\"flat\" cmpd=\"sng\">{}</a:{tag}>",
                                d.en(b.width),
                                fill_xml(Some(&Paint::solid_color(b.color.clone())), 1.0)
                            )
                        } else {
                            format!("<a:{tag} w=\"0\"><a:noFill/></a:{tag}>")
                        }
                    };
                    let empty = "<a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\"/></a:p></a:txBody><a:tcPr/>";
                    let mut rows_xml = String::new();
                    for (ri, rh) in rows_h.iter().enumerate() {
                        let mut tr = format!("<a:tr h=\"{}\">", n(*rh));
                        for ci in 0..cols_w.len() {
                            let (r, c) = (ri as i64, ci as i64);
                            let a = anchor_of(t, r, c);
                            if a.r != r || a.c != c {
                                tr.push_str(&format!(
                                    "<a:tc{}{}>{empty}</a:tc>",
                                    if a.r == r { "" } else { " vMerge=\"1\"" },
                                    if a.c == c { "" } else { " hMerge=\"1\"" }
                                ));
                                continue;
                            }
                            let span = effective_span(t, r, c);
                            let cell = t.cells.get(&cell_key(r, c));
                            let pad = cell.and_then(|x| x.padding).unwrap_or(t.style.padding);
                            let anchor = match cell
                                .and_then(|x| x.valign.as_ref())
                                .map_or("middle", |v| v.as_str())
                            {
                                "top" => "t",
                                "middle" => "ctr",
                                "bottom" => "b",
                                _ => "undefined",
                            };
                            let sides = [
                                ("lnL", CellSide::L),
                                ("lnR", CellSide::R),
                                ("lnT", CellSide::T),
                                ("lnB", CellSide::B),
                            ]
                            .iter()
                            .map(|(tag, side)| ln(self, tag, &cell_side_border(t, r, c, side)))
                            .collect::<String>();
                            let fill = cell_fill(t, r, c).map_or_else(
                                || "<a:noFill/>".to_string(),
                                |f| fill_xml(Some(&f), 1.0),
                            );
                            let attrs = format!(
                                "{}{}",
                                if span.cols > 1 {
                                    format!(" gridSpan=\"{}\"", span.cols)
                                } else {
                                    String::new()
                                },
                                if span.rows > 1 {
                                    format!(" rowSpan=\"{}\"", span.rows)
                                } else {
                                    String::new()
                                }
                            );
                            let paras = self.paras_xml(&cell_body(t, r, c));
                            let p = self.en(pad);
                            tr.push_str(&format!("<a:tc{attrs}><a:txBody><a:bodyPr/><a:lstStyle/>{paras}</a:txBody><a:tcPr marL=\"{p}\" marR=\"{p}\" marT=\"{p}\" marB=\"{p}\" anchor=\"{anchor}\">{sides}{fill}</a:tcPr></a:tc>"));
                        }
                        tr.push_str("</a:tr>");
                        rows_xml.push_str(&tr);
                    }
                    let nv = self.nv("graphicFrame", el, "");
                    Ok(format!(
                        "<p:graphicFrame>{nv}{}<a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>{}</a:tblGrid>{rows_xml}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
                        self.xfrm(&full, "p:xfrm"),
                        cols_w
                            .iter()
                            .map(|c| format!("<a:gridCol w=\"{}\"/>", n(*c)))
                            .collect::<String>()
                    ))
                }
                Element::Video(v) => {
                    let bytes = self.asset(&format!("{}/poster", v.asset)).await?;
                    match bytes.and_then(|b| ext_of(&b).map(|e| (b, e))) {
                        Some((bytes, ext)) => {
                            let name = self.add_media(bytes, ext);
                            let rid = self.image_rel(&name);
                            Ok(self.picture_xml(el, &full, &rid, Some(&v.crop), None, ""))
                        }
                        None => Ok(String::new()),
                    }
                }
                _ => {
                    // chart, draw, qr, grid and anything new: a faithful picture.
                    let b = self.bounds_of(el);
                    let Some(png) = self.raster_element(page_id, id, &b).await? else {
                        return Ok(String::new());
                    };
                    let name = self.add_media(png, "png");
                    let rid = self.image_rel(&name);
                    Ok(self.picture_xml(el, &b, &rid, None, None, ""))
                }
            }
        })
    }
}

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// `exportPptx(r, doc, pages, deps)` as named parts. `now` stamps docProps/core.xml.
pub async fn pptx_package(
    r: &dyn Renderer,
    host: &dyn RenderHost,
    doc: &DesignDoc,
    page_ids: &[String],
    deps: &dyn PptxDeps,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PptxPackage, PptxError> {
    // SEC-01: no javascript:/data: links in the output, whatever the stored design holds.
    let safe = oc_schema::links::with_safe_links(doc);
    let doc: &DesignDoc = &safe;
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    // PowerPoint accepts 1–56 in; scale the whole deck uniformly when the design is outside that range.
    let (min_e, max_e) = (914400.0, 51206400.0);
    let k = 1f64.min(max_e / e(w.max(h))) * 1f64.max(min_e / e(w.min(h)));
    let mut deck = Deck {
        r,
        host,
        doc,
        deps,
        k,
        media: Vec::new(),
        media_exts: Vec::new(),
        rels: Vec::new(),
        sp_id: 1,
    };
    let all = sorted_pages(&doc.pages);
    let pages: Vec<&oc_schema::Page> = page_ids
        .iter()
        .filter_map(|id| all.iter().find(|p| *p.id == **id).copied())
        .collect();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut slide_ids: Vec<String> = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        deck.rels.clear();
        deck.sp_id = 1;
        let mut shapes = String::new();
        for el in children_of(&doc.elements, &page.id) {
            shapes.push_str(&deck.element_xml(&page.id, el).await?);
        }
        let bg_xml = match page.background.first() {
            Some(Paint::Image(img)) => match deck
                .asset(&img.asset)
                .await?
                .and_then(|b| ext_of(&b).map(|e| (b, e)))
            {
                Some((bytes, ext)) => {
                    let name = deck.add_media(bytes, ext);
                    let rid = deck.image_rel(&name);
                    format!(
                        "<p:bg><p:bgPr><a:blipFill dpi=\"0\" rotWithShape=\"1\"><a:blip r:embed=\"{rid}\"/><a:srcRect/><a:stretch><a:fillRect/></a:stretch></a:blipFill><a:effectLst/></p:bgPr></p:bg>"
                    )
                }
                None => String::new(),
            },
            Some(bg) => format!(
                "<p:bg><p:bgPr>{}<a:effectLst/></p:bgPr></p:bg>",
                fill_xml(Some(bg), 1.0)
            ),
            None => String::new(),
        };
        let tr_xml = page.transition.as_ref().map_or_else(String::new, |t| {
            let spd = if t.duration_ms < 500.0 {
                "fast"
            } else if t.duration_ms < 900.0 {
                "med"
            } else {
                "slow"
            };
            format!(
                "<p:transition spd=\"{spd}\">{}</p:transition>",
                pptx_transition(t.kind.as_str(), t.direction.as_ref().map(|d| d.as_str()))
            )
        });
        let slide = format!(
            "{DECL}<p:sld {NS}{}><p:cSld>{bg_xml}<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>{tr_xml}</p:sld>",
            if page.hidden.unwrap_or(false) {
                " show=\"0\""
            } else {
                ""
            }
        );
        parts.push((format!("ppt/slides/slide{}.xml", i + 1), slide.into_bytes()));
        let rels = format!(
            "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>{}</Relationships>",
            deck.rels.join("")
        );
        parts.push((
            format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
            rels.into_bytes(),
        ));
        slide_ids.push(format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + i,
            i + 2
        ));
    }

    let (cx, cy) = (deck.en(w), deck.en(h));
    let has = |ext: &str| deck.media_exts.contains(&ext);
    let slide_overrides: String = (0..pages.len()).map(|i| format!("<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>", i + 1)).collect();
    let types = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>{}{}{}<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>{slide_overrides}</Types>",
        if has("png") {
            "<Default Extension=\"png\" ContentType=\"image/png\"/>"
        } else {
            ""
        },
        if has("jpeg") {
            "<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>"
        } else {
            ""
        },
        if has("gif") {
            "<Default Extension=\"gif\" ContentType=\"image/gif\"/>"
        } else {
            ""
        },
    );
    let stamp = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let core = format!(
        "{DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dcterms:created xsi:type=\"dcterms:W3CDTF\">{stamp}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{stamp}</dcterms:modified></cp:coreProperties>",
        esc(&doc.meta.title)
    );
    let app = format!(
        "{DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Pinselo</Application><Slides>{}</Slides></Properties>",
        pages.len()
    );
    let presentation = format!(
        "{DECL}<p:presentation {NS} saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{}</p:sldIdLst><p:sldSz cx=\"{cx}\" cy=\"{cy}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>",
        slide_ids.join("")
    );
    let pres_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>{}<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>",
        (0..pages.len()).map(|i| format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>", i + 2, i + 1)).collect::<String>(),
        pages.len() + 2
    );
    let mut out: Vec<(String, Vec<u8>)> = deck
        .media
        .into_iter()
        .map(|(name, b)| (format!("ppt/media/{name}"), b))
        .collect();
    out.extend(parts);
    let s = |x: &str| x.as_bytes().to_vec();
    out.extend([
        ("[Content_Types].xml".to_string(), types.into_bytes()),
        ("_rels/.rels".to_string(), s(parts::PACKAGE_RELS)),
        ("docProps/core.xml".to_string(), core.into_bytes()),
        ("docProps/app.xml".to_string(), app.into_bytes()),
        (
            "ppt/presentation.xml".to_string(),
            presentation.into_bytes(),
        ),
        (
            "ppt/_rels/presentation.xml.rels".to_string(),
            pres_rels.into_bytes(),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml".to_string(),
            s(parts::SLIDE_MASTER),
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".to_string(),
            s(parts::SLIDE_MASTER_RELS),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml".to_string(),
            s(parts::SLIDE_LAYOUT),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".to_string(),
            s(parts::SLIDE_LAYOUT_RELS),
        ),
        ("ppt/theme/theme1.xml".to_string(), s(parts::THEME)),
    ]);
    Ok(PptxPackage { parts: out })
}

/// `exportPptx`: the .pptx bytes.
pub async fn export_pptx(
    r: &dyn Renderer,
    host: &dyn RenderHost,
    doc: &DesignDoc,
    page_ids: &[String],
    deps: &dyn PptxDeps,
) -> Result<Vec<u8>, PptxError> {
    pptx_package(r, host, doc, page_ids, deps, chrono::Utc::now())
        .await?
        .to_zip()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions() {
        assert_eq!(
            pptx_transition("wipe", Some("right")),
            "<p:wipe dir=\"l\"/>"
        );
        assert_eq!(
            pptx_transition("split", Some("up")),
            "<p:split orient=\"horz\" dir=\"out\"/>"
        );
        assert_eq!(
            pptx_transition("push", Some("sideways")),
            "<p:push dir=\"l\"/>"
        );
        assert_eq!(pptx_transition("morph", None), "<p:fade/>");
    }
}
