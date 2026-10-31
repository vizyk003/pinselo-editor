// SPDX-License-Identifier: Apache-2.0
//! Design size presets, Create-dialog helpers and unit conversion (`presets.ts`).

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::design::Surface;
use crate::factory::MetaInit;
use crate::js;
use crate::macros::string_enum;
use crate::primitives::Unit;

string_enum! {
    /// A preset's group (`PresetGroup`).
    pub enum PresetGroup {
        Social = "Social media", Presentations = "Presentations", Print = "Print", Video = "Video",
        Marketing = "Marketing", Documents = "Documents", Web = "Web", Whiteboard = "Whiteboard",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SizePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub group: PresetGroup,
    pub w: f64,
    pub h: f64,
    /// Physical unit shown in the UI. Stored dims are always px @96 dpi.
    pub unit: Unit,
    pub surface: Surface,
    /// Platform name (a proper noun, not translated) used for platform tabs and search, e.g. `Instagram`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<&'static str>,
    /// Shown under "Popular" in the Create dialog (`popular?: true`; omitted from JSON when false).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub popular: bool,
    /// Extra search words (English; the translated label is searched too). Omitted from JSON when empty.
    #[serde(skip_serializing_if = "no_keywords")]
    pub keywords: &'static [&'static str],
}

fn no_keywords(k: &&'static [&'static str]) -> bool {
    k.is_empty()
}

impl SizePreset {
    fn platform(mut self, name: &'static str) -> Self {
        self.platform = Some(name);
        self
    }
    fn popular(mut self) -> Self {
        self.popular = true;
        self
    }
    fn keywords(mut self, words: &'static [&'static str]) -> Self {
        self.keywords = words;
        self
    }
}

pub(crate) const MM: f64 = 96.0 / 25.4;
pub(crate) const IN: f64 = 96.0;
pub(crate) fn mm(v: f64) -> f64 {
    js::round(v * MM * 100.0) / 100.0
}
pub(crate) fn inch(v: f64) -> f64 {
    v * IN
}

