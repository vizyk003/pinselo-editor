// SPDX-License-Identifier: Apache-2.0
//! Email export (EM-001), port of `services/render/src/email.ts`: table-based HTML that survives email clients.
//! The page is cut into horizontal bands at element edges; bands holding links are split into cells so each link
//! is its own clickable image. Text in a band becomes its alt text (shown when images are blocked). Slices are
//! hosted (content-addressed) so the HTML can be pasted into any email service.
//!
//! The band/cell plan and the HTML are computed here; the page raster comes from [`Renderer::render_rgba`] and
//! the slices are encoded by [`Renderer::encode_png`].

use futures::future::BoxFuture;
use oc_schema::DesignDoc;
use oc_schema::color::to_rgba;
use oc_schema::elements::Element;
use oc_schema::js;
use oc_schema::primitives::{LinkTarget, Paint};
use oc_schema::text::body_plain_text;
use oc_schema::tree::descendants;

use crate::renderer::{RasterOptions, RenderError, RenderHost, Renderer, Rgba};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn n(v: f64) -> String {
    js::num_to_string(v)
}

/// `EmailDeps`: store a PNG slice and return its public URL.
pub trait EmailDeps: Send + Sync {
    fn host<'a>(&'a self, png: Vec<u8>) -> BoxFuture<'a, Result<String, RenderError>>;
}

/// The email HTML and its plain-text alternative.
#[derive(Debug, Clone, PartialEq)]
pub struct EmailOut {
    pub html: String,
    pub text: String,
}

/// Copy a `cw × ch` region starting at `(x, y)` of a raster exactly as the TS does: row `yy` is the `cw * 4`
/// bytes at flat offset `((y + yy) * width + x) * 4` (cut short at the end of the buffer, zeros beyond).
pub fn crop_like_ts(src: &Rgba, x: f64, y: f64, cw: f64, ch: f64) -> Rgba {
    let (cw, ch) = (cw.max(1.0) as usize, ch.max(1.0) as usize);
    let width = src.width as usize;
    let mut out = vec![0u8; cw * ch * 4];
    for yy in 0..ch {
        let start = ((y as usize + yy) * width + x as usize) * 4;
        if start >= src.pixels.len() {
            break;
        }
        let end = (start + cw * 4).min(src.pixels.len());
        out[yy * cw * 4..yy * cw * 4 + (end - start)].copy_from_slice(&src.pixels[start..end]);
    }
    Rgba {
        width: cw as u32,
        height: ch as u32,
        pixels: out,
    }
}

/// `crop(r, src, x, y, w, h)`: sizes rounded and clamped to the raster, at least 1 × 1.
fn crop_size(src: &Rgba, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    (
        (f64::from(src.width) - x).min(js::round(w)).max(1.0),
        (f64::from(src.height) - y).min(js::round(h)).max(1.0),
    )
}

fn hidden(el: &Element) -> bool {
    match el {
        Element::Unknown(m) => m.get("visible") == Some(&serde_json::Value::Bool(false)),
        _ => el.base().is_some_and(|b| b.visible == Some(false)),
    }
}

fn collapse(s: &str) -> String {
    let mut out = String::new();
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
    out
}

fn slice16(s: &str, n: usize) -> String {
    crate::pm::slice16(s, n)
}

#[derive(Debug, Clone)]
struct Cell {
    x0: f64,
    x1: f64,
    url: Option<String>,
}

