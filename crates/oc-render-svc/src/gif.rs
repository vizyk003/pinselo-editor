// SPDX-License-Identifier: Apache-2.0
//! Port of **gifenc 1.0.3** (the `gifenc` npm package `exporters.ts` encodes GIFs with): the PnnQuant-based
//! `quantize` (rgb565 bins, the default format), `applyPalette` (nearest palette entry, cached per rgb565 key) and
//! `GIFEncoder` (`auto` mode: header, logical screen, global colour table and NETSCAPE loop extension with the first
//! frame, a local colour table on every later frame, LZW image data in 254-byte packets).
//!
//! Faithful down to the floating-point operation order, so palettes, indices and the encoded bytes are identical
//! to the JS for the same RGBA input (`tests/golden/render-video/gifenc.json`). Only the options exportGif uses
//! are ported: `quantize(rgba, maxColors)` with the default options, `applyPalette(rgba, palette)` and
//! `writeFrame(index, w, h, { palette, delay })`.

/// One palette entry (`[r, g, b]`).
pub type Color = [u8; 3];

/// `rgb888_to_rgb565`: gifenc's 16-bit bin key (note: bit 10 is unused, green keeps 5 bits).
fn rgb565(r: u32, g: u32, b: u32) -> usize {
    (((r << 8) & 0xf800) | ((g << 2) & 0x03e0) | (b >> 3)) as usize
}

#[derive(Debug, Clone, Default)]
struct Bin {
    rc: f64,
    gc: f64,
    bc: f64,
    cnt: f64,
    nn: usize,
    fw: usize,
    bk: usize,
    tm: usize,
    mtm: usize,
    err: f64,
}

fn sqr(v: f64) -> f64 {
    v * v
}

/// `find_nn(bins, idx, false)`: the nearest following bin by merge cost.
fn find_nn(bins: &mut [Bin], idx: usize) {
    let mut nn = 0;
    let mut err = 1e100;
    let n1 = bins[idx].cnt;
    let (wr, wg, wb) = (bins[idx].rc, bins[idx].gc, bins[idx].bc);
    let mut i = bins[idx].fw;
    while i != 0 {
        let bin = &bins[i];
        let n2 = bin.cnt;
        let nerr2 = (n1 * n2) / (n1 + n2);
        if nerr2 < err {
            let mut nerr = 0.0;
            nerr += nerr2 * sqr(bin.rc - wr);
            if nerr < err {
                nerr += nerr2 * sqr(bin.gc - wg);
                if nerr < err {
                    nerr += nerr2 * sqr(bin.bc - wb);
                    if nerr < err {
                        err = nerr;
                        nn = i;
                    }
                }
            }
        }
        i = bins[i].fw;
    }
    bins[idx].err = err;
    bins[idx].nn = nn;
}

/// `Math.round` for the non-negative-or-clamped values here, then `clamp(…, 0, 255)`.
fn channel(v: f64) -> u8 {
    oc_schema::js::round(v).clamp(0.0, 255.0) as u8
}

