// SPDX-License-Identifier: Apache-2.0
//! Website builder (WS-001), port of `services/render/src/site.ts`: a design → one static, responsive HTML page.
//! - Each page is a `<section>` holding the page as inline SVG (vector, real selectable text) scaled to the
//!   viewport width; fonts are declared once and loaded by URL.
//! - An overlay layer adds what a picture can't do: links (to URLs or other sections), playing video and GIFs,
//!   and live forms that post responses back.
//! - Phone-width pages get a reflowed layout ([`crate::site_mobile`]); page titles become the navigation menu.
//!
//! Everything but the SVG pictures is computed here; the SVG comes from [`Renderer::page_svg`] /
//! [`Renderer::elements_svg`] and the font faces from [`Renderer::text_faces`] / [`Renderer::pick_face`].

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use oc_schema::DesignDoc;
use oc_schema::elements::{Element, FormEl, TextEl};
use oc_schema::factory::form_layout_of;
use oc_schema::js;
use oc_schema::primitives::{ColorValue, LinkTarget, Paint, Rect};
use oc_schema::text::{body_plain_text, resolve_run_style};
use oc_schema::tree::{descendants, sorted_pages};

use crate::renderer::{
    AssetHref, FaceRequest, PickedFace, RenderError, RenderHost, Renderer, SvgOptions,
};
use crate::site_mobile::{
    Fit, MobileCtx, PageItem, TextTag, embed_iframe, esc, mirror_html, mobile_page, page_items,
    paint_css, text_kind,
};
use crate::site_reflow::{plan_reflow, reading_order, should_reflow};

/// Give every id (and reference) in an inline SVG a prefix so several SVGs can share one HTML page.
pub fn scope_svg_ids(svg: &str, prefix: &str) -> String {
    svg.replace(" id=\"", &format!(" id=\"{prefix}"))
        .replace("url(#", &format!("url(#{prefix}"))
        .replace("href=\"#", &format!("href=\"#{prefix}"))
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut ws = false;
    for c in s.chars() {
        if js::is_js_space(c) {
            if !ws {
                out.push(' ');
            }
            ws = true;
        } else {
            out.push(c);
            ws = false;
        }
    }
    js::trim(&out).to_string()
}

/// First paragraph-like text of the site, trimmed for a meta description (lengths in UTF-16 units).
pub fn describe(texts: &[String]) -> Option<String> {
    let t = texts
        .iter()
        .map(|x| collapse_ws(x))
        .find(|x| js::utf16_len(x) >= 20)?;
    let u: Vec<u16> = t.encode_utf16().collect();
    if u.len() <= 160 {
        return Some(t);
    }
    let cut = &u[..157];
    let space = cut
        .iter()
        .rposition(|c| *c == u16::from(b' '))
        .map_or(-1, |i| i as i64);
    let end = space.max(120) as usize;
    Some(format!("{}…", String::from_utf16_lossy(&cut[..end])))
}

fn pct(v: f64, total: f64) -> String {
    format!("{}%", js::to_fixed((v / total) * 100.0, 4))
}

/// site.ts `css`: `rgba(r,g,b,a)` with the alpha as stored (siteMobile rounds it).
fn css(c: &ColorValue) -> String {
    let x = oc_schema::color::to_rgba(c);
    let n = js::num_to_string;
    format!(
        "rgba({},{},{},{})",
        n(js::round(x.r * 255.0)),
        n(js::round(x.g * 255.0)),
        n(js::round(x.b * 255.0)),
        n(x.a)
    )
}

