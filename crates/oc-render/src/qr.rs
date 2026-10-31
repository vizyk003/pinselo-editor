// SPDX-License-Identifier: Apache-2.0
//! QR code encoder: a port of the `qrcode` npm package (1.5.4, MIT, © Kazuhiko Arase / Ryan Day) as the TS
//! resolver calls it — `QR.create(data, { errorCorrectionLevel })` with automatic version, optimal segmentation
//! (numeric / alphanumeric / byte, Kanji mode off) chosen by the package's Dijkstra search, and the
//! lowest-penalty mask. Every tie is broken the way the JS breaks it (object key order, stable sorts), so the
//! modules are identical to what the TS produced.

use std::collections::HashMap;

/// A finished symbol: `size × size` modules, row-major, 1 = dark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrCode {
    pub size: usize,
    pub modules: Vec<u8>,
    pub version: usize,
    pub mask: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QrError {
    #[error("No input text")]
    Empty,
    #[error("The amount of data is too big to be stored in a QR Code")]
    TooBig,
}

/// Error correction level (`bit` as in the spec's format information).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecc {
    L,
    M,
    Q,
    H,
}

impl Ecc {
    fn bit(self) -> u32 {
        match self {
            Ecc::L => 1,
            Ecc::M => 0,
            Ecc::Q => 3,
            Ecc::H => 2,
        }
    }

    fn index(self) -> usize {
        match self {
            Ecc::L => 0,
            Ecc::M => 1,
            Ecc::Q => 2,
            Ecc::H => 3,
        }
    }

    /// `ECLevel.from(value, M)`: `l`/`low`, `m`/`medium`, `q`/`quartile`, `h`/`high` (any case), else M.
    pub fn parse(s: &str) -> Ecc {
        match s.to_lowercase().as_str() {
            "l" | "low" => Ecc::L,
            "q" | "quartile" => Ecc::Q,
            "h" | "high" => Ecc::H,
            _ => Ecc::M,
        }
    }
}

const CODEWORDS_COUNT: [usize; 41] = [
    0, 26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991, 1085, 1156, 1258, 1364, 1474, 1588,
    1706, 1828, 1921, 2051, 2185, 2323, 2465, 2611, 2761, 2876, 3034, 3196, 3362, 3532, 3706,
];

/// Error correction blocks per version (L, M, Q, H), `error-correction-code.js`.
const EC_BLOCKS_TABLE: [usize; 160] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 1, 2, 2, 4, 1, 2, 4, 4, 2, 4, 4, 4, 2, 4, 6, 5, 2, 4, 6, 6, 2, 5, 8, 8, 4, 5, 8, 8, 4, 5, 8, 11, 4,
    8, 10, 11, 4, 9, 12, 16, 4, 9, 16, 16, 6, 10, 12, 18, 6, 10, 17, 16, 6, 11, 16, 19, 6, 13, 18, 21, 7, 14, 21, 25, 8, 16, 20, 25, 8,
    17, 23, 25, 9, 17, 23, 34, 9, 18, 25, 30, 10, 20, 27, 32, 12, 21, 29, 35, 12, 23, 34, 37, 12, 25, 34, 40, 13, 26, 35, 42, 14, 28,
    38, 45, 15, 29, 40, 48, 16, 31, 43, 51, 17, 33, 45, 54, 18, 35, 48, 57, 19, 37, 51, 60, 19, 38, 53, 63, 20, 40, 56, 66, 21, 43, 59,
    70, 22, 45, 62, 74, 24, 47, 65, 77, 25, 49, 68, 81,
];