/// `quantize(rgba, maxColors)`: at most `max_colors` colours (fewer when bins collapse to the same rounded colour).
pub fn quantize(rgba: &[u8], max_colors: usize) -> Vec<Color> {
    const BINCOUNT: usize = 65536;
    const DELETED: usize = BINCOUNT - 1;
    // Histogram.
    let mut hist: Vec<Option<Bin>> = vec![None; BINCOUNT];
    for px in rgba.as_chunks::<4>().0 {
        let (r, g, b) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
        let bin = hist[rgb565(r, g, b)].get_or_insert_with(Bin::default);
        bin.rc += f64::from(r);
        bin.gc += f64::from(g);
        bin.bc += f64::from(b);
        bin.cnt += 1.0;
    }
    // Cluster nonempty bins at one end of the array.
    let mut bins: Vec<Bin> = Vec::new();
    for mut bin in hist.into_iter().flatten() {
        let d = 1.0 / bin.cnt;
        bin.rc *= d;
        bin.gc *= d;
        bin.bc *= d;
        bins.push(bin);
    }
    let maxbins = bins.len();
    if maxbins == 0 {
        // The JS reads `bins[0].rc` of an empty histogram and throws.
        return Vec::new();
    }
    let mut use_sqrt = true;
    if sqr(max_colors as f64) / (maxbins as f64) < 0.022 {
        use_sqrt = false;
    }
    let mut i = 0;
    while i + 1 < maxbins {
        bins[i].fw = i + 1;
        bins[i + 1].bk = i;
        if use_sqrt {
            bins[i].cnt = bins[i].cnt.sqrt();
        }
        i += 1;
    }
    if use_sqrt {
        bins[i].cnt = bins[i].cnt.sqrt();
    }

    // Nearest neighbours and a heap of them.
    let mut heap = vec![0usize; BINCOUNT + 1];
    for i in 0..maxbins {
        find_nn(&mut bins, i);
        let err = bins[i].err;
        heap[0] += 1;
        let mut l = heap[0];
        while l > 1 {
            let l2 = l >> 1;
            let h = heap[l2];
            if bins[h].err <= err {
                break;
            }
            heap[l] = h;
            l = l2;
        }
        heap[l] = i;
    }

    // Merge the bins that increase the error least.
    let extbins = maxbins as i64 - max_colors as i64;
    let mut i: usize = 0;
    while (i as i64) < extbins {
        let mut tb;
        loop {
            let mut b1 = heap[1];
            tb = b1;
            if bins[tb].tm >= bins[tb].mtm && bins[bins[tb].nn].mtm <= bins[tb].tm {
                break;
            }
            if bins[tb].mtm == DELETED {
                // Deleted node.
                let last = heap[heap[0]];
                heap[0] -= 1;
                heap[1] = last;
                b1 = last;
            } else {
                // Too old error value.
                find_nn(&mut bins, b1);
                bins[tb].tm = i;
            }
            // Push the slot down.
            let err = bins[b1].err;
            let mut l = 1;
            loop {
                let mut l2 = l + l;
                if l2 > heap[0] {
                    break;
                }
                if l2 < heap[0] && bins[heap[l2]].err > bins[heap[l2 + 1]].err {
                    l2 += 1;
                }
                let h = heap[l2];
                if err <= bins[h].err {
                    break;
                }
                heap[l] = h;
                l = l2;
            }
            heap[l] = b1;
        }
        // Merge.
        let nbi = bins[tb].nn;
        let nb = bins[nbi].clone();
        let t = &mut bins[tb];
        let (n1, n2) = (t.cnt, nb.cnt);
        let d = 1.0 / (n1 + n2);
        t.rc = d * (n1 * t.rc + n2 * nb.rc);
        t.gc = d * (n1 * t.gc + n2 * nb.gc);
        t.bc = d * (n1 * t.bc + n2 * nb.bc);
        t.cnt += nb.cnt;
        i += 1;
        t.mtm = i;
        // Unchain the deleted bin.
        bins[nb.bk].fw = nb.fw;
        bins[nb.fw].bk = nb.bk;
        bins[nbi].mtm = DELETED;
    }

    // Fill the palette (following the chain from the first bin).
    let mut palette: Vec<Color> = Vec::new();
    let mut i = 0;
    loop {
        let c = [
            channel(bins[i].rc),
            channel(bins[i].gc),
            channel(bins[i].bc),
        ];
        if !palette.contains(&c) {
            palette.push(c);
        }
        i = bins[i].fw;
        if i == 0 {
            break;
        }
    }
    palette
}

/// `nearestColorIndexRGB`: the last of the nearest entries (ties go to the later index).
fn nearest(r: i32, g: i32, b: i32, palette: &[Color]) -> u8 {
    let mut k = 0usize;
    let mut mindist = i64::MAX;
    for (i, p) in palette.iter().enumerate() {
        let sq = |a: u8, v: i32| {
            let d = i64::from(i32::from(a) - v);
            d * d
        };
        let mut cur = sq(p[0], r);
        if cur > mindist {
            continue;
        }
        cur += sq(p[1], g);
        if cur > mindist {
            continue;
        }
        cur += sq(p[2], b);
        if cur > mindist {
            continue;
        }
        mindist = cur;
        k = i;
    }
    k as u8
}

/// `applyPalette(rgba, palette)`: palette indices per pixel; the first pixel of each rgb565 bin decides the index
/// for the whole bin (gifenc's cache).
pub fn apply_palette(rgba: &[u8], palette: &[Color]) -> Vec<u8> {
    let mut cache: Vec<Option<u8>> = vec![None; 65536];
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|px| {
            let (r, g, b) = (u32::from(px[0]), u32::from(px[1]), u32::from(px[2]));
            let key = rgb565(r, g, b);
            *cache[key].get_or_insert_with(|| nearest(r as i32, g as i32, b as i32, palette))
        })
        .collect()
}

/// `colorTableSize(length)`: `max(ceil(log2(length)), 1)`.
fn color_table_size(len: usize) -> u32 {
    let mut k = 1;
    while (1usize << k) < len {
        k += 1;
    }
    k
}

const HSIZE: usize = 5003;

