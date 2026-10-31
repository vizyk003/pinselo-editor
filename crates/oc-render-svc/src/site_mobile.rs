// SPDX-License-Identifier: Apache-2.0
//! Websites: the phone layout (`<div class="m">`) and the semantic text mirror for one page — port of
//! `services/render/src/siteMobile.ts`. The plan comes from [`crate::site_reflow`]; this turns it into HTML/CSS
//! with real text, images, links, forms, videos and embeds. Visuals HTML can't express (shapes, charts, filtered
//! photos…) are cut out of the page's display list as small inline SVGs ([`MobileCtx::svg_of`], the renderer's
//! job).
//!
//! Deviation: the TS renders the nodes of a row concurrently (`Promise.all`), so the order in which `svgOf`
//! calls finish — and thus the `s<page>c<n>` id prefixes the site gives each cut-out — depends on promise
//! timing; here nodes render in reading order. The prefixes only need to be unique per page.

use futures::future::BoxFuture;
use oc_schema::DesignDoc;
use oc_schema::color::to_rgba;
use oc_schema::elements::{Element, FormEl, SemanticRole, TextEl};
use oc_schema::embeds::EMBED_FRAME_HOSTS;
use oc_schema::js;
use oc_schema::primitives::{ColorValue, LinkTarget, Paint, Rect};
use oc_schema::text::{
    CaseTransform, ListKind, Paragraph, TextEffect, body_plain_text, resolve_run_style,
};
use oc_schema::tree::children_of;

use crate::renderer::RenderError;
use crate::site_reflow::{ReflowItem, ReflowNode, mobile_font_size, plan_reflow, reading_order};

fn n(v: f64) -> String {
    js::num_to_string(v)
}

/// HTML escape (`&<>"`).
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `rgba(r,g,b,a)` with 0..255 channels and alpha to 3 decimals.
pub fn css_color(c: &ColorValue) -> String {
    let x = to_rgba(c);
    format!(
        "rgba({},{},{},{})",
        n(js::round(x.r * 255.0)),
        n(js::round(x.g * 255.0)),
        n(js::round(x.b * 255.0)),
        n(js::to_fixed_num(x.a, 3))
    )
}

fn r1(v: f64) -> f64 {
    js::round(v * 10.0) / 10.0
}

/// CSS background for a paint (`None` for no paint).
pub fn paint_css(p: Option<&Paint>, asset_url: &dyn Fn(&str) -> String) -> Option<String> {
    let p = p?;
    let stops = |s: &[oc_schema::primitives::GradientStop]| {
        s.iter()
            .map(|s| format!("{} {}%", css_color(&s.color), n(r1(s.t * 100.0))))
            .collect::<Vec<_>>()
            .join(",")
    };
    Some(match p {
        Paint::Solid(s) => css_color(&s.color),
        Paint::Linear(l) => format!(
            "linear-gradient({}deg,{})",
            n(r1(l.angle + 90.0)),
            stops(&l.stops)
        ),
        Paint::Radial(r) => format!("radial-gradient(circle,{})", stops(&r.stops)),
        Paint::Image(i) => format!(
            "url(\"{}\") center/cover no-repeat",
            esc(&asset_url(&i.asset))
        ),
    })
}

fn first_color(p: Option<&Paint>) -> Option<&ColorValue> {
    match p? {
        Paint::Solid(s) => Some(&s.color),
        Paint::Linear(l) => l.stops.first().map(|s| &s.color),
        Paint::Radial(r) => r.stops.first().map(|s| &s.color),
        Paint::Image(_) => None,
    }
}

/// Semantic tag of a text element on the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTag {
    H1,
    H2,
    H3,
    P,
    Blockquote,
}

impl TextTag {
    pub fn as_str(self) -> &'static str {
        match self {
            TextTag::H1 => "h1",
            TextTag::H2 => "h2",
            TextTag::H3 => "h3",
            TextTag::P => "p",
            TextTag::Blockquote => "blockquote",
        }
    }
}

/// Fit of an SVG cut-out in its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Meet,
    Slice,
}

