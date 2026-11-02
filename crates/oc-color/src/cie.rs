// SPDX-License-Identifier: Apache-2.0
//! CIE colorimetry: linear RGB → XYZ, XYZ ↔ L*a*b* and CIEDE2000, in f64 (moxcms has neither f64 Lab nor CIEDE2000).

/// The ICC profile connection space white (D50), XYZ with Y = 1.
pub const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

pub type M3 = [[f64; 3]; 3];

pub fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// sRGB / Display-P3 transfer function (IEC 61966-2-1), encoded → linear.
pub fn srgb_eotf(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}


/// Encoded RGB with the sRGB curve and the given primaries → XYZ D50.
pub fn rgb_to_xyz(m: &M3, rgb: [f64; 3]) -> [f64; 3] {
    apply(m, [srgb_eotf(rgb[0]), srgb_eotf(rgb[1]), srgb_eotf(rgb[2])])
}

fn f(t: f64) -> f64 {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    if t > E { t.cbrt() } else { (K * t + 16.0) / 116.0 }
}

fn finv(t: f64) -> f64 {
    const E: f64 = 6.0 / 29.0;
    if t > E { t * t * t } else { 3.0 * E * E * (t - 4.0 / 29.0) }
}

/// XYZ (D50, Y = 1) → L*a*b*.
pub fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let (fx, fy, fz) = (f(xyz[0] / D50[0]), f(xyz[1] / D50[1]), f(xyz[2] / D50[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// L*a*b* → XYZ (D50, Y = 1).
pub fn lab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    [finv(fx) * D50[0], finv(fy) * D50[1], finv(fz) * D50[2]]
}

/// CIEDE2000 colour difference (kL = kC = kH = 1).
pub fn delta_e_2000(l1: [f64; 3], l2: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let deg = |r: f64| r * 180.0 / PI;
    let rad = |d: f64| d * PI / 180.0;
    let (lp1, a1, b1) = (l1[0], l1[1], l1[2]);
    let (lp2, a2, b2) = (l2[0], l2[1], l2[2]);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let a1p = a1 * (1.0 + g);
    let a2p = a2 * (1.0 + g);
    let c1p = a1p.hypot(b1);
    let c2p = a2p.hypot(b2);
    let h = |b: f64, a: f64| {
        if b == 0.0 && a == 0.0 {
            0.0
        } else {
            let v = deg(b.atan2(a));
            if v < 0.0 { v + 360.0 } else { v }
        }
    };
    let h1p = h(b1, a1p);
    let h2p = h(b2, a2p);
    let dlp = lp2 - lp1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (rad(dhp) / 2.0).sin();
    let lpm = (lp1 + lp2) / 2.0;
    let cpm = (c1p + c2p) / 2.0;
    let hpm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * rad(hpm - 30.0).cos() + 0.24 * rad(2.0 * hpm).cos() + 0.32 * rad(3.0 * hpm + 6.0).cos()
        - 0.20 * rad(4.0 * hpm - 63.0).cos();
    let dtheta = 30.0 * (-((hpm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cpm.powi(7) / (cpm.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lpm - 50.0).powi(2) / (20.0 + (lpm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cpm;
    let sh = 1.0 + 0.015 * cpm * t;
    let rt = -(rad(2.0 * dtheta)).sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_matrix_matches_icc_v4() {
        // The D50-adapted sRGB matrix ICC v4 profiles carry (rXYZ/gXYZ/bXYZ tags, rounded to s15Fixed16).
        let m = moxcms::ColorProfile::new_srgb().rgb_to_xyz_matrix().v;
        let want = [[0.4360, 0.3851, 0.1431], [0.2225, 0.7169, 0.0606], [0.0139, 0.0971, 0.7139]];
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - want[i][j]).abs() < 2e-3, "{i},{j}: {} vs {}", m[i][j], want[i][j]);
            }
        }
    }

    #[test]
    fn lab_roundtrip_and_white() {
        let w = xyz_to_lab(D50);
        assert!((w[0] - 100.0).abs() < 1e-9 && w[1].abs() < 1e-9 && w[2].abs() < 1e-9);
        let lab = [52.0, 40.0, -30.0];
        let back = xyz_to_lab(lab_to_xyz(lab));
        for k in 0..3 {
            assert!((back[k] - lab[k]).abs() < 1e-9);
        }
    }

    #[test]
    fn de2000_sharma_reference_pairs() {
        // Sharma, Wu, Dalal (2005) test data, pairs 1, 7, 17 and 25.
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
        ];
        for (a, b, de) in cases {
            assert!((delta_e_2000(a, b) - de).abs() < 1e-4, "{a:?} {b:?}: {}", delta_e_2000(a, b));
        }
    }
}