/// `GIFEncoder()` in auto mode.
pub struct GifEncoder {
    out: Vec<u8>,
    has_init: bool,
    accum: [u8; 256],
    htab: Vec<i32>,
    codetab: Vec<i32>,
}

impl Default for GifEncoder {
    fn default() -> Self {
        GifEncoder::new()
    }
}

impl GifEncoder {
    pub fn new() -> GifEncoder {
        GifEncoder {
            out: Vec::with_capacity(4096),
            has_init: false,
            accum: [0; 256],
            htab: vec![0; HSIZE],
            codetab: vec![0; HSIZE],
        }
    }

    fn u16(&mut self, v: u32) {
        self.out.push((v & 0xff) as u8);
        self.out.push(((v >> 8) & 0xff) as u8);
    }

    fn color_table(&mut self, palette: &[Color]) {
        let n = 1usize << color_table_size(palette.len());
        for i in 0..n {
            self.out
                .extend_from_slice(palette.get(i).unwrap_or(&[0, 0, 0]));
        }
    }

    /// `writeFrame(index, width, height, { palette, delay })` (no transparency, loop forever, colour depth 8).
    pub fn write_frame(
        &mut self,
        index: &[u8],
        width: u32,
        height: u32,
        palette: Option<&[Color]>,
        delay: f64,
    ) {
        let mut first = false;
        if !self.has_init {
            first = true;
            self.out.extend_from_slice(b"GIF89a");
            self.has_init = true;
        }
        if first {
            // gifenc throws "First frame must include a { palette } option"; exportGif always passes one.
            let palette = palette.unwrap_or(&[]);
            self.u16(width);
            self.u16(height);
            let fields = (1 << 7) | ((8 - 1) << 4) | (color_table_size(palette.len()) - 1);
            self.out.extend_from_slice(&[fields as u8, 0, 0]);
            self.color_table(palette);
            // NETSCAPE2.0, repeat 0 = forever.
            self.out.extend_from_slice(&[0x21, 0xff, 11]);
            self.out.extend_from_slice(b"NETSCAPE2.0");
            self.out.extend_from_slice(&[3, 1]);
            self.u16(0);
            self.out.push(0);
        }
        // Graphic control extension: no disposal, no transparency.
        let delay_time = oc_schema::js::round(delay / 10.0) as u32;
        self.out.extend_from_slice(&[0x21, 0xf9, 4, 0]);
        self.u16(delay_time);
        self.out.extend_from_slice(&[0, 0]);
        // Image descriptor.
        let local = palette.filter(|_| !first);
        self.out.push(0x2c);
        self.u16(0);
        self.u16(0);
        self.u16(width);
        self.u16(height);
        match local {
            Some(p) => {
                self.out.push(0x80 | (color_table_size(p.len()) - 1) as u8);
                self.color_table(p);
            }
            None => self.out.push(0),
        }
        self.lzw(index, 8);
    }

    /// `finish()`: the trailer.
    pub fn finish(&mut self) {
        self.out.push(0x3b);
    }

    /// `bytes()`.
    pub fn bytes(&self) -> Vec<u8> {
        self.out.clone()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.out
    }

    /// `lzwEncode(width, height, pixels, colorDepth, stream, accum, htab, codetab)`.
    fn lzw(&mut self, pixels: &[u8], color_depth: u32) {
        const BITS: i32 = 12;
        let hsize = self.htab.len() as i32;
        let init_code_size = color_depth.max(2) as i32;
        self.accum.fill(0);
        self.codetab.fill(0);
        self.htab.fill(-1);
        let mut st = Lzw {
            out: &mut self.out,
            accum: &mut self.accum,
            cur_accum: 0,
            cur_bits: 0,
            a_count: 0,
            n_bits: init_code_size + 1,
            maxcode: (1 << (init_code_size + 1)) - 1,
            g_init_bits: init_code_size + 1,
            clear_flg: false,
            free_ent: 0,
            eof_code: 0,
        };
        let clear_code = 1 << init_code_size;
        st.eof_code = clear_code + 1;
        st.free_ent = clear_code + 2;
        let mut ent = i32::from(pixels.first().copied().unwrap_or(0));
        let mut hshift = 0;
        let mut fcode = hsize;
        while fcode < 65536 {
            hshift += 1;
            fcode *= 2;
        }
        hshift = 8 - hshift;
        st.out.push(init_code_size as u8);
        st.output(clear_code);
        let htab = &mut self.htab;
        let codetab = &mut self.codetab;
        'pixels: for &px in pixels.iter().skip(1) {
            let c = i32::from(px);
            let fcode = (c << BITS) + ent;
            let mut i = (c << hshift) ^ ent;
            if htab[i as usize] == fcode {
                ent = codetab[i as usize];
                continue;
            }
            let disp = if i == 0 { 1 } else { hsize - i };
            while htab[i as usize] >= 0 {
                i -= disp;
                if i < 0 {
                    i += hsize;
                }
                if htab[i as usize] == fcode {
                    ent = codetab[i as usize];
                    continue 'pixels;
                }
            }
            st.output(ent);
            ent = c;
            if st.free_ent < 1 << BITS {
                codetab[i as usize] = st.free_ent;
                st.free_ent += 1;
                htab[i as usize] = fcode;
            } else {
                htab.fill(-1);
                st.free_ent = clear_code + 2;
                st.clear_flg = true;
                st.output(clear_code);
            }
        }
        st.output(ent);
        let eof = st.eof_code;
        st.output(eof);
        st.out.push(0);
    }
}