impl Fit {
    pub fn as_str(self) -> &'static str {
        match self {
            Fit::Meet => "meet",
            Fit::Slice => "slice",
        }
    }
}

/// A page element as a reflow item.
#[derive(Debug, Clone)]
pub struct PageItem<'a> {
    pub id: String,
    pub bx: Rect,
    pub z: f64,
    pub visual: bool,
    pub container: bool,
    pub el: &'a Element,
}

impl ReflowItem for PageItem<'_> {
    fn id(&self) -> &str {
        &self.id
    }
    fn bx(&self) -> Rect {
        self.bx
    }
    fn z(&self) -> f64 {
        self.z
    }
    fn container(&self) -> bool {
        self.container
    }
    fn visual(&self) -> bool {
        self.visual
    }
}

/// `MobileCtx`: what the phone layout needs from the site builder.
pub trait MobileCtx: Send + Sync {
    fn doc(&self) -> &DesignDoc;
    fn page_w(&self) -> f64;
    fn page_h(&self) -> f64;
    fn design_id(&self) -> &str;
    fn asset_url(&self, id: &str) -> String;
    /// Canonical CSS family name for a run (as declared in `@font-face`).
    fn family(&self, family: &str, weight: f64, italic: bool) -> String;
    /// Inline SVG of the given elements, cropped to `bx`.
    fn svg_of<'a>(
        &'a self,
        ids: Vec<String>,
        bx: Rect,
        fit: Fit,
    ) -> BoxFuture<'a, Result<String, RenderError>>;
    /// Anchor for a page link (`#p2`).
    fn page_href(&self, page_id: &str) -> String;
    fn tag_of(&self, el: &TextEl) -> TextTag;
}

fn base(el: &Element) -> Option<&oc_schema::ElementBase> {
    el.base()
}

fn link_of(el: &Element) -> Option<&LinkTarget> {
    base(el).and_then(|b| b.link.as_ref())
}

fn alt_of(el: &Element) -> Option<&str> {
    base(el).and_then(|b| b.alt_text.as_deref())
}

fn decorative(el: &Element) -> bool {
    base(el).and_then(|b| b.decorative).unwrap_or(false)
}

fn hidden(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") == Some(&serde_json::Value::Bool(false)),
        _ => base(el).is_some_and(|b| b.visible == Some(false)),
    }
}

/// Largest run size in a text element.
pub fn max_run_size(el: &TextEl) -> f64 {
    let mut m = el.body.default_run.size;
    for p in &el.body.paragraphs {
        for r in &p.runs {
            m = m.max(resolve_run_style(&el.body, r).size);
        }
    }
    m
}

/// Semantic text role: explicit role, else inferred from type size relative to the page width.
pub fn text_kind(el: &TextEl, page_w: f64) -> &'static str {
    match el.base.role.as_ref().map(|r| r.as_str()) {
        Some("headline") => "headline",
        Some("subhead") => "subhead",
        Some("quote") => "quote",
        // `if (r) return 'body'`: an empty role counts as none.
        Some(r) if !r.is_empty() => "body",
        _ => {
            if max_run_size(el) >= page_w * 0.035 {
                "headline"
            } else {
                "body"
            }
        }
    }
}

fn link_href(l: &LinkTarget, ctx: &dyn MobileCtx) -> String {
    match l {
        LinkTarget::Url(u) => u.url.clone(),
        LinkTarget::Page(p) => ctx.page_href(&p.page_id),
    }
}

fn link_attrs(l: &LinkTarget, ctx: &dyn MobileCtx) -> String {
    let ext = if matches!(l, LinkTarget::Url(_)) {
        r#" target="_blank" rel="noopener""#
    } else {
        ""
    };
    format!("href=\"{}\"{ext}", esc(&link_href(l, ctx)))
}

// ------------------------------------------------------------------ text

