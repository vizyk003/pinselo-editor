// SPDX-License-Identifier: Apache-2.0
//! 4×5 colour matrices (row-major, translation normalized to 0..1) for non-destructive image edits
//! (`packages/render-canvaskit/src/colorMatrix.ts`), and the tone/detail adjustments that need a per-pixel pass.

use oc_schema::color::to_rgba;
use oc_schema::elements::{Duotone, ImageAdjust, ImageEditStack, ImageFilterPreset};

/// A 4×5 colour matrix: `[r', g', b', a'] = M · [r, g, b, a, 1]` with the translation column in 0..1 units.
pub type Cm = [f64; 20];

pub const IDENTITY_CM: Cm = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// a ∘ b (apply b first, then a).
pub fn concat(a: &Cm, b: &Cm) -> Cm {
    let mut out = [0.0; 20];
    for r in 0..4 {
        for c in 0..5 {
            let mut v = if c == 4 { a[r * 5 + 4] } else { 0.0 };
            for k in 0..4 {
                v += a[r * 5 + k] * b[k * 5 + c];
            }
            out[r * 5 + c] = v;
        }
    }
    out
}

pub fn lerp(a: &Cm, b: &Cm, t: f64) -> Cm {
    let mut out = [0.0; 20];
    for i in 0..20 {
        out[i] = a[i] + (b[i] - a[i]) * t;
    }
    out
}

const LR: f64 = 0.2126;
const LG: f64 = 0.7152;
const LB: f64 = 0.0722;

