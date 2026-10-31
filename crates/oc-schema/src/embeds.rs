// SPDX-License-Identifier: Apache-2.0
//! Embeds (A11): which URLs can be embedded and the iframe each becomes. Only these providers and their
//! official embed hosts are allowed (the iframe `src` is derived here, never taken from oEmbed HTML).

use std::sync::LazyLock;

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::elements::EmbedProvider;
use crate::js;
use crate::macros::string_enum;
use crate::svg::re;

string_enum! {
    pub enum EmbedKind { Video = "video", Audio = "audio", Map = "map", Design = "design", Code = "code" }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedTarget {
    pub provider: EmbedProvider,
    pub src: String,
    pub kind: EmbedKind,
    pub aspect: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oembed: Option<String>,
}

/// Hosts an embed iframe may load (CSP frame-src and the website builder use this list).
pub const EMBED_FRAME_HOSTS: &[&str] = &[
    "https://www.youtube-nocookie.com",
    "https://player.vimeo.com",
    "https://open.spotify.com",
    "https://w.soundcloud.com",
    "https://www.google.com",
    "https://www.loom.com",
    "https://www.figma.com",
    "https://embed.figma.com",
    "https://codepen.io",
];

struct Res {
    yt_path: Regex,
    yt_id: Regex,
    vimeo: Regex,
    spotify: Regex,
    maps_at: Regex,
    maps_place: Regex,
    loom: Regex,
    figma: Regex,
    codepen: Regex,
}

static RES: LazyLock<Res> = LazyLock::new(|| Res {
    yt_path: re(r"^/(?:shorts|embed|live)/([A-Za-z0-9_-]{6,})"),
    yt_id: re(r"^[A-Za-z0-9_-]{6,20}$"),
    vimeo: re(r"([0-9]{5,})"),
    spotify: re(r"^/(?:embed/)?(track|album|playlist|episode|show|artist)/([A-Za-z0-9]{10,})"),
    maps_at: re(r"@(-?[0-9]+\.[0-9]+),(-?[0-9]+\.[0-9]+),([0-9]+(?:\.[0-9]+)?)z"),
    maps_place: re(r"/maps/place/([^/]+)"),
    loom: re(r"/(?:share|embed)/([a-f0-9]{20,})"),
    figma: re(r"^/(file|design|proto|board|slides)/"),
    codepen: re(r"^/([A-Za-z0-9_-]+)/(?:pen|full|details)/([A-Za-z0-9_]+)"),
});

fn cap(r: &Regex, s: &str, i: usize) -> Option<String> {
    r.captures(s).ok().flatten().and_then(|c| c.get(i).map(|m| m.as_str().to_string()))
}

fn param(u: &Url, key: &str) -> Option<String> {
    u.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

/// The iframe target for an embeddable URL, or `None`.
pub fn parse_embed_url(raw: &str) -> Option<EmbedTarget> {
    let r = &*RES;
    let u = Url::parse(js::trim(raw)).ok()?;
    if u.scheme() != "https" && u.scheme() != "http" {
        return None;
    }
    let hostname = u.host_str().unwrap_or("");
    let host = hostname.strip_prefix("www.").unwrap_or(hostname);
    let host = host.strip_prefix("m.").unwrap_or(host);
    let path = u.path();
    let enc = js::encode_uri_component;
    let target = |provider, kind, aspect, src: String, oembed: Option<String>| {
        Some(EmbedTarget { provider, src, kind, aspect, oembed })
    };

    if host == "youtube.com" || host == "youtu.be" || host == "music.youtube.com" {
        let id = if host == "youtu.be" {
            Some(path.get(1..).unwrap_or("").to_string())
        } else {
            param(&u, "v").or_else(|| cap(&r.yt_path, path, 1))
        };
        let id = id.filter(|id| !id.is_empty() && r.yt_id.is_match(id).unwrap_or(false))?;
        let start = match param(&u, "t").filter(|t| !t.is_empty()) {
            Some(t) => {
                let n = js::parse_int10(&t);
                format!("?start={}", js::num_to_string(if n.is_nan() || n == 0.0 { 0.0 } else { n }))
            }
            None => String::new(),
        };
        return target(
            EmbedProvider::Youtube,
            EmbedKind::Video,
            16.0 / 9.0,
            format!("https://www.youtube-nocookie.com/embed/{id}{start}"),
            Some(format!(
                "https://www.youtube.com/oembed?format=json&url={}",
                enc(&format!("https://www.youtube.com/watch?v={id}"))
            )),
        );
    }
    if host == "vimeo.com" || host == "player.vimeo.com" {
        let id = cap(&r.vimeo, path, 1)?;
        return target(
            EmbedProvider::Vimeo,
            EmbedKind::Video,
            16.0 / 9.0,
            format!("https://player.vimeo.com/video/{id}"),
            Some(format!("https://vimeo.com/api/oembed.json?url={}", enc(&format!("https://vimeo.com/{id}")))),
        );
    }
    if host == "open.spotify.com" {
        let kind = cap(&r.spotify, path, 1)?;
        let id = cap(&r.spotify, path, 2)?;
        let aspect = if kind == "track" || kind == "episode" { 4.0 } else { 1.0 };
        return target(
            EmbedProvider::Spotify,
            EmbedKind::Audio,
            aspect,
            format!("https://open.spotify.com/embed/{kind}/{id}"),
            Some(format!(
                "https://open.spotify.com/oembed?url={}",
                enc(&format!("https://open.spotify.com/{kind}/{id}"))
            )),
        );
    }
    if host == "soundcloud.com" {
        if path.split('/').filter(|p| !p.is_empty()).count() < 2 {
            return None;
        }
        let clean = format!("https://soundcloud.com{path}");
        return target(
            EmbedProvider::Soundcloud,
            EmbedKind::Audio,
            3.0,
            format!("https://w.soundcloud.com/player/?url={}&visual=true", enc(&clean)),
            Some(format!("https://soundcloud.com/oembed?format=json&url={}", enc(&clean))),
        );
    }
    if (host == "google.com" && path.starts_with("/maps"))
        || host == "maps.google.com"
        || host == "maps.app.goo.gl"
        || host == "goo.gl"
    {
        let at = r.maps_at.captures(path).ok().flatten();
        let place = cap(&r.maps_place, path, 1);
        let q = match param(&u, "q") {
            Some(q) => Some(q),
            None => match (&place, &at) {
                // decodeURIComponent throws on malformed escapes in TS; here the URL is rejected.
                (Some(p), _) => Some(js::decode_uri_component(&p.replace('+', " "))?),
                (None, Some(a)) => Some(format!("{},{}", &a[1], &a[2])),
                (None, None) => None,
            },
        };
        let q = q.filter(|q| !q.is_empty())?;
        let z = at
            .as_ref()
            .map(|a| format!("&z={}", js::num_to_string(js::round(js::string_to_number(&a[3])))))
            .unwrap_or_default();
        return target(
            EmbedProvider::Maps,
            EmbedKind::Map,
            4.0 / 3.0,
            format!("https://www.google.com/maps?q={}{z}&output=embed", enc(&q)),
            None,
        );
    }
    if host == "loom.com" {
        let id = cap(&r.loom, path, 1)?;
        return target(
            EmbedProvider::Loom,
            EmbedKind::Video,
            16.0 / 9.0,
            format!("https://www.loom.com/embed/{id}"),
            Some(format!("https://www.loom.com/v1/oembed?url={}", enc(&format!("https://www.loom.com/share/{id}")))),
        );
    }
    if host == "figma.com" {
        if !r.figma.is_match(path).unwrap_or(false) {
            return None;
        }
        return target(
            EmbedProvider::Figma,
            EmbedKind::Design,
            16.0 / 10.0,
            format!("https://www.figma.com/embed?embed_host=pinselo&url={}", enc(u.as_str())),
            None,
        );
    }
    if host == "codepen.io" {
        let user = cap(&r.codepen, path, 1)?;
        let pen = cap(&r.codepen, path, 2)?;
        return target(
            EmbedProvider::Codepen,
            EmbedKind::Code,
            16.0 / 10.0,
            format!("https://codepen.io/{user}/embed/{pen}?default-tab=result"),
            Some(format!(
                "https://codepen.io/api/oembed?format=json&url={}",
                enc(&format!("https://codepen.io/{user}/pen/{pen}"))
            )),
        );
    }
    None
}
