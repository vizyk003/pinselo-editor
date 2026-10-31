//! Pixel dimensions of a rendered file: the PNG and JPEG detectors of `image-size` 2.0.4 (the publish job's
//! `imageSize(bytes)`). Every case where `image-size` throws (unsupported type, corrupt data, reads past
//! the end) is `None`; the caller then keeps the design's dimensions, as the TS `try { … } catch {}` does.
//! The render service only produces PNG, JPEG and MP4, so the other `image-size` formats are not ported.

/// `{ width, height }` of a PNG or JPEG, or `None`.
pub fn image_size(input: &[u8]) -> Option<(u32, u32)> {
    match input.first() {
        Some(0x89) if png_validate(input)? => png_size(input),
        Some(0xff) if input.get(1) == Some(&0xd8) => jpg_size(input),
        _ => None,
    }
}

fn u16be(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u32::from(u16::from_be_bytes([s[0], s[1]])))
}

fn u16le(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u32::from(u16::from_le_bytes([s[0], s[1]])))
}

fn u32be(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `input.slice(start, end)` (clamped like `Uint8Array#slice`).
fn slice(b: &[u8], start: usize, end: usize) -> &[u8] {
    let end = end.min(b.len());
    if start >= end { &[] } else { &b[start..end] }
}

/// `PNG.validate`: `Some(false)` = not a PNG, `None` = "Invalid PNG" (signature without IHDR).
fn png_validate(input: &[u8]) -> Option<bool> {
    if slice(input, 1, 8) != b"PNG\r\n\x1a\n" {
        return Some(false);
    }
    let mut chunk = slice(input, 12, 16);
    if chunk == b"CgBI" {
        chunk = slice(input, 28, 32);
    }
    if chunk != b"IHDR" {
        return None;
    }
    Some(true)
}

fn png_size(input: &[u8]) -> Option<(u32, u32)> {
    if slice(input, 12, 16) == b"CgBI" {
        return Some((u32be(input, 32)?, u32be(input, 36)?));
    }
    Some((u32be(input, 16)?, u32be(input, 20)?))
}

/// `readUInt(block, bits, offset, isBigEndian)`.
fn read_uint(b: &[u8], bits: u32, at: usize, big: bool) -> Option<u32> {
    match (bits, big) {
        (16, true) => u16be(b, at),
        (16, false) => u16le(b, at),
        (_, true) => u32be(b, at),
        (_, false) => u32le(b, at),
    }
}

/// EXIF orientation lookup. Only its failures matter here (they make `image-size` throw);
/// `Some(())` = completed (with or without an orientation).
fn exif_orientation(exif: &[u8], big: bool) -> Option<()> {
    const OFFSET: usize = 6 + 8;
    let entries = read_uint(exif, 16, OFFSET, big)?;
    for n in 0..entries as usize {
        let start = OFFSET + 2 + n * 12;
        if start > exif.len() {
            return Some(());
        }
        let block = slice(exif, start, start + 12);
        if read_uint(block, 16, 0, big)? == 274 {
            if read_uint(block, 16, 2, big)? != 3 {
                return Some(());
            }
            if read_uint(block, 32, 4, big)? != 1 {
                return Some(());
            }
            read_uint(block, 16, 8, big)?;
            return Some(());
        }
    }
    Some(())
}

fn jpg_size(full: &[u8]) -> Option<(u32, u32)> {
    let mut input = slice(full, 4, full.len());
    while !input.is_empty() {
        let i = u16be(input, 0)? as usize;
        if i > input.len() {
            return None; // Corrupt JPG, exceeded buffer limits
        }
        if input.get(i) != Some(&0xff) {
            input = &input[1..];
            continue;
        }
        // EXIF (APP1 "Exif"): image-size reads the orientation, which can throw on truncated data.
        if slice(input, 2, 6) == b"Exif" {
            let exif = slice(input, 2, i);
            let align = slice(exif, 6, 8);
            if align == b"MM" || align == b"II" {
                exif_orientation(exif, align == b"MM")?;
            }
        }
        if matches!(input.get(i + 1), Some(0xc0..=0xc2)) {
            let height = u16be(input, i + 5)?;
            let width = u16be(input, i + 7)?;
            return Some((width, height));
        }
        input = slice(input, i + 2, input.len());
    }
    None // Invalid JPG, no size found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_jpeg() {
        // 1×1 PNG.
        let png = base64_decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
        );
        assert_eq!(image_size(&png), Some((1, 1)));
        let mut wide = png.clone();
        wide[16..20].copy_from_slice(&1080u32.to_be_bytes());
        wide[20..24].copy_from_slice(&1920u32.to_be_bytes());
        assert_eq!(image_size(&wide), Some((1080, 1920)));
        assert_eq!(image_size(&png[..18]), None);
        let mut bad = png.clone();
        bad[12..16].copy_from_slice(b"XXXX");
        assert_eq!(image_size(&bad), None);
        // Minimal JPEG: SOI, APP0 (len 16), SOF0 with 1920×1080.
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
        jpg.extend_from_slice(&[0; 14]);
        jpg.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08, 0x04, 0x38, 0x07, 0x80, 0x03]);
        jpg.extend_from_slice(&[0; 9]);
        assert_eq!(image_size(&jpg), Some((1920, 1080)));
        assert_eq!(image_size(&jpg[..24]), None);
        assert_eq!(image_size(b"GIF89a"), None);
        assert_eq!(image_size(&[]), None);
    }

    fn base64_decode(s: &str) -> Vec<u8> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(s).unwrap()
    }
}