/// Error correction codewords per version (L, M, Q, H).
const EC_CODEWORDS_TABLE: [usize; 160] = [
    7, 10, 13, 17, 10, 16, 22, 28, 15, 26, 36, 44, 20, 36, 52, 64, 26, 48, 72, 88, 36, 64, 96, 112, 40, 72, 108, 130, 48, 88, 132, 156,
    60, 110, 160, 192, 72, 130, 192, 224, 80, 150, 224, 264, 96, 176, 260, 308, 104, 198, 288, 352, 120, 216, 320, 384, 132, 240, 360,
    432, 144, 280, 408, 480, 168, 308, 448, 532, 180, 338, 504, 588, 196, 364, 546, 650, 224, 416, 600, 700, 224, 442, 644, 750, 252,
    476, 690, 816, 270, 504, 750, 900, 300, 560, 810, 960, 312, 588, 870, 1050, 336, 644, 952, 1110, 360, 700, 1020, 1200, 390, 728,
    1050, 1260, 420, 784, 1140, 1350, 450, 812, 1200, 1440, 480, 868, 1290, 1530, 510, 924, 1350, 1620, 540, 980, 1440, 1710, 570,
    1036, 1530, 1800, 570, 1064, 1590, 1890, 600, 1120, 1680, 1980, 630, 1204, 1770, 2100, 660, 1260, 1860, 2220, 720, 1316, 1950, 2310,
    750, 1372, 2040, 2430,
];

fn blocks_count(version: usize, ecc: Ecc) -> usize {
    EC_BLOCKS_TABLE[(version - 1) * 4 + ecc.index()]
}

fn total_ec_codewords(version: usize, ecc: Ecc) -> usize {
    EC_CODEWORDS_TABLE[(version - 1) * 4 + ecc.index()]
}

fn symbol_size(version: usize) -> usize {
    version * 4 + 17
}

// ------------------------------------------------------------------ modes and segments

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

impl Mode {
    fn bit(self) -> u32 {
        match self {
            Mode::Numeric => 1,
            Mode::Alphanumeric => 2,
            Mode::Byte => 4,
        }
    }

    fn cc_bits(self, version: usize) -> usize {
        let t = match self {
            Mode::Numeric => [10, 12, 14],
            Mode::Alphanumeric => [9, 11, 13],
            Mode::Byte => [8, 16, 16],
        };
        if version < 10 {
            t[0]
        } else if version < 27 {
            t[1]
        } else {
            t[2]
        }
    }

    fn bits_length(self, length: usize) -> usize {
        match self {
            Mode::Numeric => 10 * (length / 3) + if !length.is_multiple_of(3) { (length % 3) * 3 + 1 } else { 0 },
            Mode::Alphanumeric => 11 * (length / 2) + 6 * (length % 2),
            Mode::Byte => length * 8,
        }
    }
}

const ALPHA_NUM_CHARS: &[u16] = &[
    b'0' as u16, b'1' as u16, b'2' as u16, b'3' as u16, b'4' as u16, b'5' as u16, b'6' as u16, b'7' as u16, b'8' as u16, b'9' as u16,
    b'A' as u16, b'B' as u16, b'C' as u16, b'D' as u16, b'E' as u16, b'F' as u16, b'G' as u16, b'H' as u16, b'I' as u16, b'J' as u16,
    b'K' as u16, b'L' as u16, b'M' as u16, b'N' as u16, b'O' as u16, b'P' as u16, b'Q' as u16, b'R' as u16, b'S' as u16, b'T' as u16,
    b'U' as u16, b'V' as u16, b'W' as u16, b'X' as u16, b'Y' as u16, b'Z' as u16, b' ' as u16, b'$' as u16, b'%' as u16, b'*' as u16,
    b'+' as u16, b'-' as u16, b'.' as u16, b'/' as u16, b':' as u16,
];

fn is_digit(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

/// `[A-Z $%*+\-./:]` (no digits).
fn is_alpha(c: u16) -> bool {
    (0x41..=0x5a).contains(&c) || matches!(c, 0x20 | 0x24 | 0x25 | 0x2a | 0x2b | 0x2d | 0x2e | 0x2f | 0x3a)
}

/// A segment of the input: UTF-16 code units, with the length the graph uses (bytes for byte mode).
#[derive(Debug, Clone)]
struct Seg {
    data: Vec<u16>,
    mode: Mode,
    length: usize,
}

fn utf8_len(units: &[u16]) -> usize {
    // `unescape(encodeURIComponent(str)).length`: UTF-8 byte count (lone surrogates cannot occur in Rust input).
    String::from_utf16_lossy(units).len()
}

/// Segments from the three regexes (`[0-9]+`, `[A-Z $%*+\-./:]+`, `[^A-Z0-9 $%*+\-./:]+`), sorted by index.
fn raw_segments(data: &[u16]) -> Vec<Seg> {
    let class = |c: u16| if is_digit(c) { Mode::Numeric } else if is_alpha(c) { Mode::Alphanumeric } else { Mode::Byte };
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let m = class(data[i]);
        let mut j = i + 1;
        while j < data.len() && class(data[j]) == m {
            j += 1;
        }
        out.push(Seg { data: data[i..j].to_vec(), mode: m, length: j - i });
        i = j;
    }
    out
}