/// Every preset. Keep every existing id stable: designs, templates (`preset` column), app shortcuts and the v1 API
/// refer to them. Social and video sizes follow each platform's published recommendation (2026).
pub static SIZE_PRESETS: LazyLock<Vec<SizePreset>> = LazyLock::new(|| {
    use PresetGroup as G;
    use Surface as S;
    use Unit as U;
    let p = |id, label, group, w, h, unit, surface| SizePreset { id, label, group, w, h, unit, surface, platform: None, popular: false, keywords: &[] };
    vec![
        // ---- Social media (g10)
        p("ig-post", "Instagram Post (Square)", G::Social, 1080.0, 1080.0, U::Px, S::Graphic).platform("Instagram").popular(),
        p("ig-portrait", "Instagram Post (Portrait)", G::Social, 1080.0, 1350.0, U::Px, S::Graphic).platform("Instagram"),
        p("ig-post-34", "Instagram Post (3:4)", G::Social, 1080.0, 1440.0, U::Px, S::Graphic).platform("Instagram"),
        p("ig-landscape", "Instagram Post (Landscape)", G::Social, 1080.0, 566.0, U::Px, S::Graphic).platform("Instagram"),
        p("ig-story", "Instagram Story", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("Instagram").popular(),
        p("ig-ad", "Instagram Ad", G::Social, 1080.0, 1080.0, U::Px, S::Graphic).platform("Instagram").keywords(&["advert", "sponsored"]),
        p("ig-story-ad", "Instagram Story Ad", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("Instagram").keywords(&["advert", "sponsored"]),
        p("ig-highlight", "Instagram Highlight Cover", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("Instagram"),
        p("ig-profile", "Instagram Profile Picture", G::Social, 320.0, 320.0, U::Px, S::Graphic).platform("Instagram").keywords(&["avatar"]),
        p("fb-post", "Facebook Post", G::Social, 1200.0, 630.0, U::Px, S::Graphic).platform("Facebook").popular(),
        p("fb-post-square", "Facebook Post (Square)", G::Social, 1080.0, 1080.0, U::Px, S::Graphic).platform("Facebook"),
        p("fb-cover", "Facebook Cover", G::Social, 1640.0, 624.0, U::Px, S::Graphic).platform("Facebook").keywords(&["banner", "header"]),
        p("fb-event-cover", "Facebook Event Cover", G::Social, 1920.0, 1005.0, U::Px, S::Graphic).platform("Facebook").keywords(&["banner"]),
        p("fb-group-cover", "Facebook Group Cover", G::Social, 1640.0, 856.0, U::Px, S::Graphic).platform("Facebook").keywords(&["banner"]),
        p("fb-story", "Facebook Story", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("Facebook"),
        p("fb-ad", "Facebook Ad", G::Social, 1080.0, 1080.0, U::Px, S::Graphic).platform("Facebook").keywords(&["advert", "sponsored"]),
        p("fb-ad-landscape", "Facebook Ad (Landscape)", G::Social, 1200.0, 628.0, U::Px, S::Graphic).platform("Facebook").keywords(&["advert", "sponsored"]),
        p("fb-profile", "Facebook Profile Picture", G::Social, 720.0, 720.0, U::Px, S::Graphic).platform("Facebook").keywords(&["avatar"]),
        p("tiktok-photo", "TikTok Photo Post", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("TikTok").keywords(&["carousel"]),
        p("tiktok-profile", "TikTok Profile Picture", G::Social, 200.0, 200.0, U::Px, S::Graphic).platform("TikTok").keywords(&["avatar"]),
        p("x-post", "X Post", G::Social, 1600.0, 900.0, U::Px, S::Graphic).platform("X").keywords(&["twitter", "tweet"]),
        p("x-header", "X Header", G::Social, 1500.0, 500.0, U::Px, S::Graphic).platform("X").keywords(&["twitter", "banner", "cover"]),
        p("x-profile", "X Profile Picture", G::Social, 400.0, 400.0, U::Px, S::Graphic).platform("X").keywords(&["twitter", "avatar"]),
        p("x-ad", "X Ad", G::Social, 800.0, 418.0, U::Px, S::Graphic).platform("X").keywords(&["twitter", "advert", "website card"]),
        p("linkedin-post", "LinkedIn Post", G::Social, 1200.0, 1200.0, U::Px, S::Graphic).platform("LinkedIn"),
        p("linkedin-post-landscape", "LinkedIn Post (Landscape)", G::Social, 1200.0, 627.0, U::Px, S::Graphic).platform("LinkedIn"),
        p("linkedin-carousel", "LinkedIn Carousel", G::Social, 1080.0, 1350.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["document", "slides"]),
        p("linkedin-article", "LinkedIn Article Cover", G::Social, 1920.0, 1080.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["newsletter", "blog"]),
        p("linkedin-ad", "LinkedIn Ad", G::Social, 1200.0, 627.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["advert", "sponsored"]),
        p("linkedin-banner", "LinkedIn Banner", G::Social, 1584.0, 396.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["cover", "background"]),
        p("linkedin-company-cover", "LinkedIn Company Cover", G::Social, 1128.0, 191.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["page", "banner"]),
        p("linkedin-event-cover", "LinkedIn Event Cover", G::Social, 1776.0, 444.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["banner"]),
        p("linkedin-profile", "LinkedIn Profile Picture", G::Social, 400.0, 400.0, U::Px, S::Graphic).platform("LinkedIn").keywords(&["avatar"]),
        p("pinterest-pin", "Pinterest Pin", G::Social, 1000.0, 1500.0, U::Px, S::Graphic).platform("Pinterest"),
        p("pinterest-square", "Pinterest Pin (Square)", G::Social, 1000.0, 1000.0, U::Px, S::Graphic).platform("Pinterest"),
        p("pinterest-ad", "Pinterest Ad", G::Social, 1000.0, 1500.0, U::Px, S::Graphic).platform("Pinterest").keywords(&["advert", "promoted"]),
        p("pinterest-board-cover", "Pinterest Board Cover", G::Social, 600.0, 600.0, U::Px, S::Graphic).platform("Pinterest"),
        p("yt-thumb", "YouTube Thumbnail", G::Social, 1280.0, 720.0, U::Px, S::Graphic).platform("YouTube").popular(),
        p("yt-banner", "YouTube Banner", G::Social, 2560.0, 1440.0, U::Px, S::Graphic).platform("YouTube").keywords(&["channel art", "cover"]),
        p("yt-profile", "YouTube Profile Picture", G::Social, 800.0, 800.0, U::Px, S::Graphic).platform("YouTube").keywords(&["avatar"]),
        p("yt-community", "YouTube Community Post", G::Social, 1080.0, 1080.0, U::Px, S::Graphic).platform("YouTube"),
        p("yt-end-screen", "YouTube End Screen", G::Social, 1920.0, 1080.0, U::Px, S::Graphic).platform("YouTube").keywords(&["outro"]),
        p("whatsapp-status", "WhatsApp Status", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("WhatsApp").keywords(&["story"]),
        p("whatsapp-profile", "WhatsApp Profile Picture", G::Social, 500.0, 500.0, U::Px, S::Graphic).platform("WhatsApp").keywords(&["avatar"]),
        p("snapchat-story", "Snapchat Story", G::Social, 1080.0, 1920.0, U::Px, S::Graphic).platform("Snapchat"),
        p("snapchat-geofilter", "Snapchat Geofilter", G::Social, 1080.0, 2340.0, U::Px, S::Graphic).platform("Snapchat").keywords(&["filter", "lens"]),
        p("threads-post", "Threads Post", G::Social, 1080.0, 1350.0, U::Px, S::Graphic).platform("Threads"),
        p("twitch-banner", "Twitch Banner", G::Social, 1200.0, 480.0, U::Px, S::Graphic).platform("Twitch").keywords(&["profile banner", "cover"]),
        p("twitch-offline", "Twitch Offline Screen", G::Social, 1920.0, 1080.0, U::Px, S::Graphic).platform("Twitch").keywords(&["stream"]),
        p("discord-banner", "Discord Server Banner", G::Social, 960.0, 540.0, U::Px, S::Graphic).platform("Discord"),
        p("etsy-banner", "Etsy Shop Banner", G::Social, 3360.0, 840.0, U::Px, S::Graphic).platform("Etsy").keywords(&["shop", "cover"]),

        // ---- Video (g11): these open the timeline-first `video` surface.
        p("video", "Video (Landscape)", G::Video, 1920.0, 1080.0, U::Px, S::Video).popular().keywords(&["16:9", "movie", "film", "hd"]),
        p("video-mobile", "Mobile Video", G::Video, 1080.0, 1920.0, U::Px, S::Video).keywords(&["vertical", "9:16", "portrait"]),
        p("video-square", "Square Video", G::Video, 1080.0, 1080.0, U::Px, S::Video).keywords(&["1:1"]),
        p("video-4k", "Video (4K)", G::Video, 3840.0, 2160.0, U::Px, S::Video).keywords(&["uhd", "16:9"]),
        p("tiktok-video", "TikTok Video", G::Video, 1080.0, 1920.0, U::Px, S::Video).platform("TikTok").popular(),
        p("yt-video", "YouTube Video", G::Video, 1920.0, 1080.0, U::Px, S::Video).platform("YouTube"),
        p("yt-shorts", "YouTube Shorts", G::Video, 1080.0, 1920.0, U::Px, S::Video).platform("YouTube").keywords(&["short"]),
        p("ig-reel", "Instagram Reel", G::Video, 1080.0, 1920.0, U::Px, S::Video).platform("Instagram").keywords(&["reels"]),
        p("ig-video", "Instagram Video Post", G::Video, 1080.0, 1350.0, U::Px, S::Video).platform("Instagram"),
        p("fb-video", "Facebook Video", G::Video, 1280.0, 720.0, U::Px, S::Video).platform("Facebook"),
        p("fb-reel", "Facebook Reel", G::Video, 1080.0, 1920.0, U::Px, S::Video).platform("Facebook").keywords(&["reels"]),
        p("linkedin-video", "LinkedIn Video", G::Video, 1920.0, 1080.0, U::Px, S::Video).platform("LinkedIn"),
        p("x-video", "X Video", G::Video, 1280.0, 720.0, U::Px, S::Video).platform("X").keywords(&["twitter"]),
        p("pinterest-video", "Pinterest Video Pin", G::Video, 1080.0, 1920.0, U::Px, S::Video).platform("Pinterest"),

        // ---- Presentations and whiteboards
        p("presentation", "Presentation (16:9)", G::Presentations, 1920.0, 1080.0, U::Px, S::Presentation).popular().keywords(&["slides", "deck"]),
        p("presentation-43", "Presentation (4:3)", G::Presentations, 1024.0, 768.0, U::Px, S::Presentation).keywords(&["slides", "deck"]),
        p("presentation-mobile", "Mobile-first Presentation", G::Presentations, 1080.0, 1920.0, U::Px, S::Presentation).keywords(&["slides", "vertical"]),
        p("whiteboard", "Whiteboard", G::Whiteboard, 4000.0, 3000.0, U::Px, S::Whiteboard).popular().keywords(&["brainstorm", "mind map", "board"]),

        // ---- Documents (g13). 'doc' is the pageless, flowing document; the others are fixed pages.
        p("doc", "Doc", G::Documents, 816.0, 1056.0, U::Px, S::Doc).popular().keywords(&["pageless", "document", "write"]),
        p("a4", "A4 Document", G::Documents, mm(210.0), mm(297.0), U::Mm, S::Print).popular(),
        p("a3", "A3 Document", G::Documents, mm(297.0), mm(420.0), U::Mm, S::Print),
        p("a5", "A5 Document", G::Documents, mm(148.0), mm(210.0), U::Mm, S::Print),
        p("letter", "US Letter Document", G::Documents, inch(8.5), inch(11.0), U::In, S::Print),
        p("legal", "US Legal Document", G::Documents, inch(8.5), inch(14.0), U::In, S::Print),
        p("resume", "Resume (A4)", G::Documents, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["cv"]),
        p("resume-letter", "Resume (US Letter)", G::Documents, inch(8.5), inch(11.0), U::In, S::Print).keywords(&["cv"]),
        p("report", "Report (A4)", G::Documents, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["proposal"]),
        // Spreadsheet (A15). The size is the export page (A4 landscape) the grid is paginated onto.
        p("sheet", "Sheet", G::Documents, 1123.0, 794.0, U::Px, S::Sheet).keywords(&["spreadsheet", "table", "excel"]),

        // ---- Print design sizes (g12). Sizes only: ordering prints is a separate flow.
        p("poster", "Poster (18 × 24 in)", G::Print, inch(18.0), inch(24.0), U::In, S::Print).popular(),
        p("poster-24x36", "Poster (24 × 36 in)", G::Print, inch(24.0), inch(36.0), U::In, S::Print),
        p("poster-a3", "Poster (A3)", G::Print, mm(297.0), mm(420.0), U::Mm, S::Print),
        p("poster-a2", "Poster (A2)", G::Print, mm(420.0), mm(594.0), U::Mm, S::Print),
        p("flyer", "Flyer (A5)", G::Print, mm(148.0), mm(210.0), U::Mm, S::Print).popular().keywords(&["leaflet"]),
        p("flyer-a4", "Flyer (A4)", G::Print, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["leaflet"]),
        p("flyer-letter", "Flyer (US Letter)", G::Print, inch(8.5), inch(11.0), U::In, S::Print).keywords(&["leaflet"]),
        p("business-card", "Business Card", G::Print, inch(3.5), inch(2.0), U::In, S::Print).popular(),
        p("invitation", "Invitation (5 × 7 in)", G::Print, inch(5.0), inch(7.0), U::In, S::Print).keywords(&["wedding", "party", "card"]),
        p("card-folded", "Folded Card (10 × 7 in, unfolded)", G::Print, inch(10.0), inch(7.0), U::In, S::Print).keywords(&["greeting", "birthday", "card"]),
        p("postcard", "Postcard (6 × 4 in)", G::Print, inch(6.0), inch(4.0), U::In, S::Print).keywords(&["card"]),
        p("postcard-a6", "Postcard (A6)", G::Print, mm(148.0), mm(105.0), U::Mm, S::Print).keywords(&["card"]),
        p("brochure", "Brochure (Tri-fold, US Letter)", G::Print, inch(11.0), inch(8.5), U::In, S::Print).keywords(&["pamphlet", "trifold"]),
        p("brochure-a4", "Brochure (Tri-fold, A4)", G::Print, mm(297.0), mm(210.0), U::Mm, S::Print).keywords(&["pamphlet", "trifold"]),
        p("menu", "Menu (US Letter)", G::Print, inch(8.5), inch(11.0), U::In, S::Print).keywords(&["restaurant", "food"]),
        p("menu-slim", "Menu (4.25 × 11 in)", G::Print, inch(4.25), inch(11.0), U::In, S::Print).keywords(&["restaurant", "food"]),
        p("magazine", "Magazine Page (8.5 × 11 in)", G::Print, inch(8.5), inch(11.0), U::In, S::Print).keywords(&["cover", "zine"]),
        p("magazine-a4", "Magazine Page (A4)", G::Print, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["cover", "zine"]),
        p("photo-book", "Photo Album (8 × 8 in)", G::Print, inch(8.0), inch(8.0), U::In, S::Print).keywords(&["photo book", "scrapbook"]),
        p("photo-book-landscape", "Photo Album (11 × 8.5 in)", G::Print, inch(11.0), inch(8.5), U::In, S::Print).keywords(&["photo book", "scrapbook"]),
        p("gift-certificate", "Gift Certificate (8 × 3.5 in)", G::Print, inch(8.0), inch(3.5), U::In, S::Print).keywords(&["voucher", "gift card"]),
        p("certificate", "Certificate (11 × 8.5 in)", G::Print, inch(11.0), inch(8.5), U::In, S::Print).keywords(&["award", "diploma"]),
        p("bookmark", "Bookmark (2 × 6 in)", G::Print, inch(2.0), inch(6.0), U::In, S::Print),
        p("calendar", "Wall Calendar (11 × 8.5 in)", G::Print, inch(11.0), inch(8.5), U::In, S::Print).keywords(&["month", "planner"]),
        p("calendar-a4", "Calendar (A4)", G::Print, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["month", "planner"]),
        p("planner", "Planner (A5)", G::Print, mm(148.0), mm(210.0), U::Mm, S::Print).keywords(&["journal", "agenda"]),
        p("banner", "Banner (6 × 3 ft)", G::Print, inch(72.0), inch(36.0), U::In, S::Print).keywords(&["vinyl", "sign"]),
        p("banner-rollup", "Roll-up Banner (33 × 79 in)", G::Print, inch(33.0), inch(79.0), U::In, S::Print).keywords(&["pull-up", "stand", "sign"]),
        p("letterhead", "Letterhead (A4)", G::Print, mm(210.0), mm(297.0), U::Mm, S::Print).keywords(&["stationery"]),
        p("letterhead-letter", "Letterhead (US Letter)", G::Print, inch(8.5), inch(11.0), U::In, S::Print).keywords(&["stationery"]),
        p("envelope-dl", "Envelope (DL)", G::Print, mm(220.0), mm(110.0), U::Mm, S::Print).keywords(&["stationery"]),
        p("envelope-10", "Envelope (No. 10)", G::Print, inch(9.5), inch(4.125), U::In, S::Print).keywords(&["stationery"]),
        p("label", "Product Label (4 × 6 in)", G::Print, inch(4.0), inch(6.0), U::In, S::Print).keywords(&["packaging", "tag"]),
        p("sticker", "Sticker (3 × 3 in)", G::Print, inch(3.0), inch(3.0), U::In, S::Print),
        p("rack-card", "Rack Card (4 × 9 in)", G::Print, inch(4.0), inch(9.0), U::In, S::Print),
        p("door-hanger", "Door Hanger (4.25 × 11 in)", G::Print, inch(4.25), inch(11.0), U::In, S::Print),
        p("ticket", "Ticket (5.5 × 2 in)", G::Print, inch(5.5), inch(2.0), U::In, S::Print).keywords(&["event", "raffle"]),
        p("table-tent", "Table Tent (4 × 6 in)", G::Print, inch(4.0), inch(6.0), U::In, S::Print),
        p("book-cover", "Book Cover (6 × 9 in)", G::Print, inch(6.0), inch(9.0), U::In, S::Print).keywords(&["ebook", "novel"]),

        // ---- Marketing
        p("logo", "Logo", G::Marketing, 500.0, 500.0, U::Px, S::Graphic).popular().keywords(&["brand", "icon"]),
        p("email-header", "Email Header", G::Marketing, 600.0, 200.0, U::Px, S::Graphic).keywords(&["newsletter"]),
        p("infographic", "Infographic", G::Marketing, 800.0, 2000.0, U::Px, S::Graphic).keywords(&["chart", "data"]),
        p("ad-leaderboard", "Leaderboard Ad (728 × 90)", G::Marketing, 728.0, 90.0, U::Px, S::Graphic).keywords(&["display ad", "banner ad", "web banner"]),
        p("ad-rectangle", "Medium Rectangle Ad (300 × 250)", G::Marketing, 300.0, 250.0, U::Px, S::Graphic).keywords(&["display ad", "banner ad"]),
        p("ad-skyscraper", "Wide Skyscraper Ad (160 × 600)", G::Marketing, 160.0, 600.0, U::Px, S::Graphic).keywords(&["display ad", "banner ad"]),
        p("zoom-background", "Virtual Background", G::Marketing, 1920.0, 1080.0, U::Px, S::Graphic).keywords(&["zoom", "teams", "meet", "video call"]),
        p("desktop-wallpaper", "Desktop Wallpaper", G::Marketing, 1920.0, 1080.0, U::Px, S::Graphic).keywords(&["background"]),
        p("phone-wallpaper", "Phone Wallpaper", G::Marketing, 1080.0, 1920.0, U::Px, S::Graphic).keywords(&["background", "mobile", "lock screen"]),

        // ---- Web
        p("website", "Website", G::Web, 1366.0, 768.0, U::Px, S::Website).keywords(&["site", "landing page"]),
        // Mobile-first one-page site (link-in-bio): a column of buttons that link out.
        p("link-in-bio", "Link in bio", G::Web, 1080.0, 1920.0, U::Px, S::Website).keywords(&["linktree", "links"]),
        p("email", "Email", G::Web, 600.0, 1600.0, U::Px, S::Email).keywords(&["newsletter", "html"]),
        p("blog-banner", "Blog Banner", G::Web, 1200.0, 630.0, U::Px, S::Graphic).keywords(&["header", "og image", "social preview"]),

    ]
});

/// `findPreset`: the preset with this id (`None` for unknown or empty ids).
pub fn find_preset(id: &str) -> Option<&'static SizePreset> {
    if id.is_empty() {
        return None;
    }
    SIZE_PRESETS.iter().find(|p| p.id == id)
}

string_enum! {
    /// Create-dialog categories (g08). `Suggestions` and `Custom` are not preset groups.
    pub enum PresetCategoryId {
        Suggestions = "suggestions", Social = "social", Video = "video", Presentations = "presentations", Print = "print",
        Documents = "documents", Web = "web", Marketing = "marketing", Whiteboard = "whiteboard", Custom = "custom",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetCategory {
    pub id: PresetCategoryId,
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<PresetGroup>,
}

/// Create-dialog categories in nav order (`PRESET_CATEGORIES`).
pub static PRESET_CATEGORIES: [PresetCategory; 10] = {
    use PresetCategoryId as C;
    use PresetGroup as G;
    const fn c(id: PresetCategoryId, label: &'static str, group: Option<PresetGroup>) -> PresetCategory {
        PresetCategory { id, label, group }
    }
    [
        c(C::Suggestions, "Suggestions", None),
        c(C::Social, "Social media", Some(G::Social)),
        c(C::Video, "Video", Some(G::Video)),
        c(C::Presentations, "Presentations", Some(G::Presentations)),
        c(C::Print, "Print", Some(G::Print)),
        c(C::Documents, "Documents", Some(G::Documents)),
        c(C::Web, "Web", Some(G::Web)),
        c(C::Marketing, "Marketing", Some(G::Marketing)),
        c(C::Whiteboard, "Whiteboard", Some(G::Whiteboard)),
        c(C::Custom, "Custom size", None),
    ]
};

/// `presetsInCategory`: presets listed under a category. Social media also lists the platform video formats
/// (Reels, Shorts...). Categories without a group (and unknown ones) list nothing.
pub fn presets_in_category(cat: &PresetCategoryId) -> Vec<&'static SizePreset> {
    let Some(group) = PRESET_CATEGORIES.iter().find(|c| c.id == *cat).and_then(|c| c.group.as_ref()) else {
        return Vec::new();
    };
    SIZE_PRESETS
        .iter()
        .filter(|p| p.group == *group || (*group == PresetGroup::Social && p.group == PresetGroup::Video && p.platform.is_some_and(|x| !x.is_empty())))
        .collect()
}

/// `presetPlatforms`: platform tabs for a list of presets, in first-seen order.
pub fn preset_platforms(list: &[&SizePreset]) -> Vec<&'static str> {
    let mut seen = HashSet::new();
    list.iter().filter_map(|p| p.platform).filter(|x| !x.is_empty() && seen.insert(*x)).collect()
}

/// `searchPresets` without a translator: every word of `q` must appear in the label, group, platform, keywords,
/// id or size ("1080", "a4"). An empty query matches nothing.
pub fn search_presets(q: &str) -> Vec<&'static SizePreset> {
    search_presets_with(q, |s| s.to_string())
}

/// `searchPresets(q, translate)`: like [`search_presets`], also matching the translated label and group.
pub fn search_presets_with(q: &str, translate: impl Fn(&str) -> String) -> Vec<&'static SizePreset> {
    let q = q.to_lowercase();
    let words: Vec<&str> = q.split(js::is_js_space).filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return Vec::new();
    }
    SIZE_PRESETS
        .iter()
        .filter(|p| {
            let mut hay: Vec<String> = vec![p.label.into(), translate(p.label), p.group.as_str().into(), translate(p.group.as_str()), p.platform.unwrap_or("").into()];
            hay.extend(p.keywords.iter().map(|k| k.to_string()));
            hay.push(p.id.replace('-', " "));
            hay.push(format_dims(p.w, p.h, &p.unit));
            hay.push(format!("{}x{}", js::num_to_string(js::round(p.w)), js::num_to_string(js::round(p.h))));
            let hay = hay.join(" ").to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect()
}

/// `popularPresets`: the Create dialog's "Popular" row, in a stable order.
pub fn popular_presets() -> Vec<&'static SizePreset> {
    SIZE_PRESETS.iter().filter(|p| p.popular).collect()
}

/// One starter per surface for "Try something new" (`TRY_NEW_IDS`).
pub const TRY_NEW_IDS: [&str; 8] = ["video", "whiteboard", "doc", "website", "presentation", "sheet", "email", "link-in-bio"];

/// `tryNewPresets(recentIds, max = 4)`: starters on surfaces the person has not used recently.
pub fn try_new_presets(recent_ids: &[&str], max: usize) -> Vec<&'static SizePreset> {
    let used: HashSet<&Surface> = recent_ids.iter().filter_map(|id| find_preset(id)).map(|p| &p.surface).collect();
    TRY_NEW_IDS.iter().filter_map(|id| find_preset(id)).filter(|p| !used.contains(&p.surface)).take(max).collect()
}

/// `pushRecentPreset(list, id, max = 8)`: a most-recent-first list of preset ids with `id` moved to the front,
/// capped at `max`. `list` is untrusted JSON (e.g. from local storage): non-arrays count as empty, non-string and
/// unknown entries are dropped, and an unknown `id` is not added. Like the TS, only `id` itself is de-duplicated.
pub fn push_recent_preset(list: &Value, id: &str, max: usize) -> Vec<String> {
    let prev: Vec<&str> = match list {
        Value::Array(a) => a.iter().filter_map(Value::as_str).filter(|x| find_preset(x).is_some()).collect(),
        _ => Vec::new(),
    };
    std::iter::once(id)
        .chain(prev.into_iter().filter(|x| *x != id))
        .filter(|x| find_preset(x).is_some())
        .take(max)
        .map(str::to_string)
        .collect()
}

/// Explicit size / title for [`preset_design_init`] (`o`); each field wins over the preset's value.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PresetDesignOverrides {
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub unit: Option<Unit>,
    pub title: Option<String>,
}

/// The fields `emptyDesign` needs for a new design (`presetDesignInit`'s result).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PresetDesignInit {
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
    pub title: String,
    pub surface: Surface,
}