/// The live form over a page picture (inputs positioned in % of the form box).
pub fn form_html(
    doc: &DesignDoc,
    el: &FormEl,
    page_w: f64,
    page_h: f64,
    design_id: &str,
) -> String {
    let m = crate::geom::world_matrix(&doc.elements, &el.base.id);
    let (ox, oy) = (m[4], m[5]);
    let layout = form_layout_of(el);
    let st = &el.style;
    let xf = &el.base.xf;
    let pos = |x: f64, y: f64, w: f64, h: f64| {
        format!(
            "left:{};top:{};width:{};height:{}",
            pct(x, xf.w),
            pct(y, xf.h),
            pct(w, xf.w),
            pct(h, xf.h)
        )
    };
    let mut parts: Vec<String> = Vec::new();
    for r in &layout.rows {
        let f = r
            .field_id
            .as_ref()
            .and_then(|id| el.fields.iter().find(|x| &x.id == id));
        match (r.kind, f) {
            ("input", Some(f)) => {
                let req = if f.required.unwrap_or(false) {
                    " required"
                } else {
                    ""
                };
                let ty = match f.kind.as_str() {
                    "email" => "email",
                    "number" => "number",
                    "date" => "date",
                    _ => "text",
                };
                parts.push(if f.kind.as_str() == "longtext" {
                    format!(
                        "<textarea name=\"{}\" aria-label=\"{}\" style=\"{}\"{req}></textarea>",
                        esc(&f.id),
                        esc(&f.label),
                        pos(r.x, r.y, r.w, r.h)
                    )
                } else {
                    format!(
                        "<input name=\"{}\" type=\"{ty}\" aria-label=\"{}\" style=\"{}\"{req}>",
                        esc(&f.id),
                        esc(&f.label),
                        pos(r.x, r.y, r.w, r.h)
                    )
                });
            }
            ("option", Some(f)) => {
                let multi = f.kind.as_str() == "checkboxes";
                let b = (st.size * 1.1) / xf.w;
                let opt = r.option.as_deref().unwrap_or("");
                parts.push(format!(
                    "<label style=\"{}\"><input type=\"{}\" name=\"{}\" value=\"{}\" aria-label=\"{}\"><span class=\"on\" style=\"left:{}%;top:25%;width:{}%;height:50%;max-width:{}px;border-radius:{};background:{}\"></span></label>",
                    pos(r.x, r.y, r.w, r.h),
                    if multi { "checkbox" } else { "radio" },
                    esc(&f.id),
                    esc(opt),
                    esc(opt),
                    js::to_fixed(b * 25.0, 3),
                    js::to_fixed(b * 50.0 * (xf.w / r.w), 3),
                    js::num_to_string(st.size * 0.55),
                    if multi { "2px" } else { "50%" },
                    css(&st.accent)
                ));
            }
            ("submit", _) => parts.push(format!(
                "<button type=\"submit\" style=\"{}\">{}</button>",
                pos(r.x, r.y, r.w, r.h),
                esc(&el.submit_label)
            )),
            _ => {}
        }
    }
    parts.push("<input name=\"_hp\" tabindex=\"-1\" autocomplete=\"off\" style=\"left:-9999px;top:0;width:1px;height:1px\" aria-hidden=\"true\">".into());
    format!(
        "<form class=\"fm\" action=\"/api/forms/{}/{}\" method=\"post\" aria-label=\"{}\" data-w=\"{}\" data-s=\"{}\" style=\"left:{};top:{};width:{};height:{};font-family:'{}',sans-serif;color:{}\"><div class=\"fs\"></div>{}</form>",
        esc(design_id),
        esc(&el.base.id),
        esc(if el.title.is_empty() {
            "Form"
        } else {
            &el.title
        }),
        js::num_to_string(xf.w),
        js::num_to_string(st.size),
        pct(ox, page_w),
        pct(oy, page_h),
        pct(xf.w, page_w),
        pct(xf.h, page_h),
        esc(&st.family),
        css(&st.color),
        parts.join("")
    )
}

/// Semantic tags across a site, decided in reading order: the first headline is the `<h1>`, later ones `<h2>`;
/// also the body texts (for the meta description).
pub fn text_tags(doc: &DesignDoc) -> (HashMap<String, TextTag>, Vec<String>) {
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    let mut tags = HashMap::new();
    let mut body = Vec::new();
    let mut h1 = false;
    for page in sorted_pages(&doc.pages)
        .into_iter()
        .filter(|p| !p.hidden.unwrap_or(false))
    {
        let items = page_items(doc, &page.id);
        let plan = plan_reflow(&items, w, h);
        for it in reading_order(&plan) {
            let Element::Text(t) = it.el else { continue };
            let tag = match text_kind(t, w) {
                "headline" => {
                    if h1 {
                        TextTag::H2
                    } else {
                        TextTag::H1
                    }
                }
                "subhead" => TextTag::H3,
                "quote" => TextTag::Blockquote,
                _ => TextTag::P,
            };
            if tag == TextTag::H1 {
                h1 = true;
            }
            if tag == TextTag::P {
                body.push(body_plain_text(&t.body));
            }
            tags.insert(t.base.id.clone(), tag);
        }
    }
    (tags, body)
}