fn reserved_bits(mode: Mode, version: usize) -> usize {
    mode.cc_bits(version) + 4
}

/// `Version.getCapacity` for one mode (`None` = mixed: the data bits).
fn capacity(version: usize, ecc: Ecc, mode: Option<Mode>) -> usize {
    let data_bits = (CODEWORDS_COUNT[version] - total_ec_codewords(version, ecc)) * 8;
    let Some(mode) = mode else { return data_bits };
    let usable = data_bits as f64 - reserved_bits(mode, version) as f64;
    let v = match mode {
        Mode::Numeric => ((usable / 10.0) * 3.0).floor(),
        Mode::Alphanumeric => ((usable / 11.0) * 2.0).floor(),
        Mode::Byte => (usable / 8.0).floor(),
    };
    v.max(0.0) as usize
}

/// A segment as the encoder writes it (`NumericData` / `AlphanumericData` / `ByteData`).
#[derive(Debug, Clone)]
struct Data {
    mode: Mode,
    units: Vec<u16>,
    bytes: Vec<u8>,
}

impl Data {
    fn new(mode: Mode, units: Vec<u16>) -> Data {
        let bytes = if mode == Mode::Byte { String::from_utf16_lossy(&units).into_bytes() } else { Vec::new() };
        Data { mode, units, bytes }
    }

    fn length(&self) -> usize {
        match self.mode {
            Mode::Byte => self.bytes.len(),
            _ => self.units.len(),
        }
    }

    fn bits_length(&self) -> usize {
        self.mode.bits_length(self.length())
    }

    fn write(&self, buf: &mut BitBuffer) {
        match self.mode {
            Mode::Numeric => {
                let digits: Vec<u32> = self.units.iter().map(|c| u32::from(*c - 0x30)).collect();
                let mut i = 0;
                while i + 3 <= digits.len() {
                    buf.put(digits[i] * 100 + digits[i + 1] * 10 + digits[i + 2], 10);
                    i += 3;
                }
                let rest = digits.len() - i;
                if rest > 0 {
                    let v = digits[i..].iter().fold(0, |a, d| a * 10 + d);
                    buf.put(v, rest * 3 + 1);
                }
            }
            Mode::Alphanumeric => {
                let idx = |c: u16| ALPHA_NUM_CHARS.iter().position(|x| *x == c).map_or(u32::MAX, |p| p as u32);
                let mut i = 0;
                while i + 2 <= self.units.len() {
                    let v = idx(self.units[i]).wrapping_mul(45).wrapping_add(idx(self.units[i + 1]));
                    buf.put(v, 11);
                    i += 2;
                }
                if self.units.len() % 2 == 1 {
                    buf.put(idx(self.units[i]), 6);
                }
            }
            Mode::Byte => {
                for b in &self.bytes {
                    buf.put(u32::from(*b), 8);
                }
            }
        }
    }
}

/// `getBestVersionForData`.
fn best_version(segs: &[Data], ecc: Ecc) -> Option<usize> {
    if segs.len() > 1 {
        return (1..=40).find(|&v| {
            let bits: usize = segs.iter().map(|d| reserved_bits(d.mode, v) + d.bits_length()).sum();
            bits <= capacity(v, ecc, None)
        });
    }
    let Some(seg) = segs.first() else { return Some(1) };
    (1..=40).find(|&v| seg.length() <= capacity(v, ecc, Some(seg.mode)))
}

