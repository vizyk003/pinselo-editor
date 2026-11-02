// SPDX-License-Identifier: Apache-2.0
//! Pinselo colour management for print (plan 03 §5.6, R6/R7).
//!
//! - ICC output profiles through **moxcms** (pure Rust, builds for wasm32 so the editor can soft-proof with the same
//!   maths): [`inspect`] validates uploads, [`OutputProfile`] holds a CMYK (or RGB) output profile.
//! - [`Converter`]: sRGB / Display-P3 / L\*a\*b\* → CMYK through the output profile, **relative colorimetric with
//!   black-point compensation** (the default, like Adobe CMM / LittleCMS `cmsFLAGS_BLACKPOINTCOMPENSATION`) or
//!   perceptual; CMYK → L\*a\*b\* for proofing; gamut checks (CIEDE2000 between the wanted and the reproduced
//!   colour); a 33³ LUT for converting images.
//! - [`ink`]: authored [`ColorValue`]s → print inks: CMYK passes through untouched, spot colours become a
//!   Separation with a CMYK alternate, RGB/P3 convert through the profile; [`is_pure_black`] drives the rich-black
//!   rule (K-only, overprinting black text and hairlines).
//!
//! moxcms has no black-point compensation (its option is commented out), so BPC is done here, in the profile
//! connection space: source colours go to XYZ D50 (exact sRGB / P3 matrices, Bradford-adapted like ICC v4
//! profiles), are scaled so the source black (0) lands on the destination black point (detected by a
//! Lab 0 → CMYK → Lab round trip, a/b forced neutral, as LittleCMS does), and go through the profile's
//! colorimetric B2A table. The connection to moxcms is an "XYZ carrier": a matrix/TRC RGB profile whose colorants
//! are the D50-scaled XYZ axes and whose curves are a pure 3.0 gamma, so its device values are the cube roots of
//! white-relative XYZ — L\*-like spacing, so moxcms' 33³ grid keeps resolution in the shadows (a linear carrier doesn't, and
//! moxcms' generic Lab profile is an identity on XYZ and can't be a destination). Without a profile, callers keep the naive conversions in
//! `oc_schema::color` (goldens stay valid).

pub mod cie;
pub mod registry;

use std::sync::{Arc, OnceLock};

use moxcms::{ColorProfile, DataColorSpace, Layout, ProfileClass, ProfileText, RenderingIntent, TransformF32Executor, TransformOptions, Xyzd};
use oc_schema::primitives::ColorValue;

#[derive(Debug, thiserror::Error)]
pub enum ColorError {
    #[error("not a valid ICC profile: {0}")]
    Invalid(String),
    #[error("unknown colour profile `{0}`")]
    UnknownProfile(String),
    #[error("colour profile not installed: {0}")]
    Missing(String),
    #[error("colour transform: {0}")]
    Cms(String),
    #[error("{0}")]
    Unsupported(String),
}

/// What an ICC profile is (for upload validation and the profile list).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IccInfo {
    /// Profile class signature: `prtr` (output), `mntr`, `scnr`, `link`, `spac`, `abst`, `nmcl`.
    pub class: String,
    /// `CMYK`, `RGB`, `GRAY`, `Lab`, … (the data colour space).
    #[serde(rename = "colorSpace")]
    pub color_space: String,
    pub channels: u8,
    pub description: String,
    /// `2.1`, `4.3`, …
    pub version: String,
}

fn class_sig(c: ProfileClass) -> &'static str {
    match c {
        ProfileClass::InputDevice => "scnr",
        ProfileClass::DisplayDevice => "mntr",
        ProfileClass::OutputDevice => "prtr",
        ProfileClass::DeviceLink => "link",
        ProfileClass::ColorSpace => "spac",
        ProfileClass::Abstract => "abst",
        ProfileClass::Named => "nmcl",
    }
}