fn runs_html(el: &TextEl, p: &Paragraph, ctx: &dyn MobileCtx, in_link: bool) -> String {
    if p.runs.is_empty() || p.runs.iter().all(|r| r.text.is_empty()) {
        return "<br>".into();
    }
    p.runs
        .iter()
        .map(|run| {
            let st = resolve_run_style(&el.body, run);
            let col = first_color(Some(&st.fill));
            let underline = st.underline.unwrap_or(false);
            let strike = st.strike.unwrap_or(false);
            let mut css: Vec<String> = vec![
                format!(
                    "font-family:'{}',system-ui,sans-serif",
                    esc(&ctx.family(&st.family, st.weight, st.italic))
                ),
                format!("font-weight:{}", n(st.weight)),
            ];
            if st.italic {
                css.push("font-style:italic".into());
            }
            css.push(format!(
                "font-size:{}px",
                n(mobile_font_size(st.size, ctx.page_w()))
            ));
            if let Some(c) = col {
                css.push(format!("color:{}", css_color(c)));
            }
            if st.tracking != 0.0 && !st.tracking.is_nan() {
                css.push(format!(
                    "letter-spacing:{}em",
                    n(r1(st.tracking / 1000.0 * 100.0) / 100.0)
                ));
            }
            if underline || strike {
                let parts = [
                    if underline { "underline" } else { "" },
                    if strike { "line-through" } else { "" },
                ]
                .join(" ");
                css.push(format!("text-decoration:{}", js::trim(&parts)));
            }
            if let Some(ct) = &st.case_transform
                && *ct != CaseTransform::None
            {
                let v = match ct {
                    CaseTransform::Upper => "uppercase",
                    CaseTransform::Lower => "lowercase",
                    _ => "capitalize",
                };
                css.push(format!("text-transform:{v}"));
            }
            if let Some(h) = &st.highlight {
                css.push(format!("background:{}", css_color(h)));
            }
            let span = format!(
                "<span style=\"{}\">{}</span>",
                css.join(";"),
                esc(&run.text).replace('\n', "<br>")
            );
            match &run.link {
                Some(l) if !in_link => format!("<a {}>{span}</a>", link_attrs(l, ctx)),
                _ => span,
            }
        })
        .collect()
}

/// A text element as HTML (`<h1 class="tx">`, `<div class="tx"><p>`, lists as `<ul>`/`<ol>`).
pub fn text_html(el: &TextEl, ctx: &dyn MobileCtx, in_link: bool) -> String {
    let tag = ctx.tag_of(el);
    let mut bx: Vec<String> = vec![format!("opacity:{}", n(el.base.opacity))];
    match &el.effect {
        Some(TextEffect::Background(fx)) => {
            bx.push(format!("background:{}", css_color(&fx.color)));
            bx.push(format!("padding:{}px", n(r1((fx.padding * 0.5).max(6.0)))));
            bx.push(format!("border-radius:{}px", n(r1(fx.radius * 0.5))));
        }
        Some(TextEffect::Shadow(fx)) => {
            bx.push(format!(
                "text-shadow:{}px {}px {}px {}",
                n(r1(fx.dx * 0.5)),
                n(r1(fx.dy * 0.5)),
                n(r1(fx.blur * 0.5)),
                css_color(&fx.color)
            ));
        }
        _ => {}
    }
    let p_style = |p: &Paragraph| {
        let mb = if p.space_after != 0.0 && !p.space_after.is_nan() {
            format!(";margin-bottom:{}px", n(r1(24f64.min(p.space_after * 0.5))))
        } else {
            String::new()
        };
        format!(
            "text-align:{};line-height:{}{mb}",
            p.align.as_str(),
            n(p.line_height)
        )
    };
    let mut parts: Vec<String> = Vec::new();
    let paras = &el.body.paragraphs;
    let mut i = 0;
    while i < paras.len() {
        let p = &paras[i];
        if let Some(list) = &p.list {
            let kind = list.kind.clone();
            let mut items: Vec<String> = Vec::new();
            while i < paras.len() && paras[i].list.as_ref().map(|l| &l.kind) == Some(&kind) {
                items.push(format!(
                    "<li style=\"{}\">{}</li>",
                    p_style(&paras[i]),
                    runs_html(el, &paras[i], ctx, in_link)
                ));
                i += 1;
            }
            let t = if kind == ListKind::Number { "ol" } else { "ul" };
            parts.push(format!("<{t}>{}</{t}>", items.join("")));
            continue;
        }
        if tag == TextTag::P {
            parts.push(format!(
                "<p style=\"{}\">{}</p>",
                p_style(p),
                runs_html(el, p, ctx, in_link)
            ));
        } else {
            parts.push(format!(
                "<span class=\"pl\" style=\"{}\">{}</span>",
                p_style(p),
                runs_html(el, p, ctx, in_link)
            ));
        }
        i += 1;
    }
    let wrap = if tag == TextTag::P {
        "div"
    } else {
        tag.as_str()
    };
    format!(
        "<{wrap} class=\"tx\" style=\"{}\">{}</{wrap}>",
        bx.join(";"),
        parts.join("")
    )
}