/// Graph node key `'' + i + j` with JS property order: canonical array-index keys come first (ascending), the
/// rest in insertion order.
fn js_key_order(keys: &[String]) -> Vec<String> {
    let is_index = |k: &str| k == "0" || (!k.is_empty() && !k.starts_with('0') && k.bytes().all(|b| b.is_ascii_digit()) && k.len() < 10);
    let mut idx: Vec<&String> = keys.iter().filter(|k| is_index(k)).collect();
    idx.sort_by_key(|k| k.parse::<u64>().unwrap_or(0));
    let rest = keys.iter().filter(|k| !is_index(k));
    idx.into_iter().chain(rest).cloned().collect()
}

/// `Segments.fromString`: the cheapest mode per run by the package's Dijkstra search.
fn optimal_segments(data: &[u16], version: usize) -> Vec<Data> {
    let segs = raw_segments(data);
    // buildNodes
    let mut nodes: Vec<Vec<Seg>> = Vec::new();
    for s in &segs {
        match s.mode {
            Mode::Numeric => nodes.push(vec![
                s.clone(),
                Seg { data: s.data.clone(), mode: Mode::Alphanumeric, length: s.length },
                Seg { data: s.data.clone(), mode: Mode::Byte, length: s.length },
            ]),
            Mode::Alphanumeric => nodes.push(vec![s.clone(), Seg { data: s.data.clone(), mode: Mode::Byte, length: s.length }]),
            Mode::Byte => nodes.push(vec![Seg { data: s.data.clone(), mode: Mode::Byte, length: utf8_len(&s.data) }]),
        }
    }
    // buildGraph: adjacency lists in insertion order, `table` with the mutable lastCount.
    let mut graph: HashMap<String, Vec<(String, usize)>> = HashMap::new();
    let mut table: HashMap<String, (Seg, usize)> = HashMap::new();
    graph.insert("start".into(), Vec::new());
    let mut prev_ids: Vec<String> = vec!["start".into()];
    for (i, group) in nodes.iter().enumerate() {
        let mut current: Vec<String> = Vec::new();
        for (j, node) in group.iter().enumerate() {
            let key = format!("{i}{j}");
            current.push(key.clone());
            table.insert(key.clone(), (node.clone(), 0));
            graph.insert(key.clone(), Vec::new());
            for prev in &prev_ids {
                let cost;
                if let Some((pn, last)) = table.get_mut(prev).filter(|(pn, _)| pn.mode == node.mode) {
                    cost = pn.mode.bits_length(*last + node.length) - pn.mode.bits_length(*last);
                    *last += node.length;
                } else {
                    if let Some((_, last)) = table.get_mut(prev) {
                        *last = node.length;
                    }
                    cost = node.mode.bits_length(node.length) + 4 + node.mode.cc_bits(version);
                }
                let adj = graph.get_mut(prev).expect("node");
                match adj.iter_mut().find(|(k, _)| *k == key) {
                    Some(e) => e.1 = cost,
                    None => adj.push((key.clone(), cost)),
                }
            }
        }
        prev_ids = current;
    }
    for prev in &prev_ids {
        graph.get_mut(prev).expect("node").push(("end".into(), 0));
    }
    // dijkstrajs: a queue kept sorted (stable) by cost, popped from the front.
    let mut costs: HashMap<String, usize> = HashMap::from([("start".to_string(), 0)]);
    let mut preds: HashMap<String, String> = HashMap::new();
    let mut open: Vec<(String, usize)> = vec![("start".into(), 0)];
    while !open.is_empty() {
        let (u, cu) = open.remove(0);
        let adj = graph.get(&u).cloned().unwrap_or_default();
        let keys: Vec<String> = adj.iter().map(|(k, _)| k.clone()).collect();
        for v in js_key_order(&keys) {
            let ce = adj.iter().find(|(k, _)| *k == v).map_or(0, |e| e.1);
            let total = cu + ce;
            let better = costs.get(&v).is_none_or(|c| *c > total);
            if better {
                costs.insert(v.clone(), total);
                open.push((v.clone(), total));
                open.sort_by_key(|e| e.1);
                preds.insert(v, u.clone());
            }
        }
    }
    let mut path = Vec::new();
    let mut cur = Some("end".to_string());
    while let Some(c) = cur {
        cur = preds.get(&c).cloned();
        path.push(c);
    }
    path.reverse();
    // Path nodes between start and end, merged by mode.
    let mut merged: Vec<Seg> = Vec::new();
    let inner: &[String] = if path.len() >= 2 { &path[1..path.len() - 1] } else { &[] };
    for id in inner {
        let node = table[id].0.clone();
        match merged.last_mut() {
            Some(last) if last.mode == node.mode => last.data.extend_from_slice(&node.data),
            _ => merged.push(node),
        }
    }
    merged.into_iter().map(|s| Data::new(s.mode, s.data)).collect()
}

