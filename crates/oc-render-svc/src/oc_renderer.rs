// SPDX-License-Identifier: Apache-2.0
//! [`Renderer`] on top of `crates/oc-render` (tiny-skia raster, SVG and krilla PDF writers, oc-text layout).
//!
//! oc-render does no I/O: a call resolves the page's display list (pure), asks the painter which font files and
//! which asset keys it needs, fetches those asynchronously through the [`RenderHost`], and then draws on a
//! blocking thread. The painter (font registry, decoded-image caches) is shared behind a mutex, so renders are
//! serialised like the TS service's (`serial`).

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use oc_render::displaylist::{DLItem, DisplayList};
use oc_render::pdf::{PdfOptions, PdfPage, write_pdf};
use oc_render::raster::{RenderOptions, to_rgba8};
use oc_render::{AssetProvider, ResolveOptions, resolve_page};
use oc_schema::primitives::{LinkTarget, Rect};
use oc_schema::tree::descendants;
use oc_schema::{DesignDoc, TextBody};
use oc_text::{FontRegistry, TextEngine};

use crate::renderer::*;

type Painter = Arc<Mutex<oc_render::Renderer>>;

/// Asset bytes prefetched for one render.
struct Prefetched(HashMap<String, Vec<u8>>);

impl AssetProvider for Prefetched {
    fn asset(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        self.0.get(key).map(|v| Cow::Borrowed(v.as_slice()))
    }
}

/// The oc-render backed renderer.
#[derive(Clone)]
pub struct OcRenderer {
    painter: Painter,
}

fn failed(e: impl std::fmt::Display) -> RenderError {
    RenderError::Failed(e.to_string())
}

fn resolve_opts(o: &RasterOptions) -> ResolveOptions {
    ResolveOptions {
        skip: o.skip.iter().cloned().collect(),
        transparent: o.transparent,
        time_ms: o.time_ms,
        caption_time_ms: o.caption_time_ms,
        ..ResolveOptions::default()
    }
}

/// Link areas of a page (page-space px) for PDF annotations: every visible element with a URL link, as the axis
/// bounds of its rotated box (pdfVector.ts / exporters.ts).
pub fn link_areas(doc: &DesignDoc, page_id: &str) -> Vec<(Rect, String)> {
    let mut out = Vec::new();
    for el in descendants(&doc.elements, page_id) {
        let Some(b) = el.base() else { continue };
        if b.visible == Some(false) {
            continue;
        }
        // SEC-01: only allowed schemes become PDF URI actions.
        if let Some(LinkTarget::Url(u)) = &b.link
            && oc_schema::links::is_safe_link_url(&u.url)
        {
            out.push((
                crate::geom::world_bounds(&doc.elements, &b.id),
                u.url.clone(),
            ));
        }
    }
    out
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> RenderResult<T> + Send + 'static,
) -> RenderResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(failed)?
}

impl OcRenderer {
    /// A painter over the host's font manifest (fonts are loaded on demand from the host).
    pub fn new(host: &dyn RenderHost) -> OcRenderer {
        OcRenderer::with_manifest(host.manifest().clone())
    }

