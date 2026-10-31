// SPDX-License-Identifier: Apache-2.0
//! A copy of the PDF without its text: `stripTextObjects` (byte-for-byte port) applied to every page content
//! stream and every form XObject reachable from the pages' resources. The TS rewrites the file with pdf-lib;
//! here the rewritten streams are appended as an incremental update (same object numbers, uncompressed), which
//! leaves every other object untouched.

use std::collections::HashSet;
use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Name, ObjectIdentifier, Stream};

const fn is_ws(c: u8) -> bool {
    matches!(c, 0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}
const fn is_delim(c: u8) -> bool {
    matches!(c, 0x28 | 0x29 | 0x3c | 0x3e | 0x5b | 0x5d | 0x7b | 0x7d | 0x2f | 0x25)
}

/// Remove `BT … ET` text objects from a content stream (strings, comments and inline images are skipped safely).
pub fn strip_text_objects(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0;
    let mut in_text = false;
    let n = src.len();
    macro_rules! emit {
        ($from:expr, $to:expr) => {
            if !in_text {
                out.extend_from_slice(&src[$from..($to).min(n)]);
            }
        };
    }
    while i < n {
        let c = src[i];
        if c == 0x25 {
            let s = i;
            while i < n && src[i] != 0x0a && src[i] != 0x0d {
                i += 1;
            }
            emit!(s, i);
            continue;
        }
        if c == 0x28 {
            let s = i;
            let mut depth = 0;
            while i < n {
                let ch = src[i];
                if ch == 0x5c {
                    i += 2;
                    continue;
                }
                if ch == 0x28 {
                    depth += 1;
                } else if ch == 0x29 {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            emit!(s, i);
            continue;
        }
        if c == 0x3c && src.get(i + 1) != Some(&0x3c) {
            let s = i;
            while i < n && src[i] != 0x3e {
                i += 1;
            }
            i += 1;
            emit!(s, i);
            continue;
        }
        if is_ws(c) || is_delim(c) {
            emit!(i, i + 1);
            i += 1;
            continue;
        }
        let s = i;
        while i < n && !is_ws(src[i]) && !is_delim(src[i]) {
            i += 1;
        }
        let tok = &src[s..i];
        if tok == b"BT" {
            in_text = true;
            continue;
        }
        if tok == b"ET" && in_text {
            in_text = false;
            continue;
        }
        if tok == b"ID" {
            // Inline image data: copy raw bytes up to the EI token.
            let mut j = i + 1;
            while j + 2 < n && !(is_ws(src[j - 1]) && src[j] == 0x45 && src[j + 1] == 0x49 && (j + 2 >= n || is_ws(src[j + 2]))) {
                j += 1;
            }
            emit!(s, j + 2);
            i = j + 2;
            continue;
        }
        emit!(s, i);
    }
    out
}

/// Top-level `key value` spans of a dictionary's source (`<< … >>`), as byte ranges.
fn dict_entries(data: &[u8]) -> Vec<(Vec<u8>, std::ops::Range<usize>)> {
    let mut i = 0;
    let n = data.len();
    let skip_ws = |i: &mut usize| {
        while *i < n && (is_ws(data[*i]) || data[*i] == b'%') {
            if data[*i] == b'%' {
                while *i < n && data[*i] != b'\n' && data[*i] != b'\r' {
                    *i += 1;
                }
            } else {
                *i += 1;
            }
        }
    };
    skip_ws(&mut i);
    if data[i..].starts_with(b"<<") {
        i += 2;
    }
    // Skip one value starting at `i`, returning its end.
    fn value_end(data: &[u8], mut i: usize) -> usize {
        let n = data.len();
        if i >= n {
            return n;
        }
        match data[i] {
            b'<' if data.get(i + 1) == Some(&b'<') => {
                let mut depth = 0;
                while i < n {
                    if data[i..].starts_with(b"<<") {
                        depth += 1;
                        i += 2;
                    } else if data[i..].starts_with(b">>") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            return i;
                        }
                    } else if data[i] == b'(' {
                        i = value_end(data, i);
                    } else {
                        i += 1;
                    }
                }
                n
            }
            b'<' => data[i..].iter().position(|&c| c == b'>').map_or(n, |p| i + p + 1),
            b'[' => {
                let mut depth = 0;
                while i < n {
                    match data[i] {
                        b'[' => {
                            depth += 1;
                            i += 1;
                        }
                        b']' => {
                            depth -= 1;
                            i += 1;
                            if depth == 0 {
                                return i;
                            }
                        }
                        b'(' | b'<' => i = value_end(data, i),
                        _ => i += 1,
                    }
                }
                n
            }
            b'(' => {
                let mut depth = 0;
                while i < n {
                    match data[i] {
                        b'\\' => i += 2,
                        b'(' => {
                            depth += 1;
                            i += 1;
                        }
                        b')' => {
                            depth -= 1;
                            i += 1;
                            if depth == 0 {
                                return i;
                            }
                        }
                        _ => i += 1,
                    }
                }
                n
            }
            b'/' => {
                i += 1;
                while i < n && !is_ws(data[i]) && !is_delim(data[i]) {
                    i += 1;
                }
                i
            }
            _ => {
                // number, boolean, null, or an `n g R` reference.
                let tok_end = |mut j: usize| {
                    while j < n && !is_ws(data[j]) && !is_delim(data[j]) {
                        j += 1;
                    }
                    j
                };
                let e1 = tok_end(i);
                let mut j = e1;
                while j < n && is_ws(data[j]) {
                    j += 1;
                }
                let e2 = tok_end(j);
                let mut k = e2;
                while k < n && is_ws(data[k]) {
                    k += 1;
                }
                let is_int = |a: usize, b: usize| b > a && data[a..b].iter().all(u8::is_ascii_digit);
                if is_int(i, e1) && is_int(j, e2) && data.get(k) == Some(&b'R') && (k + 1 >= n || is_ws(data[k + 1]) || is_delim(data[k + 1])) {
                    return k + 1;
                }
                e1
            }
        }
    }
    let mut out = Vec::new();
    loop {
        skip_ws(&mut i);
        if i >= n || data[i..].starts_with(b">>") || data[i] != b'/' {
            break;
        }
        let ks = i;
        let ke = value_end(data, i);
        let key = data[ks + 1..ke].to_vec();
        let mut v = ke;
        skip_ws(&mut v);
        let ve = value_end(data, v);
        out.push((key, ks..ve));
        i = ve;
    }
    out
}

/// The stripped copy of `bytes` (`stripPdfText`). `None` when nothing could be rewritten.
pub fn strip_pdf_text(bytes: &[u8]) -> Option<Vec<u8>> {
    let pdf = Pdf::new(bytes.to_vec()).ok()?;
    let mut seen: HashSet<ObjectIdentifier> = HashSet::new();
    let mut rewritten: Vec<(ObjectIdentifier, Vec<u8>)> = Vec::new();

    let mut rewrite = |s: &Stream<'_>, seen: &mut HashSet<ObjectIdentifier>| {
        let id = s.obj_id();
        if !seen.insert(id) {
            return;
        }
        let Ok(data) = s.decoded() else { return };
        let stripped = strip_text_objects(&data);
        let raw = s.dict().data();
        let mut dict = b"<<".to_vec();
        for (k, r) in dict_entries(raw) {
            if matches!(&k[..], b"Filter" | b"DecodeParms" | b"Length") {
                continue;
            }
            dict.push(b' ');
            dict.extend_from_slice(&raw[r]);
        }
        let _ = write!(Wrap(&mut dict), " /Length {} >>", stripped.len());
        let mut obj = Vec::new();
        let _ = writeln!(Wrap(&mut obj), "{} {} obj", id.obj_number, id.gen_number);
        obj.extend_from_slice(&dict);
        obj.extend_from_slice(b"\nstream\n");
        obj.extend_from_slice(&stripped);
        obj.extend_from_slice(b"\nendstream\nendobj\n");
        rewritten.push((id, obj));
    };

    fn visit_resources<'a>(res: &Dict<'a>, seen: &mut HashSet<ObjectIdentifier>, rewrite: &mut dyn FnMut(&Stream<'_>, &mut HashSet<ObjectIdentifier>)) {
        let Some(xo) = res.get::<Dict<'a>>(b"XObject") else { return };
        for (name, v) in xo.entries() {
            // Forms may share (and so contain) their parent's resources.
            let MaybeRef::Ref(r) = v else { continue };
            if seen.contains(&ObjectIdentifier::from(r)) {
                continue;
            }
            let Some(s) = xo.get::<Stream<'a>>(name.as_ref()) else { continue };
            if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_str() == "Form") {
                rewrite(&s, seen);
                if let Some(r2) = s.dict().get::<Dict<'a>>(b"Resources") {
                    visit_resources(&r2, seen, rewrite);
                }
            }
        }
    }

    for page in pdf.pages().iter() {
        let raw = page.raw();
        if let Some(s) = raw.get::<Stream<'_>>(b"Contents") {
            rewrite(&s, &mut seen);
        } else if let Some(a) = raw.get::<Array<'_>>(b"Contents") {
            for s in a.iter::<Stream<'_>>() {
                rewrite(&s, &mut seen);
            }
        }
        let res = page.resources();
        // Rebuild a resources dictionary view: only the XObject entries matter.
        let xo = &res.x_objects;
        for (name, v) in xo.entries() {
            let MaybeRef::Ref(r) = v else { continue };
            if seen.contains(&ObjectIdentifier::from(r)) {
                continue;
            }
            let Some(s) = xo.get::<Stream<'_>>(name.as_ref()) else { continue };
            if s.dict().get::<Name<'_>>(b"Subtype").is_some_and(|n| n.as_str() == "Form") {
                rewrite(&s, &mut seen);
                if let Some(r2) = s.dict().get::<Dict<'_>>(b"Resources") {
                    visit_resources(&r2, &mut seen, &mut rewrite);
                }
            }
        }
    }
    if rewritten.is_empty() {
        return None;
    }

    // Incremental update.
    let prev = find_startxref(bytes)?;
    let old_size = last_size(bytes).unwrap_or(0);
    let root = pdf.xref().root_id();
    let mut out = bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    rewritten.sort_by_key(|(id, _)| id.obj_number);
    let mut offsets = Vec::new();
    for (id, obj) in &rewritten {
        offsets.push((*id, out.len()));
        out.extend_from_slice(obj);
    }
    let xref_at = out.len();
    let mut x = String::from("xref\n");
    for (id, off) in &offsets {
        let _ = write!(x, "{} 1\n{:010} {:05} n \n", id.obj_number, off, id.gen_number);
    }
    let max = rewritten.iter().map(|(id, _)| id.obj_number as usize + 1).max().unwrap_or(0);
    let size = old_size.max(max).max(pdf.len() + 1);
    let _ = write!(x, "trailer\n<< /Size {size} /Root {} {} R /Prev {prev} >>\nstartxref\n{xref_at}\n%%EOF\n", root.obj_number, root.gen_number);
    out.extend_from_slice(x.as_bytes());
    Some(out)
}