// ------------------------------------------------------------------ bits

#[derive(Default)]
struct BitBuffer {
    buffer: Vec<u8>,
    length: usize,
}

impl BitBuffer {
    fn put(&mut self, num: u32, length: usize) {
        for i in 0..length {
            let shift = (length - i - 1) as u32;
            let bit = if shift >= 32 { 0 } else { (num >> shift) & 1 };
            self.put_bit(bit == 1);
        }
    }

    fn put_bit(&mut self, bit: bool) {
        let idx = self.length / 8;
        if self.buffer.len() <= idx {
            self.buffer.push(0);
        }
        if bit {
            self.buffer[idx] |= 0x80 >> (self.length % 8);
        }
        self.length += 1;
    }
}

struct Matrix {
    size: usize,
    data: Vec<u8>,
    reserved: Vec<bool>,
}

impl Matrix {
    fn set(&mut self, r: usize, c: usize, v: bool, reserved: bool) {
        let i = r * self.size + c;
        self.data[i] = u8::from(v);
        if reserved {
            self.reserved[i] = true;
        }
    }
    fn get(&self, r: usize, c: usize) -> u8 {
        self.data[r * self.size + c]
    }
}

// ------------------------------------------------------------------ Reed-Solomon

struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

impl Gf {
    fn new() -> Gf {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u32 = 1;
        for (i, e) in exp.iter_mut().enumerate().take(255) {
            *e = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11d;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Gf { exp, log }
    }
    fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 { 0 } else { self.exp[self.log[a as usize] as usize + self.log[b as usize] as usize] }
    }
}

fn poly_mul(gf: &Gf, p1: &[u8], p2: &[u8]) -> Vec<u8> {
    let mut c = vec![0u8; p1.len() + p2.len() - 1];
    for (i, a) in p1.iter().enumerate() {
        for (j, b) in p2.iter().enumerate() {
            c[i + j] ^= gf.mul(*a, *b);
        }
    }
    c
}

fn poly_mod(gf: &Gf, dividend: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = dividend.to_vec();
    while result.len() >= divisor.len() {
        let coeff = result[0];
        for (i, d) in divisor.iter().enumerate() {
            result[i] ^= gf.mul(*d, coeff);
        }
        let offset = result.iter().position(|x| *x != 0).unwrap_or(result.len());
        result.drain(..offset);
    }
    result
}

fn rs_encode(gf: &Gf, genp: &[u8], degree: usize, data: &[u8]) -> Vec<u8> {
    let mut padded = data.to_vec();
    padded.resize(data.len() + degree, 0);
    let rem = poly_mod(gf, &padded, genp);
    if rem.len() < degree {
        let mut out = vec![0u8; degree];
        out[degree - rem.len()..].copy_from_slice(&rem);
        return out;
    }
    rem
}