/// Visually hidden, reading-order text of a page (headings, paragraphs, image descriptions).
pub fn mirror_html(items: &[&PageItem<'_>], ctx: &dyn MobileCtx) -> String {
    let mut out: Vec<String> = Vec::new();
    for it in items {
        match it.el {
            Element::Text(t) => {
                let text = body_plain_text(&t.body);
                let text = js::trim(&text);
                if text.is_empty() {
                    continue;
                }
                let tag = ctx.tag_of(t);
                let sep = if tag == TextTag::P { "<br>" } else { " " };
                out.push(format!(
                    "<{0}>{1}</{0}>",
                    tag.as_str(),
                    esc(text).replace('\n', sep)
                ));
            }
            Element::Image(_) | Element::Frame(_) | Element::Grid(_) | Element::Chart(_) => {
                if let Some(alt) = alt_of(it.el).filter(|a| !a.is_empty())
                    && !decorative(it.el)
                {
                    out.push(format!(
                        "<span role=\"img\" aria-label=\"{}\"></span>",
                        esc(alt)
                    ));
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        String::new()
    } else {
        format!("<div class=\"sr\">{}</div>", out.join(""))
    }
}

// ------------------------------------------------------------------ elements → reflow items

/// Elements HTML handles natively (their children must stay separate blocks on mobile).
fn is_interactive(e: &Element) -> bool {
    matches!(
        e,
        Element::Text(_) | Element::Form(_) | Element::Video(_) | Element::Embed(_)
    ) || link_of(e).is_some()
}

/// Leaf items of a page in paint order. Groups of pure graphics stay one visual; groups holding text, links or
/// live media are opened up.
pub fn page_items<'a>(doc: &'a DesignDoc, page_id: &str) -> Vec<PageItem<'a>> {
    fn has_interactive(doc: &DesignDoc, id: &str) -> bool {
        children_of(&doc.elements, id).into_iter().any(|c| {
            is_interactive(c)
                || (matches!(c, Element::Group(_)) && has_interactive(doc, c.id().unwrap_or("")))
        })
    }
    fn visit<'a>(doc: &'a DesignDoc, pid: &str, out: &mut Vec<PageItem<'a>>) {
        for el in children_of(&doc.elements, pid) {
            if hidden(el) || el.is_deleted() {
                continue;
            }
            let id = el.id().unwrap_or("");
            if matches!(el, Element::Group(_)) && has_interactive(doc, id) {
                visit(doc, id, out);
                continue;
            }
            let bx = crate::geom::world_bounds(&doc.elements, id);
            let visual = !matches!(el, Element::Text(_) | Element::Form(_));
            let container = matches!(
                el,
                Element::Shape(_) | Element::Image(_) | Element::Frame(_)
            );
            let z = out.len() as f64;
            out.push(PageItem {
                id: id.to_string(),
                bx,
                z,
                visual,
                container,
                el,
            });
        }
    }
    let mut out = Vec::new();
    visit(doc, page_id, &mut out);
    out
}

// ------------------------------------------------------------------ mobile layout

const CSS_SHAPES: [&str; 5] = ["rect", "squircle", "pill", "ellipse", "round-one"];

type Node<'n, 'a> = ReflowNode<'n, PageItem<'a>>;

/// Everything on top of a container is plain graphics (no text, links or live media)?
fn graphics_only(node: &Node<'_, '_>) -> bool {
    node.rows.iter().all(|row| {
        row.iter().all(|n| {
            n.item.visual
                && link_of(n.item.el).is_none()
                && !matches!(n.item.el, Element::Video(_) | Element::Embed(_))
                && graphics_only(n)
        })
    })
}

fn subtree_ids(node: &Node<'_, '_>) -> Vec<String> {
    let mut v = vec![node.item.id.clone()];
    for row in &node.rows {
        for k in row {
            v.extend(subtree_ids(k));
        }
    }
    v
}

fn embed_hosted(src: &str) -> bool {
    EMBED_FRAME_HOSTS
        .iter()
        .any(|h| src.starts_with(&format!("{h}/")))
}

/// The sandboxed player markup shared by the overlay and the phone layout.
pub fn embed_iframe(class: &str, style: &str, src: &str, title: &str) -> String {
    format!(
        "<iframe class=\"{class}\" style=\"{style}\" src=\"{}\" title=\"{}\" loading=\"lazy\" referrerpolicy=\"strict-origin-when-cross-origin\" allow=\"encrypted-media; picture-in-picture; fullscreen; clipboard-write\" allowfullscreen sandbox=\"allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox\"></iframe>",
        esc(src),
        esc(title)
    )
}

async fn media_html(node: &Node<'_, '_>, ctx: &dyn MobileCtx) -> Result<String, RenderError> {
    let (el, bx) = (node.item.el, node.item.bx);
    let composite = !node.rows.is_empty();
    let frac = bx.w / ctx.page_w();
    let width = if frac >= 0.4 {
        100.0
    } else {
        100f64.min(12f64.max(frac * 220.0))
    };
    let ar = format!("aspect-ratio:{}/{}", n(r1(bx.w)), n(r1(bx.h)));
    let size = format!("width:{}%;{ar}", n(r1(width)));
    if let Element::Video(v) = el {
        let gif = v.clip.loop_ && v.clip.muted;
        let src = ctx.asset_url(&v.asset);
        let label = alt_of(el)
            .filter(|a| !a.is_empty())
            .map_or_else(String::new, |a| format!(" aria-label=\"{}\"", esc(a)));
        return Ok(format!(
            "<video class=\"mi mv\" style=\"{size}\" src=\"{}\" poster=\"{}\" playsinline preload=\"metadata\"{}{label}></video>",
            esc(&src),
            esc(&format!("{src}/poster")),
            if gif {
                " autoplay muted loop"
            } else {
                " controls"
            }
        ));
    }
    if let Element::Embed(e) = el
        && embed_hosted(&e.src)
    {
        let title = if e.title.is_empty() {
            e.provider.as_str().to_string()
        } else {
            e.title.clone()
        };
        return Ok(embed_iframe(
            "mi me",
            &format!("width:100%;{ar}"),
            &e.src,
            &title,
        ));
    }
    let alt = if decorative(el) {
        ""
    } else {
        alt_of(el).unwrap_or("")
    };
    if !composite
        && let Element::Image(img) = el
        && img.edits.is_none()
        && img.svg_colors.is_none()
        && (img.base.xf.rot == 0.0 || img.base.xf.rot.is_nan())
        && img.base.effects.as_ref().is_none_or(Vec::is_empty)
    {
        let c = &img.crop;
        let ox = if c.w < 0.999 {
            (c.x / (1.0 - c.w)) * 100.0
        } else {
            50.0
        };
        let oy = if c.h < 0.999 {
            (c.y / (1.0 - c.h)) * 100.0
        } else {
            50.0
        };
        let radius = match img.corner_radius {
            Some(r) if r != 0.0 && !r.is_nan() => {
                format!(";border-radius:{}%", n(r1((r / bx.w.min(bx.h)) * 100.0)))
            }
            _ => String::new(),
        };
        return Ok(format!(
            "<img class=\"mi\" style=\"{size};object-fit:cover;object-position:{}% {}%;opacity:{}{radius}\" src=\"{}\" alt=\"{}\" loading=\"lazy\" decoding=\"async\">",
            n(r1(ox)),
            n(r1(oy)),
            n(img.base.opacity),
            esc(&ctx.asset_url(&img.asset)),
            esc(alt)
        ));
    }
    let svg = ctx.svg_of(subtree_ids(node), bx, Fit::Meet).await?;
    let role = if alt.is_empty() {
        " aria-hidden=\"true\"".to_string()
    } else {
        format!(" role=\"img\" aria-label=\"{}\"", esc(alt))
    };
    Ok(format!(
        "<div class=\"mi\"{role} style=\"{size}\">{svg}</div>"
    ))
}

fn form_mobile_html(el: &FormEl, ctx: &dyn MobileCtx) -> String {
    let st = &el.style;
    let field = format!(
        "background:{};border:1px solid {};border-radius:{}px",
        css_color(&st.field_fill),
        css_color(&st.border_color),
        n(16f64.min(st.radius))
    );
    let parts: Vec<String> = el
        .fields
        .iter()
        .map(|f| {
            let req = if f.required.unwrap_or(false) { " required" } else { "" };
            let ph = f.placeholder.as_deref().filter(|p| !p.is_empty()).map_or_else(String::new, |p| format!(" placeholder=\"{}\"", esc(p)));
            let kind = f.kind.as_str();
            if kind == "choice" || kind == "checkboxes" {
                let ty = if kind == "checkboxes" { "checkbox" } else { "radio" };
                let opts: String = f
                    .options
                    .iter()
                    .flatten()
                    .map(|o| format!("<label class=\"op\"><input type=\"{ty}\" name=\"{}\" value=\"{}\" style=\"accent-color:{}\"> {}</label>", esc(&f.id), esc(o), css_color(&st.accent), esc(o)))
                    .collect();
                return format!("<fieldset><legend>{}</legend>{opts}</fieldset>", esc(&f.label));
            }
            if kind == "longtext" {
                return format!("<label>{}<textarea name=\"{}\" rows=\"4\" style=\"{field}\"{req}{ph}></textarea></label>", esc(&f.label), esc(&f.id));
            }
            let ty = match kind {
                "email" => "email",
                "number" => "number",
                "date" => "date",
                _ => "text",
            };
            format!("<label>{}<input name=\"{}\" type=\"{ty}\" style=\"{field}\"{req}{ph}></label>", esc(&f.label), esc(&f.id))
        })
        .collect();
    let title = if el.title.is_empty() {
        "Form"
    } else {
        &el.title
    };
    let head = if el.title.is_empty() {
        String::new()
    } else {
        format!("<b class=\"ft\">{}</b>", esc(&el.title))
    };
    format!(
        "<form class=\"mf\" action=\"/api/forms/{}/{}\" method=\"post\" aria-label=\"{}\" style=\"font-family:'{}',system-ui,sans-serif;color:{}\">{head}{}<button type=\"submit\" style=\"background:{};border-radius:{}px\">{}</button><input name=\"_hp\" tabindex=\"-1\" autocomplete=\"off\" class=\"hp\" aria-hidden=\"true\"></form>",
        esc(ctx.design_id()),
        esc(&el.base.id),
        esc(title),
        esc(&ctx.family(&st.family, 400.0, false)),
        css_color(&st.color),
        parts.join(""),
        css_color(&st.accent),
        n(16f64.min(st.radius)),
        esc(&el.submit_label)
    )
}

fn container_html<'f>(
    node: &'f Node<'_, '_>,
    ctx: &'f dyn MobileCtx,
) -> BoxFuture<'f, Result<String, RenderError>> {
    Box::pin(async move {
        let (el, bx) = (node.item.el, node.item.bx);
        let link = link_of(el);
        let linked = link.is_some();
        let content = rows_html(&node.rows, ctx, linked).await?;
        let mut bg = String::new();
        let mut style: Vec<String> = Vec::new();
        let css_shape = match el {
            Element::Shape(s) => {
                CSS_SHAPES.contains(&s.kind.as_str())
                    && (s.base.xf.rot == 0.0 || s.base.xf.rot.is_nan())
                    && s.fills.iter().all(|f| !matches!(f, Paint::Image(_)))
                    && s.strokes.len() <= 1
            }
            _ => false,
        };
        if let (true, Element::Shape(s)) = (css_shape, el) {
            let asset_url = |id: &str| ctx.asset_url(id);
            if let Some(fill) = paint_css(s.fills.last(), &asset_url) {
                style.push(format!("background:{fill}"));
            }
            if let Some(st) = s.strokes.first()
                && let Some(sc) = first_color(Some(&st.paint))
                && st.width > 0.0
            {
                style.push(format!(
                    "border:{}px solid {}",
                    n(r1((st.width * 0.5).max(1.0))),
                    css_color(sc)
                ));
            }
            let kind = s.kind.as_str();
            let cr = if kind == "pill" {
                999.0
            } else if kind == "ellipse" {
                -1.0
            } else {
                s.params
                    .get("cornerRadius")
                    .copied()
                    .unwrap_or(if kind == "squircle" {
                        bx.w.min(bx.h) * 0.25
                    } else {
                        0.0
                    })
            };
            if cr == -1.0 {
                style.push("border-radius:50%".into());
                style.push(format!("aspect-ratio:{}/{}", n(r1(bx.w)), n(r1(bx.h))));
            } else if cr != 0.0 && !cr.is_nan() {
                style.push(format!("border-radius:{}px", n(r1(999f64.min(cr * 0.6)))));
            }
        } else {
            bg = format!(
                "<div class=\"bkb\" aria-hidden=\"true\">{}</div>",
                ctx.svg_of(vec![node.item.id.clone()], bx, Fit::Slice)
                    .await?
            );
            // Photos and artwork keep (at least) their proportions; content may make them taller.
            if bx.w / ctx.page_w() >= 0.4 || !matches!(el, Element::Shape(_)) {
                style.push(format!("aspect-ratio:{}/{}", n(r1(bx.w)), n(r1(bx.h))));
            }
        }
        let small = bx.h < ctx.page_h() * 0.2;
        let tag = if linked { "a" } else { "div" };
        let attrs = link.map_or_else(String::new, |l| format!(" {}", link_attrs(l, ctx)));
        let label = match alt_of(el) {
            Some(a) if linked && !a.is_empty() => format!(" aria-label=\"{}\"", esc(a)),
            _ => String::new(),
        };
        Ok(format!(
            "<{tag} class=\"bk{}{}\"{attrs}{label} style=\"{}\">{bg}<div class=\"bkc\">{content}</div></{tag}>",
            if linked { " btn" } else { "" },
            if small { " sm" } else { "" },
            style.join(";")
        ))
    })
}