struct Wrap<'a>(&'a mut Vec<u8>);
impl std::fmt::Write for Wrap<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn find_startxref(b: &[u8]) -> Option<usize> {
    let pos = b.windows(9).rposition(|w| w == b"startxref")?;
    let rest = &b[pos + 9..];
    let s: String = rest.iter().skip_while(|c| c.is_ascii_whitespace()).take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    s.parse().ok()
}

fn last_size(b: &[u8]) -> Option<usize> {
    let pos = b.windows(5).rposition(|w| w == b"/Size")?;
    let s: String = b[pos + 5..].iter().skip_while(|c| c.is_ascii_whitespace()).take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_text_objects_only() {
        let src = b"q 1 0 0 1 0 0 cm BT /F1 12 Tf (a BT ) ET) Tj ET 0 0 m % BT comment\n10 10 l S BI /W 1 ID \x00ET\x01 EI Q";
        let out = strip_text_objects(src);
        // What the TS `stripTextObjects` answers for the same bytes (the literal string ends at its first `)`).
        assert_eq!(String::from_utf8_lossy(&out), "q 1 0 0 1 0 0 cm ) Tj ET 0 0 m % BT comment\n10 10 l S BI /W 1 ID \u{0}ET\u{1} EI Q");
    }

    #[test]
    fn dict_entries_split() {
        let d = b"<< /Type /XObject /Subtype /Form /BBox [0 0 200 100] /Resources << /Font << /F1 5 0 R >> >> /Filter /FlateDecode /Length 12 0 R >>";
        let keys: Vec<String> = dict_entries(d).into_iter().map(|(k, r)| format!("{}={}", String::from_utf8_lossy(&k), String::from_utf8_lossy(&d[r]))).collect();
        assert_eq!(
            keys,
            ["Type=/Type /XObject", "Subtype=/Subtype /Form", "BBox=/BBox [0 0 200 100]", "Resources=/Resources << /Font << /F1 5 0 R >> >>", "Filter=/Filter /FlateDecode", "Length=/Length 12 0 R"]
        );
    }
}