fn create_codewords(buf: &BitBuffer, version: usize, ecc: Ecc) -> Vec<u8> {
    let gf = Gf::new();
    let total = CODEWORDS_COUNT[version];
    let ec_total = total_ec_codewords(version, ecc);
    let data_total = total - ec_total;
    let blocks = blocks_count(version, ecc);
    let in_group2 = total % blocks;
    let in_group1 = blocks - in_group2;
    let total_in_group1 = total / blocks;
    let data_in_group1 = data_total / blocks;
    let data_in_group2 = data_in_group1 + 1;
    let ec_count = total_in_group1 - data_in_group1;
    let mut genp = vec![1u8];
    for i in 0..ec_count {
        genp = poly_mul(&gf, &genp, &[1, gf.exp[i]]);
    }
    let mut offset = 0;
    let mut dc: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut ec: Vec<Vec<u8>> = Vec::with_capacity(blocks);
    let mut max_data = 0;
    for b in 0..blocks {
        let size = if b < in_group1 { data_in_group1 } else { data_in_group2 };
        let end = (offset + size).min(buf.buffer.len());
        let block: Vec<u8> = buf.buffer[offset.min(end)..end].to_vec();
        ec.push(rs_encode(&gf, &genp, ec_count, &block));
        dc.push(block);
        offset += size;
        max_data = max_data.max(size);
    }
    let mut data = vec![0u8; total];
    let mut index = 0;
    for i in 0..max_data {
        for block in &dc {
            if i < block.len() {
                data[index] = block[i];
                index += 1;
            }
        }
    }
    for i in 0..ec_count {
        for block in &ec {
            data[index] = block.get(i).copied().unwrap_or(0);
            index += 1;
        }
    }
    data
}

fn create_data(version: usize, ecc: Ecc, segs: &[Data]) -> Vec<u8> {
    let mut buf = BitBuffer::default();
    for d in segs {
        buf.put(d.mode.bit(), 4);
        buf.put(d.length() as u32, d.mode.cc_bits(version));
        d.write(&mut buf);
    }
    let data_bits = (CODEWORDS_COUNT[version] - total_ec_codewords(version, ecc)) * 8;
    if buf.length + 4 <= data_bits {
        buf.put(0, 4);
    }
    while !buf.length.is_multiple_of(8) {
        buf.put_bit(false);
    }
    let remaining = (data_bits as f64 - buf.length as f64) / 8.0;
    let mut i = 0;
    while (i as f64) < remaining {
        buf.put(if i % 2 == 1 { 0x11 } else { 0xec }, 8);
        i += 1;
    }
    create_codewords(&buf, version, ecc)
}

// ------------------------------------------------------------------ function patterns

fn setup_finder(m: &mut Matrix) {
    let size = m.size as i64;
    for (row, col) in [(0i64, 0i64), (size - 7, 0), (0, size - 7)] {
        for r in -1..=7i64 {
            if row + r <= -1 || size <= row + r {
                continue;
            }
            for c in -1..=7i64 {
                if col + c <= -1 || size <= col + c {
                    continue;
                }
                let dark = ((0..=6).contains(&r) && (c == 0 || c == 6)) || ((0..=6).contains(&c) && (r == 0 || r == 6)) || ((2..=4).contains(&r) && (2..=4).contains(&c));
                m.set((row + r) as usize, (col + c) as usize, dark, true);
            }
        }
    }
}

fn setup_timing(m: &mut Matrix) {
    let size = m.size;
    for r in 8..size.saturating_sub(8) {
        let v = r % 2 == 0;
        m.set(r, 6, v, true);
        m.set(6, r, v, true);
    }
}

fn alignment_coords(version: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let pos_count = version / 7 + 2;
    let size = symbol_size(version);
    let intervals = if size == 145 { 26 } else { ((size - 13) as f64 / (2 * pos_count - 2) as f64).ceil() as usize * 2 };
    let mut positions = vec![size - 7];
    for i in 1..pos_count - 1 {
        positions.push(positions[i - 1] - intervals);
    }
    positions.push(6);
    positions.reverse();
    positions
}

fn setup_alignment(m: &mut Matrix, version: usize) {
    let pos = alignment_coords(version);
    let n = pos.len();
    for i in 0..n {
        for j in 0..n {
            if (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0) {
                continue;
            }
            let (row, col) = (pos[i] as i64, pos[j] as i64);
            for r in -2..=2i64 {
                for c in -2..=2i64 {
                    let dark = r == -2 || r == 2 || c == -2 || c == 2 || (r == 0 && c == 0);
                    m.set((row + r) as usize, (col + c) as usize, dark, true);
                }
            }
        }
    }
}

fn bch_digit(mut d: u32) -> i32 {
    let mut digit = 0;
    while d != 0 {
        digit += 1;
        d >>= 1;
    }
    digit
}