fn space_name(c: DataColorSpace) -> (&'static str, u8) {
    match c {
        DataColorSpace::Cmyk => ("CMYK", 4),
        DataColorSpace::Rgb => ("RGB", 3),
        DataColorSpace::Gray => ("GRAY", 1),
        DataColorSpace::Lab => ("Lab", 3),
        DataColorSpace::Xyz => ("XYZ", 3),
        DataColorSpace::Cmy => ("CMY", 3),
        DataColorSpace::Color2 => ("2CLR", 2),
        DataColorSpace::Color3 => ("3CLR", 3),
        DataColorSpace::Color4 => ("4CLR", 4),
        DataColorSpace::Color5 => ("5CLR", 5),
        DataColorSpace::Color6 => ("6CLR", 6),
        DataColorSpace::Color7 => ("7CLR", 7),
        DataColorSpace::Color8 => ("8CLR", 8),
        _ => ("other", 3),
    }
}

fn text(t: &Option<ProfileText>) -> String {
    match t {
        Some(ProfileText::PlainString(s)) => s.clone(),
        Some(ProfileText::Localizable(v)) => v.iter().find(|l| l.language == "en").or(v.first()).map(|l| l.value.clone()).unwrap_or_default(),
        Some(ProfileText::Description(d)) => {
            if d.ascii_string.trim().is_empty() {
                d.unicode_string.clone()
            } else {
                d.ascii_string.clone()
            }
        }
        None => String::new(),
    }
    .trim_matches(char::from(0))
    .trim()
    .to_string()
}

fn parse(bytes: &[u8]) -> Result<ColorProfile, ColorError> {
    if bytes.len() < 132 || &bytes[36..40] != b"acsp" {
        return Err(ColorError::Invalid("missing the ICC header signature".into()));
    }
    let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if declared > bytes.len() {
        return Err(ColorError::Invalid(format!("header says {declared} bytes, file has {}", bytes.len())));
    }
    ColorProfile::new_from_slice(bytes).map_err(|e| ColorError::Invalid(format!("{e:?}")))
}

/// Read and describe an ICC profile (validates the header and tag table with moxcms).
pub fn inspect(bytes: &[u8]) -> Result<IccInfo, ColorError> {
    let p = parse(bytes)?;
    let (cs, n) = space_name(p.color_space);
    let v = bytes[8];
    let minor = bytes[9] >> 4;
    Ok(IccInfo { class: class_sig(p.profile_class).into(), color_space: cs.into(), channels: n, description: text(&p.description), version: format!("{v}.{minor}") })
}

/// Rendering intent for RGB/Lab → CMYK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Intent {
    /// Relative colorimetric with black-point compensation (default; what Acrobat/InDesign use for print).
    #[default]
    Relative,
    /// The profile's perceptual tables (photographs with out-of-gamut colours).
    Perceptual,
}

impl Intent {
    pub fn parse(s: &str) -> Intent {
        match s {
            "perceptual" => Intent::Perceptual,
            _ => Intent::Relative,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Relative => "relative",
            Intent::Perceptual => "perceptual",
        }
    }
}

/// A lazily built converter (or why it couldn't be built).
type ConverterSlot = OnceLock<Result<Arc<Converter>, String>>;

/// An output profile (the characterised printing condition).
#[derive(Clone)]
pub struct OutputProfile {
    bytes: Arc<Vec<u8>>,
    profile: Arc<ColorProfile>,
    pub info: IccInfo,
    converters: Arc<[ConverterSlot; 2]>,
}

impl std::fmt::Debug for OutputProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputProfile").field("info", &self.info).field("bytes", &self.bytes.len()).finish()
    }
}

impl OutputProfile {
    /// Load an output profile: CMYK (any device class that can be a destination) or RGB.
    pub fn new(bytes: Vec<u8>) -> Result<OutputProfile, ColorError> {
        let profile = parse(&bytes)?;
        let info = inspect(&bytes)?;
        if !matches!(profile.color_space, DataColorSpace::Cmyk | DataColorSpace::Rgb) {
            return Err(ColorError::Unsupported(format!("{} profiles can't be print output profiles", info.color_space)));
        }
        if matches!(profile.profile_class, ProfileClass::DeviceLink | ProfileClass::Abstract | ProfileClass::Named) {
            return Err(ColorError::Unsupported(format!("`{}` (device link / abstract / named) profiles can't be output profiles", info.class)));
        }
        Ok(OutputProfile { bytes: Arc::new(bytes), profile: Arc::new(profile), info, converters: Arc::new([OnceLock::new(), OnceLock::new()]) })
    }

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    pub fn is_cmyk(&self) -> bool {
        self.profile.color_space == DataColorSpace::Cmyk
    }