#[derive(Clone)]
pub struct SiteOptions {
    pub design_id: String,
    pub title: String,
    pub description: Option<String>,
    pub noindex: bool,
    /// Public URL of an asset (images, video proxies, posters).
    pub asset_url: AssetHref,
    /// Social preview image URL.
    pub og_image: Option<String>,
    /// Show a small "Made with" link.
    pub badge: Option<(String, String)>,
}

struct Ctx<'a> {
    doc: &'a DesignDoc,
    w: f64,
    h: f64,
    opts: &'a SiteOptions,
    pages: Vec<String>,
    tags: &'a HashMap<String, TextTag>,
    families: HashMap<String, String>,
    renderer: &'a dyn Renderer,
    host: &'a dyn RenderHost,
    page_id: String,
    page_no: usize,
    cut: Mutex<usize>,
    svg_opts: SvgOptions,
}

fn face_key(family: &str, weight: f64, italic: bool) -> String {
    format!("{family}|{}|{italic}", js::num_to_string(weight))
}

impl MobileCtx for Ctx<'_> {
    fn doc(&self) -> &DesignDoc {
        self.doc
    }
    fn page_w(&self) -> f64 {
        self.w
    }
    fn page_h(&self) -> f64 {
        self.h
    }
    fn design_id(&self) -> &str {
        &self.opts.design_id
    }
    fn asset_url(&self, id: &str) -> String {
        (self.opts.asset_url)(id)
    }
    fn family(&self, family: &str, weight: f64, italic: bool) -> String {
        self.families
            .get(&face_key(family, weight, italic))
            .cloned()
            .unwrap_or_else(|| family.to_string())
    }
    fn svg_of<'b>(
        &'b self,
        ids: Vec<String>,
        bx: Rect,
        fit: Fit,
    ) -> BoxFuture<'b, Result<String, RenderError>> {
        Box::pin(async move {
            let mut set = BTreeSet::new();
            for id in &ids {
                set.insert(id.clone());
                for d in descendants(&self.doc.elements, id) {
                    if let Some(i) = d.id() {
                        set.insert(i.to_string());
                    }
                }
            }
            let out = self
                .renderer
                .elements_svg(self.host, self.doc, &self.page_id, &set, &self.svg_opts)
                .await?;
            let n = js::num_to_string;
            let head = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" preserveAspectRatio=\"xMidYMid {}\" focusable=\"false\"{}>",
                n(bx.x),
                n(bx.y),
                n(bx.w),
                n(bx.h),
                fit.as_str(),
                if fit == Fit::Meet {
                    " style=\"overflow:visible\""
                } else {
                    ""
                }
            );
            let k = {
                let mut c = self.cut.lock().expect("cut counter");
                *c += 1;
                *c
            };
            Ok(scope_svg_ids(
                &replace_svg_head(&out, &head),
                &format!("s{}c{k}", self.page_no),
            ))
        })
    }
    fn page_href(&self, page_id: &str) -> String {
        format!(
            "#p{}",
            self.pages
                .iter()
                .position(|p| p == page_id)
                .map_or(0, |i| i + 1)
        )
    }
    fn tag_of(&self, el: &TextEl) -> TextTag {
        self.tags.get(&el.base.id).copied().unwrap_or(TextTag::P)
    }
}

/// `out.replace(/<svg [^>]*>/, head)`.
fn replace_svg_head(svg: &str, head: &str) -> String {
    match svg.find("<svg ") {
        Some(i) => match svg[i..].find('>') {
            Some(j) => format!("{}{head}{}", &svg[..i], &svg[i + j + 1..]),
            None => svg.to_string(),
        },
        None => svg.to_string(),
    }
}

