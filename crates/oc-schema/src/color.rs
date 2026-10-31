// SPDX-License-Identifier: Apache-2.0
//! Colour constructors, conversion and WCAG contrast.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::js;
use crate::primitives::{ColorValue, Paint};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Invalid hex colour: {0}")]
pub struct InvalidHex(pub String);

fn clamp01(v: f64) -> f64 {
    // Math.min(1, Math.max(0, v)) — NaN propagates.
    if v.is_nan() { v } else { v.clamp(0.0, 1.0) }
}

pub fn rgb(r: f64, g: f64, b: f64, a: f64) -> ColorValue {
    ColorValue::srgb(clamp01(r), clamp01(g), clamp01(b), clamp01(a))
}

/// `'#rgb'`, `'#rgba'`, `'#rrggbb'`, `'#rrggbbaa'` → sRGB colour.
pub fn hex(s: &str) -> Result<ColorValue, InvalidHex> {
    let t = js::trim(s);
    let mut h: String = t.strip_prefix('#').unwrap_or(t).to_string();
    let n16 = js::utf16_len(&h);
    if n16 == 3 || n16 == 4 {
        h = h.chars().flat_map(|c| [c, c]).collect();
    }
    let ok = (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit());
    if !ok {
        return Err(InvalidHex(s.to_string()));
    }
    let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(f64::from).unwrap_or(0.0) / 255.0;
    Ok(rgb(n(0), n(2), n(4), if h.len() == 8 { n(6) } else { 1.0 }))
}

/// `hex()` for compile-time-known literals in this crate.
pub(crate) fn hexc(s: &str) -> ColorValue {
    hex(s).expect("valid literal colour")
}

/// Convert any authored colour to display sRGB (naive for CMYK/spot).
pub fn to_rgba(c: &ColorValue) -> Rgba {
    match c {
        ColorValue::Srgb(x) | ColorValue::DisplayP3(x) => Rgba { r: x.r, g: x.g, b: x.b, a: x.a },
        ColorValue::Cmyk(x) => Rgba {
            r: (1.0 - x.c) * (1.0 - x.k),
            g: (1.0 - x.m) * (1.0 - x.k),
            b: (1.0 - x.y) * (1.0 - x.k),
            a: x.a,
        },
        ColorValue::Spot(x) => Rgba { r: 0.5, g: 0.5, b: 0.5, a: x.a },
    }
}

fn hex2(v: f64) -> String {
    let n = js::round(clamp01(v) * 255.0);
    format!("{:02x}", n as i64)
}

pub fn to_hex(c: &ColorValue, with_alpha: bool) -> String {
    let Rgba { r, g, b, a } = to_rgba(c);
    let alpha = if with_alpha && a < 1.0 { hex2(a) } else { String::new() };
    format!("#{}{}{}{}", hex2(r), hex2(g), hex2(b), alpha)
}

pub fn to_css(c: &ColorValue) -> String {
    let Rgba { r, g, b, a } = to_rgba(c);
    format!(
        "rgba({}, {}, {}, {})",
        js::num_to_string(js::round(r * 255.0)),
        js::num_to_string(js::round(g * 255.0)),
        js::num_to_string(js::round(b * 255.0)),
        js::num_to_string(js::to_fixed_num(a, 3))
    )
}

/// `solid(colour)`.
pub fn solid(c: ColorValue) -> Paint {
    Paint::solid_color(c)
}

/// `solid('#rrggbb')`.
pub fn solid_hex(s: &str) -> Result<Paint, InvalidHex> {
    hex(s).map(Paint::solid_color)
}

pub(crate) fn solidc(s: &str) -> Paint {
    Paint::solid_color(hexc(s))
}

/// The representative colour of a paint (first stop for gradients).
pub fn paint_color(p: Option<&Paint>) -> Option<&ColorValue> {
    match p? {
        Paint::Solid(s) => Some(&s.color),
        Paint::Linear(l) => l.stops.first().map(|s| &s.color),
        Paint::Radial(r) => r.stops.first().map(|s| &s.color),
        Paint::Image(_) => None,
    }
}

/// WCAG relative luminance.
pub fn luminance(c: &ColorValue) -> f64 {
    let Rgba { r, g, b, .. } = to_rgba(c);
    let lin = |v: f64| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio.
pub fn contrast_ratio(a: &ColorValue, b: &ColorValue) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (l1, l2) = if lb > la { (lb, la) } else { (la, lb) };
    (l1 + 0.05) / (l2 + 0.05)
}