    /// The (cached) converter for an intent. CMYK profiles only.
    pub fn converter(&self, intent: Intent) -> Result<Arc<Converter>, ColorError> {
        let slot = &self.converters[intent as usize];
        slot.get_or_init(|| Converter::new(&self.profile, intent).map(Arc::new).map_err(|e| e.to_string())).clone().map_err(ColorError::Cms)
    }
}

/// A colour-managed conversion into one CMYK output profile.
pub struct Converter {
    intent: Intent,
    xyz_to_cmyk: Arc<TransformF32Executor>,
    cmyk_to_xyz: Arc<TransformF32Executor>,
    /// Destination black point (XYZ D50) for BPC; `None` for perceptual.
    black: Option<[f64; 3]>,
    /// The profile's darkest neutral (the CMYK of L\* 0) and its L\*: the shadow ramp ends exactly on it.
    max_black: Option<([f64; 4], f64)>,
    srgb: cie::M3,
    p3: cie::M3,
    lut: OnceLock<Vec<[f32; 4]>>,
}

const GRID: usize = 33;
/// L\* range above the black point over which shadows blend into the profile's darkest neutral.
const SHADOW: f64 = 4.0;

/// An RGB profile whose colorants are the D50-scaled XYZ axes with gamma-3 curves: device RGB = ∛(XYZ / D50), so
/// white and black are exact grid corners.
fn xyz_carrier() -> ColorProfile {
    let mut p = ColorProfile::new_aces_cg_linear();
    let curve = moxcms::ToneReprCurve::Parametric(vec![3.0]);
    p.red_trc = Some(curve.clone());
    p.green_trc = Some(curve.clone());
    p.blue_trc = Some(curve);
    p.red_colorant = Xyzd { x: cie::D50[0], y: 0.0, z: 0.0 };
    p.green_colorant = Xyzd { x: 0.0, y: cie::D50[1], z: 0.0 };
    p.blue_colorant = Xyzd { x: 0.0, y: 0.0, z: cie::D50[2] };
    p.media_white_point = Some(Xyzd { x: cie::D50[0], y: cie::D50[1], z: cie::D50[2] });
    p.white_point = Xyzd { x: cie::D50[0], y: cie::D50[1], z: cie::D50[2] };
    p.cicp = None;
    p
}

fn opts(intent: RenderingIntent) -> TransformOptions {
    TransformOptions { rendering_intent: intent, prefer_fixed_point: false, ..TransformOptions::default() }
}

impl Converter {
    fn new(dest: &ColorProfile, intent: Intent) -> Result<Converter, ColorError> {
        if dest.color_space != DataColorSpace::Cmyk {
            return Err(ColorError::Unsupported("colour conversion needs a CMYK output profile".into()));
        }
        let xyz = xyz_carrier();
        let ri = match intent {
            Intent::Relative => RenderingIntent::RelativeColorimetric,
            Intent::Perceptual => RenderingIntent::Perceptual,
        };
        let e = |e: moxcms::CmsError| ColorError::Cms(format!("{e:?}"));
        let o = TransformOptions { allow_use_cicp_transfer: false, ..opts(ri) };
        let xyz_to_cmyk = xyz.create_transform_f32(Layout::Rgb, dest, Layout::Rgba, o).map_err(e)?;
        // Proofing is always colorimetric: what the inks will look like.
        let o = TransformOptions { allow_use_cicp_transfer: false, ..opts(RenderingIntent::RelativeColorimetric) };
        let cmyk_to_xyz = dest.create_transform_f32(Layout::Rgba, &xyz, Layout::Rgb, o).map_err(e)?;
        let mut c = Converter {
            intent,
            xyz_to_cmyk,
            cmyk_to_xyz,
            black: None,
            max_black: None,
            srgb: ColorProfile::new_srgb().rgb_to_xyz_matrix().v,
            p3: ColorProfile::new_display_p3().rgb_to_xyz_matrix().v,
            lut: OnceLock::new(),
        };
        if intent == Intent::Relative {
            // Destination black point: the darkest neutral the profile reproduces (Lab 0 → CMYK → Lab, a/b → 0).
            let k = c.raw_lab_to_cmyk([0.0, 0.0, 0.0]);
            let l = c.cmyk_to_lab(k)[0].clamp(0.0, 50.0);
            c.black = Some(cie::lab_to_xyz([l, 0.0, 0.0]));
            c.max_black = Some((k, l));
        }
        Ok(c)
    }