impl PresetDesignInit {
    /// As the `init` of [`crate::factory::empty_design`] / [`crate::factory::make_meta`].
    pub fn meta_init(&self) -> MetaInit {
        MetaInit { w: self.w, h: self.h, unit: Some(self.unit.clone()), title: Some(self.title.clone()), surface: Some(self.surface.clone()), pages: None }
    }
}

impl From<PresetDesignInit> for MetaInit {
    fn from(v: PresetDesignInit) -> Self {
        MetaInit { w: v.w, h: v.h, unit: Some(v.unit), title: Some(v.title), surface: Some(v.surface), pages: None }
    }
}

/// `presetDesignInit`: a new design from a preset and/or an explicit size (custom size wins over the preset's).
pub fn preset_design_init(preset: Option<&SizePreset>, o: &PresetDesignOverrides) -> PresetDesignInit {
    PresetDesignInit {
        w: o.w.or(preset.map(|p| p.w)).unwrap_or(1080.0),
        h: o.h.or(preset.map(|p| p.h)).unwrap_or(1080.0),
        unit: o.unit.clone().or_else(|| preset.map(|p| p.unit.clone())).unwrap_or(Unit::Px),
        title: o.title.clone().unwrap_or_else(|| match preset {
            Some(p) => format!("Untitled – {}", p.label),
            None => "Untitled design".into(),
        }),
        surface: preset.map_or(Surface::Graphic, |p| p.surface.clone()),
    }
}

