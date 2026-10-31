// SPDX-License-Identifier: AGPL-3.0-only
//! A1-style addressing: column letters, cell/range parsing and relative-reference shifting for paste.

use serde::{Deserialize, Serialize};

/// A cell position (0-based row and column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellAddr {
    pub r: usize,
    pub c: usize,
}

/// Inclusive, normalised (r0 ≤ r1, c0 ≤ c1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

impl Range {
    pub fn cell(r: usize, c: usize) -> Range {
        Range { r0: r, c0: c, r1: r, c1: c }
    }
    pub fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }
}

/// 0 → "A", 25 → "Z", 26 → "AA".
pub fn col_name(c: usize) -> String {
    let mut s = Vec::new();
    let mut n = c + 1;
    while n > 0 {
        let m = (n - 1) % 26;
        s.push(b'A' + m as u8);
        n = (n - 1) / 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// "A" → 0, "AA" → 26; `None` when not 1–3 ASCII letters.
pub fn col_index(name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > 3 || !name.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut n = 0usize;
    for b in name.bytes() {
        n = n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize;
    }
    Some(n - 1)
}

/// `a1(r, c)`: "B3" for (2, 1).
pub fn a1(r: usize, c: usize) -> String {
    format!("{}{}", col_name(c), r + 1)
}

/// "B2", "$B$2" → position (`None` for anything else, including row 0).
pub fn parse_a1(s: &str) -> Option<CellAddr> {
    let s = oc_schema::js::trim(s);
    let b = s.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'$') {
        i += 1;
    }
    let cs = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    let col = &s[cs..i];
    if col.is_empty() || col.len() > 3 {
        return None;
    }
    if b.get(i) == Some(&b'$') {
        i += 1;
    }
    let rs = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let row = &s[rs..i];
    if i != b.len() || row.is_empty() || row.len() > 7 {
        return None;
    }
    let r: usize = row.parse().ok()?;
    if r == 0 {
        return None;
    }
    Some(CellAddr { r: r - 1, c: col_index(col)? })
}

pub fn norm_range(a: CellAddr, b: CellAddr) -> Range {
    Range { r0: a.r.min(b.r), c0: a.c.min(b.c), r1: a.r.max(b.r), c1: a.c.max(b.c) }
}

/// A parsed range reference with its optional sheet name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    pub range: Range,
}

/// "B2", "A1:C5" or "'My tab'!A1:C5" → sheet name (if any) and range.
pub fn parse_range(s: &str) -> Option<RangeRef> {
    let mut rest = oc_schema::js::trim(s);
    let mut sheet = None;
    if let Some(bang) = rest.rfind('!')
        && bang > 0
    {
        let mut name = rest[..bang].to_string();
        if name.len() >= 2 && name.starts_with('\'') && name.ends_with('\'') {
            name = name[1..name.len() - 1].replace("''", "'");
        } else if name == "'" {
            // `"'".startsWith("'") && endsWith("'")` → slice(1, -1) is empty.
            name = String::new();
        }
        sheet = Some(name);
        rest = &rest[bang + 1..];
    }
    let mut parts = rest.split(':');
    let a = parts.next().unwrap_or("");
    let b = parts.next();
    let pa = if a.is_empty() { None } else { parse_a1(a) };
    let pb = match b {
        None => pa,
        Some(b) => parse_a1(b),
    };
    let (pa, pb) = (pa?, pb?);
    Some(RangeRef { sheet, range: norm_range(pa, pb) })
}

pub fn format_range(r: &Range) -> String {
    let a = a1(r.r0, r.c0);
    if r.r0 == r.r1 && r.c0 == r.c1 { a } else { format!("{a}:{}", a1(r.r1, r.c1)) }
}

/// Sheet name as it must appear in a formula reference.
pub fn quote_sheet(name: &str) -> String {
    let b = name.as_bytes();
    let simple = !b.is_empty()
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    if simple { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

/// Shift the relative references of a formula by (dr, dc) — what copy/paste and fill do. Absolute parts
/// (`$A`, `$1`) stay; string literals and quoted sheet names are left alone; a reference pushed off the
/// grid becomes `#REF!`. Non-formulas are returned unchanged.
pub fn shift_formula(src: &str, dr: i64, dc: i64) -> String {
    if !src.starts_with('=') || (dr == 0 && dc == 0) {
        return src.to_string();
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' || ch == '\'' {
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == ch {
                    if chars.get(j + 1) == Some(&ch) {
                        j += 2;
                    } else {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            let end = (j + 1).min(chars.len());
            out.extend(&chars[i..end]);
            i = j + 1;
            continue;
        }
        let prev_ok = i == 0 || !is_ident_char(chars[i - 1]);
        if prev_ok && let Some((len, c_abs, col, r_abs, row)) = match_ref(&chars, i) {
            let next = chars.get(i + len).copied();
            if !next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '_' || n == '(' || n == '!') {
                let c = col_index(&col).map_or(-1, |x| x as i64) + if c_abs { 0 } else { dc };
                let r = row - 1 + if r_abs { 0 } else { dr };
                if c < 0 || r < 0 {
                    out.push_str("#REF!");
                } else {
                    out.push_str(&format!(
                        "{}{}{}{}",
                        if c_abs { "$" } else { "" },
                        col_name(c as usize),
                        if r_abs { "$" } else { "" },
                        r + 1
                    ));
                }
                i += len;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Sticky match of `(\$?)([A-Za-z]{1,3})(\$?)(\d{1,7})` at `i` (greedy, with the regex's backtracking on the
/// letter run: 1–3 letters must be followed directly by `$` or a digit).
fn match_ref(chars: &[char], i: usize) -> Option<(usize, bool, String, bool, i64)> {
    let mut j = i;
    let c_abs = chars.get(j) == Some(&'$');
    if c_abs {
        j += 1;
    }
    let ls = j;
    while j < chars.len() && j - ls < 3 && chars[j].is_ascii_alphabetic() {
        j += 1;
    }
    if j == ls {
        return None;
    }
    let col: String = chars[ls..j].iter().collect();
    let r_abs = chars.get(j) == Some(&'$');
    if r_abs {
        j += 1;
    }
    let ds = j;
    while j < chars.len() && j - ds < 7 && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j == ds {
        return None;
    }
    let row: i64 = chars[ds..j].iter().collect::<String>().parse().ok()?;
    Some((j - i, c_abs, col, r_abs, row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_index("aa"), Some(26));
        assert_eq!(col_index("A1"), None);
        assert_eq!(parse_a1(" $b$2 "), Some(CellAddr { r: 1, c: 1 }));
        assert_eq!(parse_a1("A0"), None);
    }
}
