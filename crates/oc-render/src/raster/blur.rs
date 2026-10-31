// SPDX-License-Identifier: Apache-2.0
//! Gaussian blur of premultiplied RGBA and of coverage masks. Small sigmas use the exact kernel; larger ones the
//! triple box blur Skia's CPU blur uses (window `⌊σ·3·√(2π)/4 + ½⌋`, the SVG filter-effects approximation).

/// What lies outside the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// Transparent (Skia `TileMode::Decal`).
    Transparent,
    /// The edge pixel repeats (Skia `TileMode::Clamp`).
    Clamp,
}

/// A run of `n` samples starting at `start`, `stride` apart, of `ch` interleaved channels.
#[derive(Clone, Copy)]
struct Line {
    start: usize,
    n: usize,
    stride: usize,
    ch: usize,
}

/// One 1-D blur pass along a line.
fn pass(buf: &mut [f32], tmp: &mut Vec<f32>, line: Line, sigma: f32, edge: Edge) {
    let Line { start, n, stride, ch } = line;
    if n == 0 {
        return;
    }
    tmp.clear();
    tmp.reserve(n * ch);
    for i in 0..n {
        let o = start + i * stride;
        tmp.extend_from_slice(&buf[o..o + ch]);
    }
    let get = |tmp: &[f32], i: isize, c: usize| -> f32 {
        if i < 0 || i >= n as isize {
            match edge {
                Edge::Transparent => 0.0,
                Edge::Clamp => tmp[(i.clamp(0, n as isize - 1) as usize) * ch + c],
            }
        } else {
            tmp[i as usize * ch + c]
        }
    };
    if sigma < 2.0 {
        let r = (sigma * 3.0).ceil() as isize;
        let mut k: Vec<f32> = (-r..=r).map(|x| (-((x * x) as f32) / (2.0 * sigma * sigma)).exp()).collect();
        let s: f32 = k.iter().sum();
        k.iter_mut().for_each(|v| *v /= s);
        for i in 0..n as isize {
            for c in 0..ch {
                let mut acc = 0.0;
                for (j, w) in k.iter().enumerate() {
                    acc += w * get(tmp, i + j as isize - r, c);
                }
                buf[start + i as usize * stride + c] = acc;
            }
        }
        return;
    }
    let d = (sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0 + 0.5).floor() as isize;
    let d = d.max(1);
    // (left, right) extents of the three boxes.
    let boxes: [(isize, isize); 3] = if d % 2 == 1 { [((d - 1) / 2, (d - 1) / 2); 3] } else { [(d / 2, d / 2 - 1), (d / 2 - 1, d / 2), (d / 2, d / 2)] };
    let mut src = tmp.clone();
    let mut dst = vec![0.0f32; n * ch];
    for (l, r) in boxes {
        let w = (l + r + 1) as f32;
        for c in 0..ch {
            let mut acc = 0.0f32;
            for j in -l..=r {
                acc += get(&src, j, c);
            }
            for i in 0..n as isize {
                dst[i as usize * ch + c] = acc / w;
                acc += get(&src, i + r + 1, c) - get(&src, i - l, c);
            }
        }
        std::mem::swap(&mut src, &mut dst);
    }
    for i in 0..n {
        let o = start + i * stride;
        buf[o..o + ch].copy_from_slice(&src[i * ch..i * ch + ch]);
    }
}

/// Blur an interleaved float image (`ch` channels) in place.
pub fn blur_f32(buf: &mut [f32], w: usize, h: usize, ch: usize, sx: f32, sy: f32, edge: Edge) {
    let mut tmp = Vec::new();
    if sx > 0.0 {
        for y in 0..h {
            pass(buf, &mut tmp, Line { start: y * w * ch, n: w, stride: ch, ch }, sx, edge);
        }
    }
    if sy > 0.0 {
        for x in 0..w {
            pass(buf, &mut tmp, Line { start: x * ch, n: h, stride: w * ch, ch }, sy, edge);
        }
    }
}

/// Blur premultiplied RGBA8 pixels in place.
pub fn blur_rgba8(data: &mut [u8], w: usize, h: usize, sx: f32, sy: f32, edge: Edge) {
    if (sx <= 0.0 && sy <= 0.0) || w == 0 || h == 0 {
        return;
    }
    let mut f: Vec<f32> = data.iter().map(|v| f32::from(*v)).collect();
    blur_f32(&mut f, w, h, 4, sx, sy, edge);
    for (o, v) in data.as_chunks_mut::<4>().0.iter_mut().zip(f.as_chunks::<4>().0.iter()) {
        let a = v[3].round().clamp(0.0, 255.0);
        o[3] = a as u8;
        for c in 0..3 {
            o[c] = v[c].round().clamp(0.0, a) as u8;
        }
    }
}

/// Blur a coverage mask (0..=1 floats) in place.
pub fn blur_alpha(buf: &mut [f32], w: usize, h: usize, sx: f32, sy: f32, edge: Edge) {
    if (sx <= 0.0 && sy <= 0.0) || w == 0 || h == 0 {
        return;
    }
    blur_f32(buf, w, h, 1, sx, sy, edge);
}

/// How far (in pixels) a blur of `sigma` reaches.
pub fn extent(sigma: f32) -> i32 {
    if sigma <= 0.0 { 0 } else { (sigma * 3.0).ceil() as i32 + 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conserves_mass_and_spreads() {
        let (w, h) = (41, 1);
        let mut a = vec![0.0f32; w];
        a[20] = 1.0;
        blur_alpha(&mut a, w, h, 4.0, 0.0, Edge::Transparent);
        let s: f32 = a.iter().sum();
        assert!((s - 1.0).abs() < 1e-4, "{s}");
        assert!(a[20] > a[24] && a[24] > a[28] && a[28] > 0.0);
        assert!((a[16] - a[24]).abs() < 1e-3);
        let mut b = vec![0.0f32; w];
        b[20] = 1.0;
        blur_alpha(&mut b, w, h, 1.0, 0.0, Edge::Transparent);
        assert!((b.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn clamp_keeps_flat_images_flat() {
        let mut px = [200u8, 100, 50, 255].repeat(16 * 8);
        blur_rgba8(&mut px, 16, 8, 3.0, 3.0, Edge::Clamp);
        assert!(px.chunks(4).all(|p| p == [200, 100, 50, 255]));
    }
}