fn version_bits(version: usize) -> u32 {
    const G18: u32 = (1 << 12) | (1 << 11) | (1 << 10) | (1 << 9) | (1 << 8) | (1 << 5) | (1 << 2) | 1;
    let g18_bch = bch_digit(G18);
    let mut d = (version as u32) << 12;
    while bch_digit(d) - g18_bch >= 0 {
        d ^= G18 << (bch_digit(d) - g18_bch);
    }
    ((version as u32) << 12) | d
}

fn setup_version_info(m: &mut Matrix, version: usize) {
    let size = m.size;
    let bits = version_bits(version);
    for i in 0..18 {
        let row = i / 3;
        let col = i % 3 + size - 8 - 3;
        let v = (bits >> i) & 1 == 1;
        m.set(row, col, v, true);
        m.set(col, row, v, true);
    }
}

fn format_bits(ecc: Ecc, mask: u8) -> u32 {
    const G15: u32 = (1 << 10) | (1 << 8) | (1 << 5) | (1 << 4) | (1 << 2) | (1 << 1) | 1;
    const G15_MASK: u32 = (1 << 14) | (1 << 12) | (1 << 10) | (1 << 4) | (1 << 1);
    let g15_bch = bch_digit(G15);
    let data = (ecc.bit() << 3) | u32::from(mask);
    let mut d = data << 10;
    while bch_digit(d) - g15_bch >= 0 {
        d ^= G15 << (bch_digit(d) - g15_bch);
    }
    ((data << 10) | d) ^ G15_MASK
}

fn setup_format_info(m: &mut Matrix, ecc: Ecc, mask: u8) {
    let size = m.size;
    let bits = format_bits(ecc, mask);
    for i in 0..15 {
        let v = (bits >> i) & 1 == 1;
        if i < 6 {
            m.set(i, 8, v, true);
        } else if i < 8 {
            m.set(i + 1, 8, v, true);
        } else {
            m.set(size - 15 + i, 8, v, true);
        }
        if i < 8 {
            m.set(8, size - i - 1, v, true);
        } else if i < 9 {
            m.set(8, 15 - i - 1 + 1, v, true);
        } else {
            m.set(8, 15 - i - 1, v, true);
        }
    }
    m.set(size - 8, 8, true, true);
}

fn setup_data(m: &mut Matrix, data: &[u8]) {
    let size = m.size as i64;
    let mut inc: i64 = -1;
    let mut row: i64 = size - 1;
    let mut bit_index: i32 = 7;
    let mut byte_index = 0usize;
    let mut col = size - 1;
    while col > 0 {
        if col == 6 {
            col -= 1;
        }
        loop {
            for c in 0..2 {
                let (r, cc) = (row as usize, (col - c) as usize);
                if !m.reserved[r * m.size + cc] {
                    let dark = byte_index < data.len() && (data[byte_index] >> bit_index) & 1 == 1;
                    m.set(r, cc, dark, false);
                    bit_index -= 1;
                    if bit_index == -1 {
                        byte_index += 1;
                        bit_index = 7;
                    }
                }
            }
            row += inc;
            if row < 0 || size <= row {
                row -= inc;
                inc = -inc;
                break;
            }
        }
        col -= 2;
    }
}

fn mask_at(p: u8, i: usize, j: usize) -> bool {
    match p {
        0 => (i + j).is_multiple_of(2),
        1 => i.is_multiple_of(2),
        2 => j.is_multiple_of(3),
        3 => (i + j).is_multiple_of(3),
        4 => (i / 2 + j / 3).is_multiple_of(2),
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3).is_multiple_of(2),
        _ => ((i * j) % 3 + (i + j) % 2).is_multiple_of(2),
    }
}

fn apply_mask(p: u8, m: &mut Matrix) {
    let size = m.size;
    for col in 0..size {
        for row in 0..size {
            let i = row * size + col;
            if m.reserved[i] {
                continue;
            }
            m.data[i] ^= u8::from(mask_at(p, row, col));
        }
    }
}