/// `(family, weight, italic)` of every run the phone layout may render (text runs, form labels).
fn families_used(items: &[PageItem<'_>]) -> Vec<FaceRequest> {
    let mut out: Vec<FaceRequest> = Vec::new();
    let mut add = |family: &str, weight: f64, italic: bool| {
        if !out
            .iter()
            .any(|f| f.family == family && f.weight == weight && f.italic == italic)
        {
            out.push(FaceRequest {
                family: family.to_string(),
                weight,
                italic,
            });
        }
    };
    for it in items {
        match it.el {
            Element::Text(t) => {
                for p in &t.body.paragraphs {
                    for r in &p.runs {
                        let st = resolve_run_style(&t.body, r);
                        add(&st.family, st.weight, st.italic);
                    }
                }
            }
            Element::Form(f) => add(&f.style.family, 400.0, false),
            _ => {}
        }
    }
    out
}

const STYLE: &str = r#"*{box-sizing:border-box}html{scroll-behavior:smooth}body{margin:0;background:#fff;font-family:system-ui,sans-serif}
nav.top{position:sticky;top:0;z-index:10;display:flex;gap:4px 18px;flex-wrap:wrap;justify-content:center;padding:12px 16px;background:rgba(255,255,255,.88);backdrop-filter:blur(10px);border-bottom:1px solid rgba(0,0,0,.06)}
nav.top a{color:#111;text-decoration:none;font-weight:600;font-size:15px}nav.top a:hover{text-decoration:underline}
section{width:100%}.pg{position:relative;max-width:{MAXW}px;margin:0 auto}.pg>svg{display:block;width:100%;height:auto}
.ov{position:absolute;inset:0}.ov>*{position:absolute}.lk{display:block}.em{border:0}.vd{object-fit:cover;background:#000}
.fm input,.fm textarea{position:absolute;background:transparent;border:0;outline:none;font:inherit;padding:0 .6em;color:inherit}.fm textarea{resize:none;padding:.5em .6em}
.fm input:focus-visible,.fm textarea:focus-visible{outline:2px solid #2563eb;outline-offset:-2px;border-radius:6px}.fm .bad{outline:2px solid #dc2626}
.fm label{position:absolute;cursor:pointer}.fm label input{opacity:0;width:1px;height:1px}.fm label .on{position:absolute;display:none}.fm label input:checked+.on{display:block}
.fm button{position:absolute;background:transparent;border:0;cursor:pointer;color:transparent}.fm .done,.mf .done{position:absolute;inset:0;display:grid;place-items:center;background:rgba(255,255,255,.95);font-weight:600;text-align:center;padding:16px}
.sr{position:absolute;width:1px;height:1px;margin:-1px;padding:0;overflow:hidden;clip:rect(0 0 0 0);clip-path:inset(50%);white-space:nowrap;border:0}
.m{display:none;position:relative;overflow:hidden;padding:40px 20px}.bgl{position:absolute;inset:0}.bgl>svg,.bkb>svg{position:absolute;inset:0;width:100%;height:100%;display:block}
.mc,.bkc{position:relative;display:flex;flex-direction:column;gap:16px}.bkc{gap:10px;padding:24px 20px;justify-content:center;min-height:100%}.bk.sm>.bkc{padding:12px 18px}
.bk{position:relative;display:block;overflow:hidden;color:inherit;text-decoration:none}.bkb{position:absolute;inset:0}.btn{transition:transform .15s}.btn:active{transform:scale(.98)}
.mi{display:block;margin:0 auto;max-width:100%;height:auto}.mi>svg{display:block;width:100%;height:100%}.me{border:0}.mv{background:#000;object-fit:cover}
.ml{display:block;color:inherit;text-decoration:none}.tx{margin:0;overflow-wrap:anywhere}.tx p,.tx ul,.tx ol{margin:0}.tx ul,.tx ol{padding-left:1.3em}.pl{display:block}
.rt{display:flex;flex-wrap:wrap;justify-content:space-between;align-items:baseline;gap:4px 16px}.rt>*{flex:0 1 auto}
.ri{display:flex;flex-wrap:wrap;justify-content:center;align-items:center;gap:12px}.ri>.mi,.ri>.ml{margin:0;flex:0 0 auto;width:auto;max-width:30%}.ri .mi{width:64px}
.mf{display:grid;gap:12px}.mf .ft{font-size:18px}.mf label{display:grid;gap:6px;font-size:16px}.mf input,.mf textarea{font:inherit;font-size:16px;padding:10px 12px;color:inherit;width:100%}
.mf fieldset{border:0;margin:0;padding:0;display:grid;gap:6px}.mf legend{font-size:16px;margin-bottom:4px}.mf .op{display:flex;gap:8px;align-items:center}.mf .op input{width:auto}
.mf button{font:inherit;font-size:16px;font-weight:600;color:#fff;border:0;padding:12px 16px;cursor:pointer}.mf .hp{position:absolute;left:-9999px;width:1px;height:1px}.mf .bad{outline:2px solid #dc2626}.mf{position:relative}
@media (max-width:699px){section.rf>.pg{display:none}section.rf>.m{display:block}}
.badge{position:fixed;right:12px;bottom:12px;background:#fff;border:1px solid #e5e7eb;border-radius:999px;padding:6px 12px;font:600 12px system-ui;color:#111;text-decoration:none;box-shadow:0 2px 8px rgba(0,0,0,.08)}"#;

const SCRIPT: &str = r#"<script>
document.querySelectorAll('form.fm,form.mf').forEach(function(f){
  if(f.classList.contains('fm')){var fs=f.querySelector('.fs');
  f.addEventListener('input',function(){fs.style.fontSize=(f.clientWidth/parseFloat(f.dataset.w)*parseFloat(f.dataset.s))+'px'});
  function size(){f.style.fontSize=(f.clientWidth/parseFloat(f.dataset.w)*parseFloat(f.dataset.s)*0.85)+'px'}size();addEventListener('resize',size)}
  f.addEventListener('submit',function(e){e.preventDefault();var d={};
    f.querySelectorAll('[name]').forEach(function(i){if(i.type==='checkbox'){if(i.checked)(d[i.name]=d[i.name]||[]).push(i.value)}else if(i.type==='radio'){if(i.checked)d[i.name]=i.value}else d[i.name]=i.value});
    fetch(f.action,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(d)}).then(function(r){return r.json().then(function(j){
      f.querySelectorAll('.bad').forEach(function(x){x.classList.remove('bad')});
      if(r.ok){var m=document.createElement('div');m.className='done';m.setAttribute('role','status');m.textContent=j.message||'Thanks!';f.appendChild(m)}
      else Object.keys(j.problems||{}).forEach(function(k){var x=f.querySelector('[name="'+k+'"]');if(x)x.classList.add('bad')})})})});
});
</script>"#;

fn is_hidden(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") == Some(&serde_json::Value::Bool(false)),
        _ => el.base().is_some_and(|b| b.visible == Some(false)),
    }
}

/// `buildSite(r, doc, opts)`.
pub async fn build_site(
    renderer: &dyn Renderer,
    host: &dyn RenderHost,
    doc: &DesignDoc,
    opts: &SiteOptions,
) -> Result<String, RenderError> {
    // SEC-01: no javascript:/data: links in the output, whatever the stored design holds.
    let safe = oc_schema::links::with_safe_links(doc);
    let doc: &DesignDoc = &safe;
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    let pages: Vec<&oc_schema::Page> = sorted_pages(&doc.pages)
        .into_iter()
        .filter(|p| !p.hidden.unwrap_or(false))
        .collect();
    let page_ids: Vec<String> = pages.iter().map(|p| p.id.to_string()).collect();
    let anchor = |i: Option<usize>| format!("p{}", i.map_or(0, |i| i + 1));
    let (tags, body_texts) = text_tags(doc);
    let description = opts
        .description
        .clone()
        .filter(|d| !d.is_empty())
        .or_else(|| describe(&body_texts));
    let mut font_faces: Vec<(String, String)> = Vec::new();
    let mut sections: Vec<String> = Vec::new();
    let mut nav: Vec<String> = Vec::new();
    let asset_url = opts.asset_url.clone();
    let href: AssetHref = Arc::new(move |id: &str| {
        let base = id.strip_suffix("/poster").unwrap_or(id);
        format!(
            "{}{}",
            asset_url(base),
            if id.ends_with("/poster") {
                "/poster"
            } else {
                ""
            }
        )
    });
    let svg_opts = SvgOptions {
        asset_href: Some(href),
        external_fonts: true,
    };

    for (i, page) in pages.iter().enumerate() {
        for f in renderer.text_faces(host, doc, &page.id).await? {
            let face: PickedFace = renderer.pick_face(host, &f).await?;
            let key = face_key(&face.family, face.weight, face.italic);
            if !font_faces.iter().any(|(k, _)| *k == key) {
                font_faces.push((
                    key,
                    format!(
                        "@font-face{{font-family:\"{}\";font-weight:{};font-style:{};font-display:swap;src:url(\"/api/fonts/files/{}\") format(\"truetype\")}}",
                        face.family,
                        js::num_to_string(face.weight),
                        if face.italic { "italic" } else { "normal" },
                        face.file
                    ),
                ));
            }
        }
        let svg = scope_svg_ids(
            &renderer.page_svg(host, doc, &page.id, &svg_opts).await?,
            &format!("s{}", i + 1),
        );
        let items = page_items(doc, &page.id);
        let reflow = should_reflow(page.mobile_layout.as_ref().map(|m| m.as_str()), w, h);
        let mut families = HashMap::new();
        if reflow {
            for f in families_used(&items) {
                let picked = renderer.pick_face(host, &f).await?;
                families.insert(face_key(&f.family, f.weight, f.italic), picked.family);
            }
        }
        let ctx = Ctx {
            doc,
            w,
            h,
            opts,
            pages: page_ids.clone(),
            tags: &tags,
            families,
            renderer,
            host,
            page_id: page.id.to_string(),
            page_no: i + 1,
            cut: Mutex::new(0),
            svg_opts: svg_opts.clone(),
        };
        let mobile = if reflow {
            Some(mobile_page(&items, &ctx, page.background.first()).await?)
        } else {
            None
        };
        let plan = plan_reflow(&items, w, h);
        let order = reading_order(&plan);
        let mut overlay: Vec<String> = Vec::new();
        for el in descendants(&doc.elements, &page.id) {
            if is_hidden(el) || el.is_deleted() {
                continue;
            }
            let id = el.id().unwrap_or("");
            let b = crate::geom::world_bounds(&doc.elements, id);
            let pos = format!(
                "left:{};top:{};width:{};height:{}",
                pct(b.x, w),
                pct(b.y, h),
                pct(b.w, w),
                pct(b.h, h)
            );
            let base = el.base();
            if let Some(link) = base.and_then(|b| b.link.as_ref()) {
                let (target, ext) = match link {
                    LinkTarget::Url(u) => (u.url.clone(), true),
                    LinkTarget::Page(p) => (
                        format!(
                            "#{}",
                            anchor(page_ids.iter().position(|x| *x == *p.page_id))
                        ),
                        false,
                    ),
                };
                let label = base
                    .and_then(|b| b.alt_text.clone().or_else(|| b.name.clone()))
                    .unwrap_or_else(|| "Link".into());
                overlay.push(format!(
                    "<a class=\"lk\" style=\"{pos}\" href=\"{}\"{} aria-label=\"{}\"></a>",
                    esc(&target),
                    if ext {
                        " target=\"_blank\" rel=\"noopener\""
                    } else {
                        ""
                    },
                    esc(&label)
                ));
            }
            match el {
                Element::Video(v) => {
                    let gif = v.clip.loop_ && v.clip.muted;
                    let src = (opts.asset_url)(&v.asset);
                    let radius = match v.corner_radius {
                        Some(r) if r != 0.0 && !r.is_nan() => {
                            format!(";border-radius:{}", pct(r, b.w.min(b.h)))
                        }
                        _ => String::new(),
                    };
                    let label = v
                        .base
                        .alt_text
                        .as_deref()
                        .filter(|a| !a.is_empty())
                        .map_or_else(String::new, |a| format!(" aria-label=\"{}\"", esc(a)));
                    overlay.push(format!(
                        "<video class=\"vd\" style=\"{pos}{radius}\" src=\"{}\" poster=\"{}\" playsinline preload=\"metadata\"{}{label}></video>",
                        esc(&src),
                        esc(&format!("{src}/poster")),
                        if gif { " autoplay muted loop" } else { " controls" }
                    ));
                }
                Element::Form(f) if f.base.xf.rot == 0.0 || f.base.xf.rot.is_nan() => {
                    overlay.push(form_html(doc, f, w, h, &opts.design_id))
                }
                Element::Embed(e)
                    if (e.base.xf.rot == 0.0 || e.base.xf.rot.is_nan())
                        && oc_schema::embeds::EMBED_FRAME_HOSTS
                            .iter()
                            .any(|x| e.src.starts_with(&format!("{x}/"))) =>
                {
                    // Live player over the poster; lazy so pages stay fast.
                    let m = b.w.min(b.h);
                    let title = if e.title.is_empty() {
                        e.provider.as_str().to_string()
                    } else {
                        e.title.clone()
                    };
                    overlay.push(embed_iframe(
                        "em",
                        &format!("{pos};border-radius:{}", pct(m * 0.04, m)),
                        &e.src,
                        &title,
                    ));
                }
                _ => {}
            }
        }
        let bg = page.background.first();
        let bg_css = match bg {
            Some(Paint::Image(_)) | None => None,
            Some(p) => paint_css(Some(p), &|id: &str| (opts.asset_url)(id)),
        };
        let title = page.title.clone().unwrap_or_default();
        let label = if title.is_empty() {
            format!("Section {}", i + 1)
        } else {
            title.clone()
        };
        if !title.is_empty() {
            nav.push(format!(
                "<a href=\"#{}\">{}</a>",
                anchor(Some(i)),
                esc(&title)
            ));
        }
        let picture = svg.replacen("<svg ", "<svg aria-hidden=\"true\" focusable=\"false\" ", 1);
        sections.push(format!(
            "<section id=\"{}\" aria-label=\"{}\"{}{}><div class=\"pg\">{}{picture}<div class=\"ov\">{}</div></div>{}</section>",
            anchor(Some(i)),
            esc(&label),
            if mobile.is_some() { " class=\"rf\"" } else { "" },
            bg_css.map_or_else(String::new, |c| format!(" style=\"background:{c}\"")),
            mirror_html(&order, &ctx),
            overlay.join(""),
            mobile.as_ref().map_or("", |m| m.html.as_str())
        ));
    }

    let lang = if doc.meta.lang.is_empty() {
        "en".to_string()
    } else {
        doc.meta.lang.clone()
    };
    let max_w = if w <= 1242.0 && h / w >= 1.5 {
        w.min(480.0)
    } else {
        w
    };
    let og = opts.og_image.as_deref().map_or_else(String::new, |o| format!("<meta property=\"og:image\" content=\"{}\"><meta name=\"twitter:card\" content=\"summary_large_image\">", esc(o)));
    let desc = |prop: &str| {
        description.as_deref().map_or_else(String::new, |d| {
            format!("<meta {prop} content=\"{}\">", esc(d))
        })
    };
    Ok(format!(
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n{}\n{}\n<meta property=\"og:title\" content=\"{}\">\n{}\n{og}\n<style>\n{}\n{}\n</style>\n</head>\n<body>\n{}\n<main>\n{}\n</main>\n{}\n{SCRIPT}\n</body>\n</html>",
        esc(&lang),
        esc(&opts.title),
        desc("name=\"description\""),
        if opts.noindex { "<meta name=\"robots\" content=\"noindex\">" } else { "" },
        esc(&opts.title),
        desc("property=\"og:description\""),
        font_faces.iter().map(|(_, f)| f.as_str()).collect::<Vec<_>>().join("\n"),
        STYLE.replace("{MAXW}", &js::num_to_string(max_w)),
        if nav.len() > 1 { format!("<nav class=\"top\" aria-label=\"Sections\">{}</nav>", nav.join("")) } else { String::new() },
        sections.join("\n"),
        opts.badge.as_ref().map_or_else(String::new, |(name, url)| format!("<a class=\"badge\" href=\"{}\" target=\"_blank\" rel=\"noopener\">Made with {}</a>", esc(url), esc(name))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_replacement() {
        assert_eq!(
            replace_svg_head("<svg a=\"1\"><g/></svg>", "<svg b>"),
            "<svg b><g/></svg>"
        );
        assert_eq!(
            scope_svg_ids(
                "<svg><path id=\"a\" fill=\"url(#g)\"/><use href=\"#a\"/></svg>",
                "p"
            ),
            "<svg><path id=\"pa\" fill=\"url(#pg)\"/><use href=\"#pa\"/></svg>"
        );
    }
}
