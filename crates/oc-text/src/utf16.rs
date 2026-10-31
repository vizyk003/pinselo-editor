// SPDX-License-Identifier: Apache-2.0
//! UTF-8 byte offset ↔ UTF-16 offset mapping. Public offsets are UTF-16 (the JS shell's string indices, and what
//! SkParagraph reported); parley works in UTF-8 bytes.

pub(crate) struct Utf16Map {
    /// `u16_at[byte]` for every char boundary (and the end); other entries hold the next boundary's value.
    u16_at: Vec<usize>,
}

impl Utf16Map {
    pub(crate) fn new(s: &str) -> Self {
        let mut u16_at = vec![0usize; s.len() + 1];
        let mut u = 0usize;
        let mut last = 0usize;
        for (b, c) in s.char_indices() {
            for slot in &mut u16_at[last..=b] {
                *slot = u;
            }
            u += c.len_utf16();
            last = b + 1;
        }
        for slot in &mut u16_at[last..] {
            *slot = u;
        }
        Utf16Map { u16_at }
    }

    pub(crate) fn at(&self, byte: usize) -> usize {
        self.u16_at[byte.min(self.u16_at.len() - 1)]
    }
}

/// UTF-16 offset → byte offset in `s` (clamped; offsets inside a surrogate pair round down).
pub(crate) fn byte_at_utf16(s: &str, u16: usize) -> usize {
    let mut u = 0usize;
    for (b, c) in s.char_indices() {
        if u + c.len_utf16() > u16 {
            return b;
        }
        u += c.len_utf16();
    }
    s.len()
}