pub fn saturation(s: f64) -> Cm {
    let r = (1.0 - s) * LR;
    let g = (1.0 - s) * LG;
    let b = (1.0 - s) * LB;
    [r + s, g, b, 0.0, 0.0, r, g + s, b, 0.0, 0.0, r, g, b + s, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
}

pub fn contrast(c: f64) -> Cm {
    let t = 0.5 * (1.0 - c);
    [c, 0.0, 0.0, 0.0, t, 0.0, c, 0.0, 0.0, t, 0.0, 0.0, c, 0.0, t, 0.0, 0.0, 0.0, 1.0, 0.0]
}

pub fn brightness(b: f64) -> Cm {
    [1.0, 0.0, 0.0, 0.0, b, 0.0, 1.0, 0.0, 0.0, b, 0.0, 0.0, 1.0, 0.0, b, 0.0, 0.0, 0.0, 1.0, 0.0]
}

pub fn temperature(t: f64, tint: f64) -> Cm {
    [1.0, 0.0, 0.0, 0.0, t, 0.0, 1.0, 0.0, 0.0, -tint, 0.0, 0.0, 1.0, 0.0, -t, 0.0, 0.0, 0.0, 1.0, 0.0]
}

const SEPIA: Cm = [0.393, 0.769, 0.189, 0.0, 0.0, 0.349, 0.686, 0.168, 0.0, 0.0, 0.272, 0.534, 0.131, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

const INVERT: Cm = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// Luminance mapped from a shadow to a highlight colour (sRGB 0..1), like a duotone.
fn tone_map(s: [f64; 3], h: [f64; 3]) -> Cm {
    let mut m = [0.0; 20];
    for k in 0..3 {
        let d = h[k] - s[k];
        m[k * 5..k * 5 + 5].copy_from_slice(&[d * LR, d * LG, d * LB, 0.0, s[k]]);
    }
    m[18] = 1.0;
    m
}

/// The matrix of a filter preset (`PRESETS[preset]`); `None` for presets this build does not know.
pub fn preset(p: &ImageFilterPreset) -> Option<Cm> {
    use ImageFilterPreset as F;
    let c = |a: Cm, b: Cm| concat(&a, &b);
    Some(match p {
        F::None => IDENTITY_CM,
        F::Mono => saturation(0.0),
        F::Sepia => SEPIA,
        F::Vivid => c(contrast(1.1), saturation(1.45)),
        F::Fade => c(brightness(0.06), c(contrast(0.78), saturation(0.8))),
        F::Cool => c(temperature(-0.07, 0.0), saturation(1.05)),
        F::Warm => c(temperature(0.08, -0.01), saturation(1.08)),
        F::Noir => c(contrast(1.45), saturation(0.0)),
        F::Retro => c(brightness(0.04), c(contrast(0.9), lerp(&IDENTITY_CM, &SEPIA, 0.6))),
        F::Chrome => c(contrast(1.18), c(saturation(0.85), brightness(0.02))),
        F::Dramatic => c(contrast(1.35), c(saturation(0.75), brightness(-0.04))),
        F::Vintage => c(temperature(0.06, -0.02), c(contrast(0.85), c(saturation(0.7), brightness(0.05)))),
        F::Summer => c(temperature(0.06, 0.0), c(saturation(1.25), brightness(0.04))),
        F::Dusk => c(temperature(0.05, 0.05), c(contrast(1.08), c(saturation(0.9), brightness(-0.03)))),
        F::Lomo => c(contrast(1.3), saturation(1.3)),
        F::Crisp => c(contrast(1.15), saturation(1.12)),
        F::Pastel => c(brightness(0.08), c(contrast(0.75), saturation(0.75))),
        F::Matte => c([1.0, 0.0, 0.0, 0.0, 0.06, 0.0, 1.0, 0.0, 0.0, 0.06, 0.0, 0.0, 1.0, 0.0, 0.06, 0.0, 0.0, 0.0, 1.0, 0.0], c(contrast(0.85), saturation(0.9))),
        F::TealOrange => c(contrast(1.1), [1.12, 0.0, 0.0, 0.0, 0.02, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.85, 0.0, 0.05, 0.0, 0.0, 0.0, 1.0, 0.0]),
        F::Blush => c(temperature(0.03, -0.06), c(saturation(1.05), brightness(0.04))),
        F::Forest => c(temperature(-0.02, 0.05), c(saturation(1.1), contrast(1.05))),
        F::Silver => c(contrast(0.9), c(brightness(0.04), saturation(0.0))),
        F::Contrast => c(contrast(1.7), saturation(0.0)),
        F::Selenium => c(temperature(-0.03, 0.04), saturation(0.0)),
        F::Cyanotype => tone_map([0.04, 0.15, 0.35], [0.85, 0.95, 1.0]),
        F::Invert => INVERT,
        F::Infrared => c(contrast(1.15), [-0.2, 1.4, -0.2, 0.0, 0.0, 0.1, 0.9, 0.0, 0.0, 0.0, 0.3, 0.6, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
        F::Golden => c(temperature(0.1, -0.02), c(saturation(1.15), contrast(1.05))),
        F::Arctic => c(temperature(-0.1, 0.0), c(brightness(0.05), saturation(0.85))),
        F::Rose => lerp(&IDENTITY_CM, &tone_map([0.25, 0.05, 0.12], [1.0, 0.86, 0.9]), 0.7),
        F::Mint => lerp(&IDENTITY_CM, &tone_map([0.03, 0.2, 0.18], [0.85, 1.0, 0.94]), 0.6),
        F::Lavender => lerp(&IDENTITY_CM, &tone_map([0.18, 0.1, 0.3], [0.95, 0.9, 1.0]), 0.6),
        F::Sunset => lerp(&IDENTITY_CM, &tone_map([0.3, 0.05, 0.15], [1.0, 0.85, 0.5]), 0.65),
        F::Duo => tone_map([0.1, 0.1, 0.35], [1.0, 0.5, 0.4]),
        F::Neon => tone_map([0.35, 0.0, 0.45], [0.3, 1.0, 0.85]),
        F::Other(_) => return None,
    })
}

/// Duotone: luminance → linear interpolation from shadow to highlight colour.
pub fn duotone_matrix(d: &Duotone) -> Cm {
    let s = to_rgba(&d.shadow);
    let h = to_rgba(&d.highlight);
    let row = |a: f64, b: f64| [(b - a) * LR, (b - a) * LG, (b - a) * LB, 0.0, a];
    let (r, g, b) = (row(s.r, h.r), row(s.g, h.g), row(s.b, h.b));
    let mut m = [0.0; 20];
    m[0..5].copy_from_slice(&r);
    m[5..10].copy_from_slice(&g);
    m[10..15].copy_from_slice(&b);
    m[18] = 1.0;
    m
}

fn nz(v: Option<f64>) -> bool {
    v.is_some_and(crate::jsfmt::truthy)
}

pub fn adjust_matrix(a: Option<&ImageAdjust>) -> Cm {
    let Some(a) = a else { return IDENTITY_CM };
    let mut m = IDENTITY_CM;
    if let Some(v) = a.brightness.filter(|v| crate::jsfmt::truthy(*v)) {
        m = concat(&brightness((v / 100.0) * 0.35), &m);
    }
    if let Some(v) = a.contrast.filter(|v| crate::jsfmt::truthy(*v)) {
        m = concat(&contrast(1.0 + v / 100.0), &m);
    }
    if let Some(v) = a.saturation.filter(|v| crate::jsfmt::truthy(*v)) {
        m = concat(&saturation(1.0 + v / 100.0), &m);
    }
    if nz(a.temperature) || nz(a.tint) {
        m = concat(&temperature((a.temperature.unwrap_or(0.0) / 100.0) * 0.12, (a.tint.unwrap_or(0.0) / 100.0) * 0.08), &m);
    }
    if nz(a.whites) || nz(a.blacks) {
        // Levels: move the black and white points (positive blacks deepen, positive whites brighten).
        let b = (a.blacks.unwrap_or(0.0) / 100.0) * 0.2;
        let w = 1.0 - (a.whites.unwrap_or(0.0) / 100.0) * 0.2;
        let k = 1.0 / crate::jsfmt::max(0.05, w - b);
        m = concat(&[k, 0.0, 0.0, 0.0, -b * k, 0.0, k, 0.0, 0.0, -b * k, 0.0, 0.0, k, 0.0, -b * k, 0.0, 0.0, 0.0, 1.0, 0.0], &m);
    }
    if let Some(v) = a.fade.filter(|v| crate::jsfmt::truthy(*v)) {
        // Matte: lift the blacks and soften contrast.
        let f = (v / 100.0) * 0.3;
        m = concat(&[1.0 - f, 0.0, 0.0, 0.0, f * 0.45, 0.0, 1.0 - f, 0.0, 0.0, f * 0.45, 0.0, 0.0, 1.0 - f, 0.0, f * 0.45, 0.0, 0.0, 0.0, 1.0, 0.0], &m);
    }
    m
}

/// The full colour matrix for an edit stack, or `None` when it is the identity.
pub fn edit_matrix(edits: Option<&ImageEditStack>) -> Option<Cm> {
    let edits = edits?;
    let mut m = adjust_matrix(edits.adjust.as_ref());
    if let Some(f) = &edits.filter
        && f.preset != ImageFilterPreset::None
        && let Some(p) = preset(&f.preset)
    {
        m = concat(&lerp(&IDENTITY_CM, &p, f.amount), &m);
    }
    if let Some(d) = &edits.duotone
        && d.amount > 0.0
    {
        m = concat(&lerp(&IDENTITY_CM, &duotone_matrix(d), d.amount), &m);
    }
    if m.iter().zip(IDENTITY_CM.iter()).all(|(v, i)| (v - i).abs() < 1e-9) { None } else { Some(m) }
}

/// Adjustments that need a per-pixel pass (tone ranges, detail, pixelate), not a colour matrix.
pub fn needs_tone_shader(a: Option<&ImageAdjust>) -> bool {
    has_tone(a) || pixelate_amount(a) > 0.0
}

/// Tone ranges and detail (the SkSL shader's part of the per-pixel pass).
pub fn has_tone(a: Option<&ImageAdjust>) -> bool {
    a.is_some_and(|a| nz(a.highlights) || nz(a.shadows) || nz(a.sharpen) || nz(a.clarity))
}

/// Pixelate (0..100, for redaction): 0 when off.
pub fn pixelate_amount(a: Option<&ImageAdjust>) -> f64 {
    a.and_then(|a| a.pixelate).filter(|v| v.is_finite() && *v > 0.0).map_or(0.0, |v| v.min(100.0))
}

/// Pixelate cell size in image pixels for an image whose longer side is `long` px: about 16 cells across at 100.
pub fn pixelate_cell(amount: f64, long: f64) -> usize {
    if amount <= 0.0 {
        return 1;
    }
    ((long * amount / 100.0 * 0.06).round() as usize).max(2)
}