    pub fn intent(&self) -> Intent {
        self.intent
    }

    /// The destination black point (L\*) BPC maps source black to (relative intent).
    pub fn black_point_l(&self) -> Option<f64> {
        self.black.map(|b| cie::xyz_to_lab(b)[0])
    }

    /// Lab (already black-point compensated) → CMYK. Within [`SHADOW`] L\* above the black point the result is
    /// blended towards the profile's darkest neutral: the B2A tables clip below their black point and a 33³ grid
    /// interpolates across that edge, which would otherwise print source black a few L\* light.
    fn compensated_lab_to_cmyk(&self, lab: [f64; 3]) -> [f64; 4] {
        if let Some((k, bl)) = self.max_black
            && lab[0] < bl + SHADOW
        {
            let top = bl + SHADOW;
            let t = ((lab[0] - bl) / SHADOW).clamp(0.0, 1.0);
            let s = (top / lab[0].max(1e-6)).min(1.0e3);
            // Keep the hue: scale a/b with L up to the ramp top.
            let hi = self.raw_lab_to_cmyk([top, (lab[1] * s).clamp(-128.0, 127.0), (lab[2] * s).clamp(-128.0, 127.0)]);
            return [0, 1, 2, 3].map(|i| k[i] * (1.0 - t) + hi[i] * t);
        }
        self.raw_lab_to_cmyk(lab)
    }

    fn raw_lab_to_cmyk(&self, lab: [f64; 3]) -> [f64; 4] {
        let x = cie::lab_to_xyz(lab);
        let src = [0, 1, 2].map(|i| (x[i] / cie::D50[i]).max(0.0).cbrt() as f32);
        let mut dst = [0f32; 4];
        if self.xyz_to_cmyk.transform(&src, &mut dst).is_err() {
            return [0.0, 0.0, 0.0, 1.0];
        }
        dst.map(|v| f64::from(v).clamp(0.0, 1.0))
    }

    /// Black-point compensation in XYZ (source black 0 → destination black, white fixed).
    fn bpc(&self, xyz: [f64; 3]) -> [f64; 3] {
        match self.black {
            Some(b) => [0, 1, 2].map(|i| xyz[i] * (1.0 - b[i] / cie::D50[i]) + b[i]),
            None => xyz,
        }
    }

    /// The colour the conversion aims for (L\*a\*b\* after BPC).
    pub fn target_lab(&self, lab: [f64; 3]) -> [f64; 3] {
        cie::xyz_to_lab(self.bpc(cie::lab_to_xyz(lab)))
    }

    /// L\*a\*b\* (D50) → CMYK (0..1).
    pub fn lab_to_cmyk(&self, lab: [f64; 3]) -> [f64; 4] {
        self.compensated_lab_to_cmyk(self.target_lab(lab))
    }

    /// XYZ (D50) → CMYK.
    pub fn xyz_to_cmyk(&self, xyz: [f64; 3]) -> [f64; 4] {
        self.compensated_lab_to_cmyk(cie::xyz_to_lab(self.bpc(xyz)))
    }

    pub fn srgb_to_lab(&self, rgb: [f64; 3]) -> [f64; 3] {
        cie::xyz_to_lab(cie::rgb_to_xyz(&self.srgb, rgb))
    }

    pub fn p3_to_lab(&self, rgb: [f64; 3]) -> [f64; 3] {
        cie::xyz_to_lab(cie::rgb_to_xyz(&self.p3, rgb))
    }