fn node_html<'f>(
    node: &'f Node<'_, '_>,
    ctx: &'f dyn MobileCtx,
    in_link: bool,
) -> BoxFuture<'f, Result<String, RenderError>> {
    Box::pin(async move {
        let el = node.item.el;
        if !node.rows.is_empty() && !graphics_only(node) {
            return container_html(node, ctx).await;
        }
        let link = link_of(el);
        let html = match el {
            Element::Text(t) => text_html(t, ctx, in_link || link.is_some()),
            Element::Form(f) => return Ok(form_mobile_html(f, ctx)),
            _ => media_html(node, ctx).await?,
        };
        if let Some(l) = link
            && !in_link
        {
            let label = match alt_of(el) {
                Some(a) if !matches!(el, Element::Text(_)) && !a.is_empty() => {
                    format!(" aria-label=\"{}\"", esc(a))
                }
                _ => String::new(),
            };
            return Ok(format!(
                "<a class=\"ml\" {}{label}>{html}</a>",
                link_attrs(l, ctx)
            ));
        }
        Ok(html)
    })
}

/// Is a decorative bit (ornament, divider) that only clutters a phone layout?
fn skip_on_mobile(node: &Node<'_, '_>, ctx: &dyn MobileCtx) -> bool {
    let (el, bx) = (node.item.el, node.item.bx);
    if (!node.rows.is_empty() && !graphics_only(node)) || link_of(el).is_some() {
        return false;
    }
    if matches!(el, Element::Line(_) | Element::Draw(_)) {
        return true;
    }
    // Mostly off the page (bleed).
    let (w, h) = (ctx.page_w(), ctx.page_h());
    let vis = ((bx.x + bx.w).min(w) - bx.x.max(0.0)).max(0.0)
        * ((bx.y + bx.h).min(h) - bx.y.max(0.0)).max(0.0);
    if vis < bx.w * bx.h * 0.3 {
        return true;
    }
    let role = base(el).and_then(|b| b.role.as_ref());
    let deco = decorative(el)
        || matches!(
            role,
            Some(SemanticRole::Decor) | Some(SemanticRole::Background)
        );
    deco && bx.w * bx.h < w * h * 0.1
}

