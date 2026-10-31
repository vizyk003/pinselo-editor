//! Caption helpers: `composeText`, `charCount`, `mediaMime` (publish.ts), `littleText` (linkedin.ts),
//! `tagFacets` (bluesky.ts) and `chunkPlan` (tiktok.ts).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use super::http::js_trim;
use super::types::SocialAdapter;

/// ECMAScript `\s` (WhiteSpace + LineTerminator) as a regex class body; Rust's `\s` differs (U+0085, U+FEFF).
const JS_WS: &str = r"\t\n\x0B\x0C\r \u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";

static WORD_CHAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}_]").unwrap());
static X_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"https?://[^{JS_WS}]+")).unwrap());
static LITTLE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#([\p{L}\p{N}_]+)").unwrap());
static FACET_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(^|[{JS_WS}])#([\p{{L}}\p{{N}}_]{{1,64}})")).unwrap());

/// Caption + hashtags as posted: hashtags not already in the caption are appended after a blank line.
pub fn compose_text(caption: &str, hashtags: &[String]) -> String {
    let tags: Vec<String> = hashtags
        .iter()
        .map(|h| js_trim(h.trim_start_matches('#')).to_string())
        .filter(|h| !h.is_empty())
        .map(|h| format!("#{h}"))
        .collect();
    let body = js_trim(caption);
    let missing: Vec<&str> = tags
        .iter()
        .filter(|t| !contains_tag(body, t))
        .map(String::as_str)
        .collect();
    let tail = missing.join(" ");
    [body, tail.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `new RegExp("(^|\\s)" + escape(tag) + "(?![\\p{L}\\p{N}_])", "u").test(body)`.
fn contains_tag(body: &str, tag: &str) -> bool {
    // Every start position (overlapping occurrences included), like a regex scan.
    body.char_indices()
        .map(|(i, _)| i)
        .filter(|&i| body[i..].starts_with(tag))
        .any(|i| {
            let before_ok = body[..i]
                .chars()
                .next_back()
                .is_none_or(oc_js::is_js_whitespace);
            let after_ok = !WORD_CHAR.is_match(&body[i + tag.len()..]);
            before_ok && after_ok
        })
}

/// Count as the networks do (grapheme clusters; X counts any URL as 23).
pub fn char_count(text: &str, provider: Option<&str>) -> usize {
    if provider == Some("x") {
        let t = X_URL.replace_all(text, "x".repeat(23).as_str());
        t.graphemes(true).count()
    } else {
        text.graphemes(true).count()
    }
}

/// Escape LinkedIn little-text reserved characters; keep #hashtags as hashtag templates.
pub fn little_text(s: &str) -> String {
    fn esc(out: &mut String, x: &str) {
        for c in x.chars() {
            if matches!(
                c,
                '\\' | '|'
                    | '{'
                    | '}'
                    | '@'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '<'
                    | '>'
                    | '#'
                    | '*'
                    | '_'
                    | '~'
            ) {
                out.push('\\');
            }
            out.push(c);
        }
    }
    let mut out = String::new();
    let mut last = 0;
    for m in LITTLE_TAG.captures_iter(s) {
        let whole = m.get(0).unwrap();
        esc(&mut out, &s[last..whole.start()]);
        out.push_str("{hashtag|\\#|");
        out.push_str(&m[1]);
        out.push('}');
        last = whole.end();
    }
    esc(&mut out, &s[last..]);
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteSlice {
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetFeature {
    #[serde(rename = "$type")]
    pub kind: String,
    pub tag: String,
}

/// A Bluesky richtext facet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facet {
    pub index: ByteSlice,
    pub features: Vec<FacetFeature>,
}

/// Hashtag facets with UTF-8 byte ranges, as the Bluesky app produces them.
pub fn tag_facets(text: &str) -> Vec<Facet> {
    FACET_TAG
        .captures_iter(text)
        .map(|m| {
            let byte_start = m.get(1).unwrap().end();
            let tag = m[2].to_string();
            Facet {
                index: ByteSlice {
                    byte_start,
                    byte_end: byte_start + 1 + tag.len(),
                },
                features: vec![FacetFeature {
                    kind: "app.bsky.richtext.facet#tag".into(),
                    tag,
                }],
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkPlan {
    pub chunk_size: usize,
    pub count: usize,
}

const MB: usize = 1024 * 1024;

/// TikTok chunking rules: ≤64 MB in one chunk; otherwise 10 MB chunks with the remainder folded into the last.
pub fn chunk_plan(size: usize) -> ChunkPlan {
    if size <= 64 * MB {
        return ChunkPlan {
            chunk_size: size,
            count: 1,
        };
    }
    let chunk_size = 10 * MB;
    ChunkPlan {
        chunk_size,
        count: size / chunk_size,
    }
}

/// MIME type to render for a post format ('png' | 'jpg' | 'mp4') on an adapter: PNG falls back to JPEG
/// for networks that don't accept it. `None` for an unknown format that has no JPEG fallback (TS `undefined`).
pub fn media_mime(format: &str, a: &dyn SocialAdapter) -> Option<&'static str> {
    if format == "mp4" {
        return Some("video/mp4");
    }
    let want = match format {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        _ => None,
    };
    let types = a.limits().image_types;
    if let Some(w) = want
        && types.contains(&w)
    {
        return Some(w);
    }
    if types.contains(&"image/jpeg") {
        Some("image/jpeg")
    } else {
        want
    }
}