const MASKS: [i32; 17] = [
    0x0000, 0x0001, 0x0003, 0x0007, 0x000f, 0x001f, 0x003f, 0x007f, 0x00ff, 0x01ff, 0x03ff, 0x07ff,
    0x0fff, 0x1fff, 0x3fff, 0x7fff, 0xffff,
];

struct Lzw<'a> {
    out: &'a mut Vec<u8>,
    accum: &'a mut [u8; 256],
    cur_accum: i32,
    cur_bits: i32,
    a_count: usize,
    n_bits: i32,
    maxcode: i32,
    g_init_bits: i32,
    clear_flg: bool,
    free_ent: i32,
    eof_code: i32,
}

impl Lzw<'_> {
    fn char_out(&mut self) {
        self.accum[self.a_count] = (self.cur_accum & 0xff) as u8;
        self.a_count += 1;
        if self.a_count >= 254 {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.out.push(self.a_count as u8);
        self.out.extend_from_slice(&self.accum[..self.a_count]);
        self.a_count = 0;
    }

    fn output(&mut self, code: i32) {
        self.cur_accum &= MASKS[self.cur_bits as usize];
        if self.cur_bits > 0 {
            self.cur_accum |= code << self.cur_bits;
        } else {
            self.cur_accum = code;
        }
        self.cur_bits += self.n_bits;
        while self.cur_bits >= 8 {
            self.char_out();
            self.cur_accum >>= 8;
            self.cur_bits -= 8;
        }
        if self.free_ent > self.maxcode || self.clear_flg {
            if self.clear_flg {
                self.n_bits = self.g_init_bits;
                self.maxcode = (1 << self.n_bits) - 1;
                self.clear_flg = false;
            } else {
                self.n_bits += 1;
                self.maxcode = if self.n_bits == 12 {
                    1 << self.n_bits
                } else {
                    (1 << self.n_bits) - 1
                };
            }
        }
        if code == self.eof_code {
            while self.cur_bits > 0 {
                self.char_out();
                self.cur_accum >>= 8;
                self.cur_bits -= 8;
            }
            if self.a_count > 0 {
                self.flush();
            }
        }
    }
}

/// The palette-reuse key of exportGif (`${len}:${p[1000]}:${p[len/2]}:${p[len-1000]}`).
pub type FrameKey = (usize, Option<u8>, Option<u8>, Option<u8>);

/// [`FrameKey`] of a frame (`undefined` outside the buffer = `None`).
pub fn frame_key(p: &[u8]) -> FrameKey {
    let at = |i: i64| {
        if i < 0 {
            None
        } else {
            p.get(i as usize).copied()
        }
    };
    let n = p.len() as i64;
    (p.len(), at(1000), at(n / 2), at(n - 1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_sizes() {
        assert_eq!(color_table_size(1), 1);
        assert_eq!(color_table_size(2), 1);
        assert_eq!(color_table_size(3), 2);
        assert_eq!(color_table_size(128), 7);
        assert_eq!(color_table_size(129), 8);
        assert_eq!(color_table_size(256), 8);
    }

    #[test]
    fn single_colour() {
        let px = [10u8, 20, 30, 255].repeat(50);
        let pal = quantize(&px, 256);
        assert_eq!(pal, vec![[10, 20, 30]]);
        assert!(apply_palette(&px, &pal).iter().all(|i| *i == 0));
        let mut g = GifEncoder::new();
        g.write_frame(&apply_palette(&px, &pal), 10, 5, Some(&pal), 83.0);
        g.finish();
        let b = g.into_bytes();
        assert_eq!(&b[..6], b"GIF89a");
        assert_eq!(*b.last().unwrap(), 0x3b);
    }
}