    pub fn srgb_to_cmyk(&self, rgb: [f64; 3]) -> [f64; 4] {
        self.xyz_to_cmyk(cie::rgb_to_xyz(&self.srgb, rgb))
    }

    pub fn p3_to_cmyk(&self, rgb: [f64; 3]) -> [f64; 4] {
        self.xyz_to_cmyk(cie::rgb_to_xyz(&self.p3, rgb))
    }

    /// CMYK → L\*a\*b\* (relative colorimetric: what the inks reproduce, paper = white).
    pub fn cmyk_to_lab(&self, cmyk: [f64; 4]) -> [f64; 3] {
        let src = cmyk.map(|v| v.clamp(0.0, 1.0) as f32);
        let mut dst = [0f32; 3];
        if self.cmyk_to_xyz.transform(&src, &mut dst).is_err() {
            return [0.0, 0.0, 0.0];
        }
        cie::xyz_to_lab([0, 1, 2].map(|i| f64::from(dst[i]).max(0.0).powi(3) * cie::D50[i]))
    }

    /// Gamut check for a wanted L\*a\*b\*: the CMYK it converts to and how far the print lands from it (ΔE00).
    pub fn gamut_lab(&self, lab: [f64; 3]) -> Gamut {
        let target = self.target_lab(lab);
        let cmyk = self.compensated_lab_to_cmyk(target);
        let got = self.cmyk_to_lab(cmyk);
        let delta_e = cie::delta_e_2000(target, got);
        Gamut { cmyk, delta_e, in_gamut: delta_e <= GAMUT_DE }
    }

    /// Gamut check for an authored colour. CMYK and spot colours are in gamut by definition.
    pub fn gamut(&self, c: &ColorValue) -> Gamut {
        match c {
            ColorValue::Srgb(x) => self.gamut_lab(self.srgb_to_lab([x.r, x.g, x.b])),
            ColorValue::DisplayP3(x) => self.gamut_lab(self.p3_to_lab([x.r, x.g, x.b])),
            ColorValue::Cmyk(x) => Gamut { cmyk: [x.c, x.m, x.y, x.k].map(|v| v.clamp(0.0, 1.0)), delta_e: 0.0, in_gamut: true },
            ColorValue::Spot(_) => Gamut { cmyk: [0.0; 4], delta_e: 0.0, in_gamut: true },
        }
    }

    fn lut(&self) -> &[[f32; 4]] {
        self.lut.get_or_init(|| {
            let mut v = Vec::with_capacity(GRID * GRID * GRID);
            let s = (GRID - 1) as f64;
            for r in 0..GRID {
                for g in 0..GRID {
                    for b in 0..GRID {
                        let c = self.srgb_to_cmyk([r as f64 / s, g as f64 / s, b as f64 / s]);
                        v.push(c.map(|x| x as f32));
                    }
                }
            }
            v
        })
    }

    /// Convert interleaved 8-bit sRGB pixels to 8-bit CMYK (33³ LUT, trilinear), for images.
    pub fn srgb8_to_cmyk8(&self, rgb: &[u8]) -> Vec<u8> {
        let lut = self.lut();
        let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
        let s = (GRID - 1) as f32 / 255.0;
        let at = |r: usize, g: usize, b: usize| &lut[(r * GRID + g) * GRID + b];
        let mut last: Option<([u8; 3], [u8; 4])> = None;
        for px in rgb.as_chunks::<3>().0 {
            let key = [px[0], px[1], px[2]];
            if let Some((k, v)) = last
                && k == key
            {
                out.extend_from_slice(&v);
                continue;
            }
            let (fr, fg, fb) = (f32::from(px[0]) * s, f32::from(px[1]) * s, f32::from(px[2]) * s);
            let (r0, g0, b0) = ((fr as usize).min(GRID - 2), (fg as usize).min(GRID - 2), (fb as usize).min(GRID - 2));
            let (dr, dg, db) = (fr - r0 as f32, fg - g0 as f32, fb - b0 as f32);
            let mut c = [0f32; 4];
            for (i, ci) in c.iter_mut().enumerate() {
                let l = |r, g, b| at(r, g, b)[i];
                let c00 = l(r0, g0, b0) * (1.0 - db) + l(r0, g0, b0 + 1) * db;
                let c01 = l(r0, g0 + 1, b0) * (1.0 - db) + l(r0, g0 + 1, b0 + 1) * db;
                let c10 = l(r0 + 1, g0, b0) * (1.0 - db) + l(r0 + 1, g0, b0 + 1) * db;
                let c11 = l(r0 + 1, g0 + 1, b0) * (1.0 - db) + l(r0 + 1, g0 + 1, b0 + 1) * db;
                let c0 = c00 * (1.0 - dg) + c01 * dg;
                let c1 = c10 * (1.0 - dg) + c11 * dg;
                *ci = c0 * (1.0 - dr) + c1 * dr;
            }
            let v = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
            last = Some((key, v));
            out.extend_from_slice(&v);
        }
        out
    }
}