fn rows_html<'f>(
    rows: &'f [Vec<Node<'_, '_>>],
    ctx: &'f dyn MobileCtx,
    in_link: bool,
) -> BoxFuture<'f, Result<String, RenderError>> {
    Box::pin(async move {
        let mut out: Vec<String> = Vec::new();
        for row in rows {
            let kept: Vec<&Node<'_, '_>> = row.iter().filter(|n| !skip_on_mobile(n, ctx)).collect();
            if kept.is_empty() {
                continue;
            }
            let pw = ctx.page_w();
            // A row of small graphics (icons, logos, social badges) stays a row.
            let inline_row = kept.len() > 1
                && kept.iter().all(|n| {
                    n.item.visual
                        && graphics_only(n)
                        && n.item.bx.w <= pw * 0.25
                        && !matches!(n.item.el, Element::Video(_) | Element::Embed(_))
                });
            // Short single-line texts side by side (name + price, a row of nav links) stay side by side.
            let text_row = !inline_row
                && kept.len() > 1
                && kept.len() <= 6
                && kept.iter().all(|n| match n.item.el {
                    Element::Text(t) => {
                        n.rows.is_empty()
                            && n.item.bx.h <= max_run_size(t) * 1.7
                            && n.item.bx.w <= pw * 0.5
                    }
                    _ => false,
                });
            let mut html = String::new();
            for n in kept {
                html.push_str(&node_html(n, ctx, in_link).await?);
            }
            out.push(if inline_row {
                format!("<div class=\"ri\">{html}</div>")
            } else if text_row {
                format!("<div class=\"rt\">{html}</div>")
            } else {
                html
            });
        }
        Ok(out.join(""))
    })
}