/// A shortcut in the Custom size form (g14). Sizes are in their own unit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PopularLayout {
    pub id: &'static str,
    pub label: &'static str,
    pub w: f64,
    pub h: f64,
    pub unit: Unit,
}

/// `POPULAR_LAYOUTS`.
pub static POPULAR_LAYOUTS: [PopularLayout; 7] = {
    const fn l(id: &'static str, label: &'static str, w: f64, h: f64, unit: Unit) -> PopularLayout {
        PopularLayout { id, label, w, h, unit }
    }
    [
        l("square", "Square (1:1)", 1080.0, 1080.0, Unit::Px),
        l("portrait", "Portrait (4:5)", 1080.0, 1350.0, Unit::Px),
        l("story", "Story (9:16)", 1080.0, 1920.0, Unit::Px),
        l("widescreen", "Widescreen (16:9)", 1920.0, 1080.0, Unit::Px),
        l("standard", "Standard (4:3)", 1024.0, 768.0, Unit::Px),
        l("a4", "A4", 210.0, 297.0, Unit::Mm),
        l("letter", "US Letter", 8.5, 11.0, Unit::In),
    ]
};

string_enum! {
    /// The side of a locked-ratio size the person changed (`'w' | 'h'`).
    pub enum SizeSide { W = "w", H = "h" }
}