    /// A painter over a font manifest.
    pub fn with_manifest(manifest: oc_gfonts::manifest::FontManifest) -> OcRenderer {
        OcRenderer {
            painter: Arc::new(Mutex::new(oc_render::Renderer::new(TextEngine::new(
                FontRegistry::new(manifest),
            )))),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, oc_render::Renderer> {
        self.painter.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Load the fonts the bodies need (registering uploaded families the manifest does not know). `strict`: a family
    /// that is unknown or whose face could not be fetched is an error ([`RenderError::MissingFonts`]) instead of
    /// falling back.
    async fn ensure_fonts(&self, host: &dyn RenderHost, bodies: &[&TextBody], strict: bool) -> RenderResult<()> {
        let unknown: Vec<String> = {
            let r = self.lock();
            let mut v: Vec<String> = Vec::new();
            for b in bodies {
                for f in oc_text::faces(b) {
                    if !r.text.fonts.has(&f.family) && !v.contains(&f.family) {
                        v.push(f.family.clone());
                    }
                }
            }
            v
        };
        if !unknown.is_empty() {
            let found = host.custom_families(&unknown).await.unwrap_or_default();
            let mut r = self.lock();
            for f in found {
                r.text.fonts.add_family(f);
            }
        }
        let mut files: Vec<String> = vec![];
        {
            let r = self.lock();
            let reqs: Vec<oc_text::FaceRequest> = bodies
                .iter()
                .flat_map(|b| oc_text::faces(b))
                .chain([oc_text::FaceRequest::new(
                    oc_text::FALLBACK_FAMILY,
                    400.0,
                    false,
                )])
                .collect();
            for f in r.text.fonts.missing(reqs.iter()) {
                if !files.contains(&f) {
                    files.push(f);
                }
            }
        }
        for file in files {
            // A face that cannot be fetched falls back like the editor while fonts load.
            let Ok(bytes) = host.font_file(&file).await else {
                continue;
            };
            let mut r = self.lock();
            if r.text.fonts.insert_face(&file, bytes.clone()).is_err() {
                let _ = r.text.fonts.insert_unchecked(&file, bytes);
            }
        }
        if strict {
            let r = self.lock();
            let mut lost: Vec<String> = Vec::new();
            for f in bodies.iter().flat_map(|b| oc_text::faces(b)) {
                let unavailable = !r.text.fonts.has(&f.family) || !r.text.fonts.missing([&f]).is_empty();
                if unavailable && !lost.contains(&f.family) {
                    lost.push(f.family.clone());
                }
            }
            if !lost.is_empty() {
                return Err(RenderError::MissingFonts(lost));
            }
        }
        Ok(())
    }

    /// Fonts and asset bytes for a display list.
    async fn prepare(&self, host: &dyn RenderHost, dl: &DisplayList, strict_fonts: bool) -> RenderResult<Prefetched> {
        let bodies: Vec<&TextBody> = dl
            .items
            .iter()
            .filter_map(|it| {
                if let DLItem::Text { body, .. } = it {
                    Some(body)
                } else {
                    None
                }
            })
            .collect();
        self.ensure_fonts(host, &bodies, strict_fonts).await?;
        let mut keys: Vec<String> = dl.assets.iter().map(|a| a.to_string()).collect();
        for it in &dl.items {
            if let DLItem::Video { asset, .. } = it {
                keys.push(format!("{asset}/poster"));
            }
        }
        let mut map = HashMap::new();
        for k in keys {
            if map.contains_key(&k) {
                continue;
            }
            // Missing assets draw their placeholders.
            if let Ok(b) = host.asset_bytes(&k).await {
                map.insert(k, b);
            }
        }
        Ok(Prefetched(map))
    }

    async fn pixmap(
        &self,
        host: &dyn RenderHost,
        doc: &DesignDoc,
        page_id: &str,
        opts: &RasterOptions,
        white: bool,
    ) -> RenderResult<oc_render::tiny_skia::Pixmap> {
        let dl = resolve_page(doc, page_id, &resolve_opts(opts));
        let assets = self.prepare(host, &dl, opts.strict_fonts).await?;
        let p = self.painter.clone();
        let ro = RenderOptions {
            scale: opts.scale,
            background: white.then_some(oc_render::tiny_skia::Color::WHITE),
        };
        let frames = opts.video_frames.clone();
        blocking(move || {
            let mut r = p.lock().unwrap_or_else(|e| e.into_inner());
            // `painter.videoFrames` for this render only.
            r.video_frames = match frames {
                Some(f) => Some(Box::new(FrameMap::new(&f)?)),
                None => None,
            };
            let out = r.render(&dl, &assets, &ro);
            r.video_frames = None;
            out.ok_or_else(|| failed("could not allocate the page raster"))
        })
        .await
    }

    async fn svg_of(
        &self,
        host: &dyn RenderHost,
        dl: DisplayList,
        opts: &SvgOptions,
    ) -> RenderResult<String> {
        let assets = self.prepare(host, &dl, false).await?;
        let mut fonts = Vec::new();
        if !opts.external_fonts {
            let picks: Vec<(String, f64, bool, String)> = {
                let r = self.lock();
                let mut v: Vec<(String, f64, bool, String)> = Vec::new();
                for it in &dl.items {
                    if let DLItem::Text { body, .. } = it {
                        for f in oc_text::faces(body) {
                            if let Some((fam, face)) =
                                r.text.fonts.pick_face(&f.family, f.weight, f.italic)
                                && !v.iter().any(|x| {
                                    x.0 == fam.family
                                        && x.1 == f64::from(face.weight)
                                        && x.2 == face.italic
                                })
                            {
                                v.push((
                                    fam.family.clone(),
                                    f64::from(face.weight),
                                    face.italic,
                                    face.file.clone(),
                                ));
                            }
                        }
                    }
                }
                v
            };
            for (family, weight, italic, file) in picks {
                if let Ok(bytes) = host.font_file(&file).await {
                    fonts.push(oc_render::svg::SvgFont {
                        family,
                        weight,
                        italic,
                        bytes,
                    });
                }
            }
        }
        let href = opts.asset_href.clone();
        let external = opts.external_fonts;
        let p = self.painter.clone();
        blocking(move || {
            let f = href.map(|h| move |k: &str| Some(h(k)));
            let hf: Option<&oc_render::svg::AssetHref> =
                f.as_ref().map(|x| x as &oc_render::svg::AssetHref);
            let o = oc_render::svg::SvgOptions {
                fonts,
                asset_href: hf,
                external_fonts: external,
            };
            Ok(oc_render::svg::display_list_to_svg(
                &mut p.lock().unwrap_or_else(|e| e.into_inner()),
                &dl,
                &assets,
                &o,
            ))
        })
        .await
    }

    async fn pdf(
        &self,
        host: &dyn RenderHost,
        doc: &DesignDoc,
        dls: Vec<DisplayList>,
        links: Vec<Vec<(Rect, String)>>,
        opts: PdfOptions,
        extra: HashMap<String, Vec<u8>>,
    ) -> RenderResult<Vec<u8>> {
        let mut assets = Prefetched(extra);
        for dl in &dls {
            assets.0.extend(self.prepare(host, dl, false).await?.0);
        }
        let _ = doc;
        let p = self.painter.clone();
        blocking(move || {
            let pages: Vec<PdfPage<'_>> = dls
                .iter()
                .zip(links)
                .map(|(dl, links)| PdfPage { dl, links })
                .collect();
            write_pdf(
                &mut p.lock().unwrap_or_else(|e| e.into_inner()),
                &pages,
                &assets,
                &opts,
            )
            .map_err(failed)
        })
        .await
    }
}

/// Decoded video frames handed to the painter (premultiplied once per render).
struct FrameMap(HashMap<String, Arc<oc_render::tiny_skia::Pixmap>>);

impl FrameMap {
    fn new(frames: &VideoFrameSet) -> RenderResult<FrameMap> {
        let mut m = HashMap::new();
        for (k, f) in frames.iter() {
            m.insert(k.clone(), Arc::new(to_pixmap(f)?));
        }
        Ok(FrameMap(m))
    }
}

impl oc_render::VideoFrames for FrameMap {
    fn frame(
        &self,
        _asset: &str,
        _source_time_ms: f64,
        key: &str,
    ) -> Option<Arc<oc_render::tiny_skia::Pixmap>> {
        self.0.get(key).cloned()
    }
}

/// `composeTransition(r, a, b, tf)`: the transition frame drawn like Present mode over black, both frames placed
/// at the origin unfiltered (CanvasKit `drawImage(img, 0, 0, null)`: nearest sampling). The result has the size of
/// the incoming frame.
pub fn compose_frames(prev: &Rgba, next: &Rgba, t: &TransitionSpec) -> RenderResult<Rgba> {
    use oc_render::raster::Canvas;
    use oc_render::raster::transition::{Side, draw_transition};
    use oc_render::tiny_skia::{
        FilterQuality, Paint, Pattern, Rect as SkRect, SpreadMode, Transform,
    };
    use oc_schema::design::TransitionKind;
    use oc_schema::elements::AnimDirection;
    let (w, h) = (next.width, next.height);
    let pa = to_pixmap(prev)?;
    let pb = to_pixmap(next)?;
    let mut c =
        Canvas::new(w.max(1), h.max(1)).ok_or_else(|| failed("could not allocate the frame"))?;
    let mut black = Paint::default();
    black.set_color(oc_render::tiny_skia::Color::BLACK);
    c.fill_all(&black);
    let frame = oc_render::motion::transition_frame(
        &TransitionKind::parse(&t.kind),
        t.progress,
        t.direction.as_deref().map(AnimDirection::parse).as_ref(),
    );
    draw_transition(&mut c, f64::from(w), f64::from(h), &frame, |side, cv| {
        let img = if side == Side::Prev { &pa } else { &pb };
        let Some(r) = SkRect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32) else {
            return;
        };
        let paint = Paint {
            shader: Pattern::new(
                img.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Nearest,
                1.0,
                Transform::identity(),
            ),
            anti_alias: false,
            ..Default::default()
        };
        cv.fill_rect(r, &paint);
    });
    let p = c.into_pixmap();
    Ok(Rgba {
        width: p.width(),
        height: p.height(),
        pixels: to_rgba8(&p),
    })
}

fn to_pixmap(rgba: &Rgba) -> RenderResult<oc_render::tiny_skia::Pixmap> {
    let mut data = rgba.pixels.clone();
    oc_render::assets::premultiply(&mut data);
    let size = oc_render::tiny_skia::IntSize::from_wh(rgba.width.max(1), rgba.height.max(1))
        .ok_or_else(|| failed("empty raster"))?;
    oc_render::tiny_skia::Pixmap::from_vec(data, size).ok_or_else(|| failed("bad raster size"))
}

impl Renderer for OcRenderer {
    fn engine(&self) -> &'static str {
        "oc-render"
    }

    fn render_rgba<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a RasterOptions,
    ) -> RenderFuture<'a, Rgba> {
        Box::pin(async move {
            let p = self.pixmap(host, doc, page_id, opts, false).await?;
            Ok(Rgba {
                width: p.width(),
                height: p.height(),
                pixels: to_rgba8(&p),
            })
        })
    }

    fn render_image<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a RasterOptions,
        format: ImageFormat,
    ) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let jpeg = matches!(format, ImageFormat::Jpeg { .. });
            let p = self.pixmap(host, doc, page_id, opts, jpeg).await?;
            Ok(match format {
                ImageFormat::Png => oc_render::encode_png(&p),
                ImageFormat::Jpeg { quality } => oc_render::encode_jpeg(&p, quality),
            })
        })
    }

    fn compose_transition<'a>(
        &'a self,
        prev: &'a Rgba,
        next: &'a Rgba,
        transition: &'a TransitionSpec,
    ) -> RenderFuture<'a, Rgba> {
        Box::pin(async move {
            let (a, b, t) = (prev.clone(), next.clone(), transition.clone());
            blocking(move || compose_frames(&a, &b, &t)).await
        })
    }

    fn vector_pdf<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_ids: &'a [String],
        opts: &'a VectorPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let dls: Vec<DisplayList> = page_ids
                .iter()
                .map(|p| resolve_page(doc, p, &ResolveOptions::default()))
                .collect();
            let links = page_ids.iter().map(|p| link_areas(doc, p)).collect();
            let o = PdfOptions {
                title: Some(opts.title.clone().unwrap_or_else(|| doc.meta.title.clone())),
                producer: opts.producer.clone(),
                lang: Some(if doc.meta.lang.is_empty() {
                    "en".into()
                } else {
                    doc.meta.lang.clone()
                }),
                print: match opts.print.as_ref() {
                    Some(p) => Some(crate::print::to_pdf_print(p).map_err(RenderError::Failed)?),
                    None => None,
                },
                created_unix: Some(chrono::Utc::now().timestamp()),
                ..PdfOptions::default()
            };
            self.pdf(host, doc, dls, links, o, HashMap::new()).await
        })
    }

    fn raster_pdf<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_ids: &'a [String],
        opts: &'a RasterPdfOptions,
    ) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            // Each page is one picture (150 dpi PNG, or 300 dpi JPEG for print) placed by the PDF writer, plus the
            // page's link areas.
            let scale = if opts.print {
                300.0 / 96.0
            } else {
                150.0 / 96.0
            };
            let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
            let mut dls = Vec::new();
            let mut extra = HashMap::new();
            for (i, page) in page_ids.iter().enumerate() {
                let fmt = if opts.print {
                    ImageFormat::Jpeg { quality: 92 }
                } else {
                    ImageFormat::Png
                };
                let bytes = self
                    .render_image(host, doc, page, &RasterOptions::at_scale(scale), fmt)
                    .await?;
                let key = format!("__page_raster_{i}");
                extra.insert(key.clone(), bytes);
                let mut dl = DisplayList::new(w, h);
                // The picture, then the page's text ops (with their transforms and clips) drawn invisibly: the
                // selectable text layer. Layers become plain saves (their look is in the picture).
                let text_layer = resolve_page(doc, page, &ResolveOptions::default())
                    .items
                    .into_iter()
                    .filter_map(|it| match it {
                        DLItem::Save { .. }
                        | DLItem::Restore
                        | DLItem::Clip { .. }
                        | DLItem::Text { .. } => Some(it),
                        DLItem::Layer { .. } => Some(DLItem::Save {
                            matrix: oc_geometry::IDENTITY,
                        }),
                        _ => None,
                    });
                dl.items.push(DLItem::Image {
                    asset: key.clone(),
                    dst: Rect {
                        x: 0.0,
                        y: 0.0,
                        w,
                        h,
                    },
                    crop: Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 1.0,
                        h: 1.0,
                    }
                    .into(),
                    edits: None,
                    clip: None,
                    src_el: String::new(),
                });
                dl.items.extend(text_layer);
                dls.push(dl);
            }
            let links = page_ids.iter().map(|p| link_areas(doc, p)).collect();
            let o = PdfOptions {
                title: Some(doc.meta.title.clone()),
                producer: opts.producer.clone(),
                lang: Some(if doc.meta.lang.is_empty() {
                    "en".into()
                } else {
                    doc.meta.lang.clone()
                }),
                invisible_text: true,
                ..PdfOptions::default()
            };
            self.pdf(host, doc, dls, links, o, extra).await
        })
    }

    fn page_svg<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        opts: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        Box::pin(async move {
            self.svg_of(
                host,
                resolve_page(doc, page_id, &ResolveOptions::default()),
                opts,
            )
            .await
        })
    }

    fn elements_svg<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
        ids: &'a BTreeSet<String>,
        opts: &'a SvgOptions,
    ) -> RenderFuture<'a, String> {
        Box::pin(async move {
            let mut dl = resolve_page(doc, page_id, &ResolveOptions::default());
            dl.background.clear();
            dl.items.retain(|it| {
                let src = match it {
                    DLItem::Save { .. }
                    | DLItem::Restore
                    | DLItem::Clip { .. }
                    | DLItem::Layer { .. }
                    | DLItem::Mask { .. }
                    | DLItem::MaskContent => return true,
                    DLItem::Path { src_el, .. }
                    | DLItem::Line { src_el, .. }
                    | DLItem::Image { src_el, .. }
                    | DLItem::Text { src_el, .. }
                    | DLItem::Qr { src_el, .. }
                    | DLItem::Placeholder { src_el, .. }
                    | DLItem::Video { src_el, .. } => src_el,
                };
                ids.contains(src.as_str())
            });
            self.svg_of(host, dl, opts).await
        })
    }

    fn text_faces<'a>(
        &'a self,
        _: &'a dyn RenderHost,
        doc: &'a DesignDoc,
        page_id: &'a str,
    ) -> RenderFuture<'a, Vec<FaceRequest>> {
        Box::pin(async move {
            let dl = resolve_page(doc, page_id, &ResolveOptions::default());
            Ok(dl
                .items
                .iter()
                .filter_map(|it| {
                    if let DLItem::Text { body, .. } = it {
                        Some(body)
                    } else {
                        None
                    }
                })
                .flat_map(oc_text::faces)
                .map(|f| FaceRequest {
                    family: f.family,
                    weight: f.weight,
                    italic: f.italic,
                })
                .collect())
        })
    }

    fn pick_face<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        request: &'a FaceRequest,
    ) -> RenderFuture<'a, PickedFace> {
        Box::pin(async move {
            let known = self.lock().text.fonts.has(&request.family);
            if !known
                && let Ok(found) = host
                    .custom_families(std::slice::from_ref(&request.family))
                    .await
            {
                let mut r = self.lock();
                for f in found {
                    r.text.fonts.add_family(f);
                }
            }
            let r = self.lock();
            let (fam, face) = r
                .text
                .fonts
                .pick_face(&request.family, request.weight, request.italic)
                .ok_or_else(|| failed("no fonts in the manifest"))?;
            Ok(PickedFace {
                family: fam.family.clone(),
                weight: f64::from(face.weight),
                italic: face.italic,
                file: face.file.clone(),
            })
        })
    }

    fn layout_text<'a>(
        &'a self,
        host: &'a dyn RenderHost,
        body: &'a TextBody,
        width: f64,
    ) -> RenderFuture<'a, TextMeasure> {
        Box::pin(async move {
            self.ensure_fonts(host, &[body], false).await?;
            let mut r = self.lock();
            let l = r.text.layout_uncached(body, width, None);
            let lines = l.paras.first().map_or_else(Vec::new, |p| {
                p.lines
                    .iter()
                    .map(|m| LineMetrics {
                        start: m.start,
                        baseline: m.baseline,
                        descent: m.descent,
                    })
                    .collect()
            });
            Ok(TextMeasure {
                height: l.height,
                lines,
            })
        })
    }

    fn encode_png<'a>(&'a self, rgba: &'a Rgba) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move { Ok(oc_render::encode_png(&to_pixmap(rgba)?)) })
    }

    fn transcode_png<'a>(&'a self, bytes: &'a [u8]) -> RenderFuture<'a, Option<DecodedImage>> {
        Box::pin(async move {
            Ok(
                oc_render::assets::decode_raster(bytes).map(|p| DecodedImage {
                    png: oc_render::encode_png(&p),
                    width: p.width(),
                    height: p.height(),
                }),
            )
        })
    }

    fn rasterize_svg<'a>(&'a self, svg: &'a str, width: u32) -> RenderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let img = oc_render::assets::decode_svg(svg.as_bytes())
                .ok_or_else(|| failed("invalid SVG"))?;
            let (sw, sh) = img.size();
            let w = width.max(1);
            let h = ((f64::from(w) * f64::from(sh) / f64::from(sw.max(1.0))).round() as u32).max(1);
            let r = img
                .raster(w, h)
                .ok_or_else(|| failed("could not rasterise SVG"))?;
            Ok(oc_render::encode_png(&r.pixmap))
        })
    }

    fn path_commands(&self, d: &str) -> RenderResult<Option<Vec<PathCmd>>> {
        use oc_render::path::Cmd;
        Ok(oc_render::path::parse(d).map(|p| {
            p.cmds
                .iter()
                .map(|c| match *c {
                    Cmd::Move(x, y) => PathCmd::Move(x, y),
                    Cmd::Line(x, y) => PathCmd::Line(x, y),
                    Cmd::Quad(a, b, x, y) => PathCmd::Quad(a, b, x, y),
                    Cmd::Cubic(a, b, c2, d2, x, y) => PathCmd::Cubic(a, b, c2, d2, x, y),
                    Cmd::Close => PathCmd::Close,
                })
                .collect()
        }))
    }
}