fn penalty(m: &Matrix) -> u64 {
    let size = m.size;
    let mut points: u64 = 0;
    // N1
    for row in 0..size {
        let (mut same_col, mut same_row) = (0u64, 0u64);
        let (mut last_col, mut last_row): (Option<u8>, Option<u8>) = (None, None);
        for col in 0..size {
            let v = m.get(row, col);
            if Some(v) == last_col {
                same_col += 1;
            } else {
                if same_col >= 5 {
                    points += 3 + (same_col - 5);
                }
                last_col = Some(v);
                same_col = 1;
            }
            let v = m.get(col, row);
            if Some(v) == last_row {
                same_row += 1;
            } else {
                if same_row >= 5 {
                    points += 3 + (same_row - 5);
                }
                last_row = Some(v);
                same_row = 1;
            }
        }
        if same_col >= 5 {
            points += 3 + (same_col - 5);
        }
        if same_row >= 5 {
            points += 3 + (same_row - 5);
        }
    }
    // N2
    let mut n2 = 0u64;
    for row in 0..size - 1 {
        for col in 0..size - 1 {
            let s = m.get(row, col) + m.get(row, col + 1) + m.get(row + 1, col) + m.get(row + 1, col + 1);
            if s == 4 || s == 0 {
                n2 += 1;
            }
        }
    }
    points += n2 * 3;
    // N3
    let mut n3 = 0u64;
    for row in 0..size {
        let (mut bc, mut br) = (0u32, 0u32);
        for col in 0..size {
            bc = ((bc << 1) & 0x7ff) | u32::from(m.get(row, col));
            if col >= 10 && (bc == 0x5d0 || bc == 0x05d) {
                n3 += 1;
            }
            br = ((br << 1) & 0x7ff) | u32::from(m.get(col, row));
            if col >= 10 && (br == 0x5d0 || br == 0x05d) {
                n3 += 1;
            }
        }
    }
    points += n3 * 40;
    // N4
    let dark: f64 = m.data.iter().map(|v| f64::from(*v)).sum();
    let count = m.data.len() as f64;
    let k = (((dark * 100.0 / count) / 5.0).ceil() - 10.0).abs();
    points + (k as u64) * 10
}

/// `QR.create(data, { errorCorrectionLevel })`.
pub fn create(data: &str, ecc: Ecc) -> Result<QrCode, QrError> {
    if data.is_empty() {
        return Err(QrError::Empty);
    }
    let units: Vec<u16> = data.encode_utf16().collect();
    let raw: Vec<Data> = raw_segments(&units).into_iter().map(|s| Data::new(s.mode, s.data)).collect();
    let estimated = best_version(&raw, ecc).unwrap_or(40);
    let segs = optimal_segments(&units, estimated);
    let version = best_version(&segs, ecc).ok_or(QrError::TooBig)?;
    let bits = create_data(version, ecc, &segs);
    let size = symbol_size(version);
    let mut m = Matrix { size, data: vec![0; size * size], reserved: vec![false; size * size] };
    setup_finder(&mut m);
    setup_timing(&mut m);
    setup_alignment(&mut m, version);
    setup_format_info(&mut m, ecc, 0);
    if version >= 7 {
        setup_version_info(&mut m, version);
    }
    setup_data(&mut m, &bits);
    let mut best = 0u8;
    let mut lowest = u64::MAX;
    for p in 0..8u8 {
        setup_format_info(&mut m, ecc, p);
        apply_mask(p, &mut m);
        let pen = penalty(&m);
        apply_mask(p, &mut m);
        if pen < lowest {
            lowest = pen;
            best = p;
        }
    }
    apply_mask(best, &mut m);
    setup_format_info(&mut m, ecc, best);
    Ok(QrCode { size, modules: m.data, version, mask: best })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_version_and_size() {
        let q = create("https://example.com", Ecc::M).unwrap();
        assert_eq!((q.version, q.size), (2, 25));
        assert_eq!(create("", Ecc::M), Err(QrError::Empty));
        assert_eq!(create(&"9".repeat(8000), Ecc::H), Err(QrError::TooBig));
    }

    #[test]
    fn key_order_like_js() {
        let keys: Vec<String> = ["00", "01", "10", "11", "end"].iter().map(|s| s.to_string()).collect();
        assert_eq!(js_key_order(&keys), ["10", "11", "00", "01", "end"]);
    }
}