/// ΔE00 above which a colour counts as out of gamut (clearly visible difference).
pub const GAMUT_DE: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Gamut {
    /// CMYK 0..1.
    pub cmyk: [f64; 4],
    #[serde(rename = "deltaE")]
    pub delta_e: f64,
    #[serde(rename = "inGamut")]
    pub in_gamut: bool,
}

/// A print ink for an authored colour.
#[derive(Debug, Clone, PartialEq)]
pub enum Ink {
    /// Process CMYK, 0..1.
    Cmyk([f64; 4]),
    /// A spot colour: Separation `name` at `tint`, with a full-strength CMYK alternate.
    Spot { name: String, tint: f64, alternate: [f64; 4] },
}

/// CMYK alternates for spot colours (swatch name → CMYK 0..1). Unknown swatches fall back to 50 % K, the same
/// neutral the naive display conversion shows.
pub type SpotAlternates = std::collections::HashMap<String, [f64; 4]>;

pub const SPOT_FALLBACK: [f64; 4] = [0.0, 0.0, 0.0, 0.5];

/// Authored colour → print ink. CMYK passes through exactly (no round trip); spot stays a separation; RGB and P3
/// go through the profile (`conv`), or the naive conversion when there is no profile.
pub fn ink(c: &ColorValue, conv: Option<&Converter>, spots: &SpotAlternates) -> Ink {
    match c {
        ColorValue::Cmyk(x) => Ink::Cmyk([x.c, x.m, x.y, x.k].map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })),
        ColorValue::Spot(s) => Ink::Spot { name: s.swatch.clone(), tint: if s.tint.is_nan() { 1.0 } else { s.tint.clamp(0.0, 1.0) }, alternate: spots.get(&s.swatch).copied().unwrap_or(SPOT_FALLBACK) },
        ColorValue::Srgb(x) => Ink::Cmyk(match conv {
            Some(cv) => cv.srgb_to_cmyk([x.r, x.g, x.b]),
            None => naive_cmyk([x.r, x.g, x.b]),
        }),
        ColorValue::DisplayP3(x) => Ink::Cmyk(match conv {
            Some(cv) => cv.p3_to_cmyk([x.r, x.g, x.b]),
            None => naive_cmyk([x.r, x.g, x.b]),
        }),
    }
}

/// The inverse of `oc_schema::color::to_rgba`'s naive CMYK → RGB (full GCR), for the no-profile fallback.
pub fn naive_cmyk(rgb: [f64; 3]) -> [f64; 4] {
    let [r, g, b] = rgb.map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) });
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k]
}

/// Pure black (RGB/P3 0,0,0 or CMYK 0/0/0/100): becomes K-only, overprinting black for text and hairlines.
pub fn is_pure_black(c: &ColorValue) -> bool {
    const E: f64 = 0.5 / 255.0;
    match c {
        ColorValue::Srgb(x) | ColorValue::DisplayP3(x) => x.r <= E && x.g <= E && x.b <= E && x.a >= 0.999,
        ColorValue::Cmyk(x) => x.c <= E && x.m <= E && x.y <= E && x.k >= 1.0 - E && x.a >= 0.999,
        ColorValue::Spot(_) => false,
    }
}

#[cfg(test)]
mod tests;