/// `exportEmail(r, doc, pages, deps)`.
pub async fn export_email(
    r: &dyn Renderer,
    host: &dyn RenderHost,
    doc: &DesignDoc,
    page_ids: &[String],
    deps: &dyn EmailDeps,
) -> Result<EmailOut, RenderError> {
    // SEC-01: no javascript:/data: links in the output, whatever the stored design holds.
    let safe = oc_schema::links::with_safe_links(doc);
    let doc: &DesignDoc = &safe;
    let (w, h) = (doc.meta.dims.w, doc.meta.dims.h);
    let scale = 2.0;
    let mut rows: Vec<String> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for page_id in page_ids {
        let Some(page) = doc.pages.get(page_id.as_str()) else {
            continue;
        };
        let rgba = r
            .render_rgba(host, doc, page_id, &RasterOptions::at_scale(scale))
            .await?;
        let els: Vec<&Element> = descendants(&doc.elements, page_id)
            .into_iter()
            .filter(|e| !hidden(e) && !e.is_deleted())
            .collect();
        let boxes: Vec<(&Element, oc_schema::primitives::Rect)> = els
            .iter()
            .map(|e| {
                (
                    *e,
                    crate::geom::world_bounds(&doc.elements, e.id().unwrap_or("")),
                )
            })
            .collect();
        // Band edges at element tops/bottoms (clamped), merged so bands are at least 24 px tall.
        let mut cuts: Vec<f64> = vec![0.0, h];
        for (_, b) in &boxes {
            for y in [b.y, b.y + b.h] {
                if y > 0.0 && y < h {
                    let v = js::round(y);
                    if !cuts.contains(&v) {
                        cuts.push(v);
                    }
                }
            }
        }
        cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut edges: Vec<f64> = vec![0.0];
        for y in cuts.iter().skip(1) {
            if y - edges[edges.len() - 1] >= 24.0 || *y == h {
                edges.push(*y);
            }
        }
        if edges[edges.len() - 1] != h {
            edges.push(h);
        }
        let bg = match page.background.first() {
            Some(Paint::Solid(s)) => {
                let c = to_rgba(&s.color);
                format!(
                    "#{}",
                    [c.r, c.g, c.b]
                        .iter()
                        .map(|v| format!("{:02x}", js::round(v * 255.0) as i64))
                        .collect::<String>()
                )
            }
            _ => "#ffffff".to_string(),
        };
        for i in 0..edges.len().saturating_sub(1) {
            let (y0, y1) = (edges[i], edges[i + 1]);
            let in_band: Vec<&(&Element, oc_schema::primitives::Rect)> = boxes
                .iter()
                .filter(|(_, b)| b.y < y1 && b.y + b.h > y0)
                .collect();
            let texts: Vec<String> = in_band
                .iter()
                .filter_map(|(e, _)| {
                    if let Element::Text(t) = e {
                        Some(body_plain_text(&t.body))
                    } else {
                        None
                    }
                })
                .collect();
            let mut alt = slice16(js::trim(&collapse(&texts.join(" "))), 300);
            if alt.is_empty() {
                let alts: Vec<&str> = in_band
                    .iter()
                    .filter_map(|(e, _)| e.base().and_then(|b| b.alt_text.as_deref()))
                    .filter(|a| !a.is_empty())
                    .collect();
                alt = slice16(&alts.join(" "), 300);
            }
            if !alt.is_empty() {
                plain.push(alt.clone());
            }
            // Horizontal cells: linked elements become their own cells.
            let mut links: Vec<Cell> = in_band
                .iter()
                .filter_map(|(e, b)| match e.base().and_then(|x| x.link.as_ref()) {
                    Some(LinkTarget::Url(u)) => Some(Cell {
                        x0: b.x.floor().max(0.0),
                        x1: w.min((b.x + b.w).ceil()),
                        url: Some(u.url.clone()),
                    }),
                    _ => None,
                })
                .collect();
            links.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
            let mut cells: Vec<Cell> = Vec::new();
            let mut x = 0.0;
            for l in links {
                if l.x0 < x {
                    continue; // overlapping links: the first wins
                }
                if l.x0 > x {
                    cells.push(Cell {
                        x0: x,
                        x1: l.x0,
                        url: None,
                    });
                }
                x = l.x1;
                plain.push(format!("[{}]", l.url.as_deref().unwrap_or("")));
                cells.push(l);
            }
            if x < w {
                cells.push(Cell {
                    x0: x,
                    x1: w,
                    url: None,
                });
            }
            let mut tds = String::new();
            for c in &cells {
                let cw = c.x1 - c.x0;
                if cw <= 0.0 {
                    continue;
                }
                let (sx, sy) = (c.x0 * scale, y0 * scale);
                let (ow, oh) = crop_size(&rgba, sx, sy, cw * scale, (y1 - y0) * scale);
                let png = r.encode_png(&crop_like_ts(&rgba, sx, sy, ow, oh)).await?;
                let url = deps.host(png).await?;
                let label = match &c.url {
                    Some(_) if alt.is_empty() => "Link".to_string(),
                    _ => alt.clone(),
                };
                let img = format!(
                    "<img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{}\" style=\"display:block;width:100%;height:auto;border:0;outline:none;text-decoration:none\">",
                    esc(&url),
                    n(cw),
                    n(y1 - y0),
                    esc(&label)
                );
                let inner = match &c.url {
                    Some(u) => format!(
                        "<a href=\"{}\" target=\"_blank\" style=\"display:block\">{img}</a>",
                        esc(u)
                    ),
                    None => img,
                };
                tds.push_str(&format!(
                    "<td width=\"{}\" style=\"padding:0;width:{}%\">{inner}</td>",
                    n(cw),
                    js::to_fixed((cw / w) * 100.0, 3)
                ));
            }
            rows.push(format!(
                "<tr><td style=\"padding:0;background:{bg}\"><table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr>{tds}</tr></table></td></tr>"
            ));
        }
    }
    let lang = if doc.meta.lang.is_empty() {
        "en"
    } else {
        doc.meta.lang.as_str()
    };
    let html = format!(
        "<!doctype html>\n<html lang=\"{}\" xmlns=\"http://www.w3.org/1999/xhtml\">\n<head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"x-apple-disable-message-reformatting\"><title>{}</title>\n<style>body{{margin:0;padding:0;background:#f4f4f7}}img{{-ms-interpolation-mode:bicubic}}@media (max-width:{W}px){{.wrap{{width:100%!important}}}}</style></head>\n<body style=\"margin:0;padding:0;background:#f4f4f7\">\n<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"background:#f4f4f7\"><tr><td align=\"center\" style=\"padding:16px 0\">\n<table role=\"presentation\" class=\"wrap\" width=\"{W}\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"width:{W}px;max-width:100%\">\n{}\n</table></td></tr></table>\n</body></html>",
        esc(lang),
        esc(&doc.meta.title),
        rows.join("\n"),
        W = n(w)
    );
    Ok(EmailOut {
        html,
        text: plain.join("\n\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_reads_like_ts() {
        let mut src = Rgba::blank(3, 2);
        for (i, p) in src.pixels.iter_mut().enumerate() {
            *p = i as u8;
        }
        // Starting at the last column, a 2-wide crop runs into the next row (flat offsets, like the TS).
        let c = crop_like_ts(&src, 2.0, 0.0, 2.0, 1.0);
        assert_eq!(&c.pixels[..8], &[8, 9, 10, 11, 12, 13, 14, 15]);
        let c = crop_like_ts(&src, 0.0, 1.0, 3.0, 3.0);
        assert_eq!(c.height, 3);
        assert!(c.pixels[12..].iter().all(|b| *b == 0));
    }
}