/// `lockedSide(changed, value, ratio, decimals = 2)`: the other side of a size whose aspect ratio (`w / h`) is
/// locked, kept to `decimals` places (two, what the physical-unit inputs show; 0 for whole pixels, never below 1).
/// Non-positive or NaN `value` / `ratio` give 0.
pub fn locked_side(changed: &SizeSide, value: f64, ratio: f64, decimals: i32) -> f64 {
    // `!(value > 0) || !(ratio > 0)`: NaN counts as not positive.
    let positive = |x: f64| x.partial_cmp(&0.0) == Some(std::cmp::Ordering::Greater);
    if !positive(value) || !positive(ratio) {
        return 0.0;
    }
    let v = if *changed == SizeSide::W { value / ratio } else { value * ratio };
    let f = 10f64.powi(decimals);
    let r = js::round(v * f) / f;
    // `Math.max(decimals === 0 ? 1 : 0, r)`: NaN propagates (f64::max would drop it).
    if r.is_nan() { r } else { r.max(if decimals == 0 { 1.0 } else { 0.0 }) }
}

/// px per unit (`UNIT_PX`). Unknown units count as px.
pub fn unit_px(unit: &Unit) -> f64 {
    match unit {
        Unit::Px => 1.0,
        Unit::Mm => MM,
        Unit::Cm => MM * 10.0,
        Unit::In => IN,
        Unit::Pt => 96.0 / 72.0,
        // `UNIT_PX[unknown]` is undefined in TS (NaN results).
        Unit::Other(_) => f64::NAN,
    }
}

pub fn px_to_unit(px: f64, unit: &Unit) -> f64 {
    px / unit_px(unit)
}

pub fn unit_to_px(v: f64, unit: &Unit) -> f64 {
    v * unit_px(unit)
}

pub fn format_dims(w: f64, h: f64, unit: &Unit) -> String {
    let f = |v: f64| {
        let n = px_to_unit(v, unit);
        if *unit == Unit::Px {
            js::num_to_string(js::round(n))
        } else {
            js::num_to_string(js::round(n * 10.0) / 10.0)
        }
    };
    format!("{} × {} {}", f(w), f(h), unit)
}