/// The phone layout of one page.
pub struct MobilePage<'a> {
    pub html: String,
    /// Reading order (for the text mirror).
    pub order: Vec<&'a PageItem<'a>>,
}

/// `mobilePage(items, ctx, pageBg)`.
pub async fn mobile_page<'a>(
    items: &'a [PageItem<'a>],
    ctx: &dyn MobileCtx,
    page_bg: Option<&Paint>,
) -> Result<MobilePage<'a>, RenderError> {
    let (w, h) = (ctx.page_w(), ctx.page_h());
    let plan = plan_reflow(items, w, h);
    let mut layers: Vec<String> = Vec::new();
    for b in &plan.background {
        layers.push(
            ctx.svg_of(
                vec![b.id.clone()],
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                },
                Fit::Slice,
            )
            .await?,
        );
    }
    let asset_url = |id: &str| ctx.asset_url(id);
    let bg = paint_css(page_bg, &asset_url);
    let body = rows_html(&plan.rows, ctx, false).await?;
    let html = format!(
        "<div class=\"m\"{}>{}<div class=\"mc\">{body}</div></div>",
        bg.map_or_else(String::new, |b| format!(" style=\"background:{b}\"")),
        if layers.is_empty() {
            String::new()
        } else {
            format!(
                "<div class=\"bgl\" aria-hidden=\"true\">{}</div>",
                layers.join("")
            )
        }
    );
    Ok(MobilePage {
        html,
        order: reading_order(&plan),
    })
}
