//! Time in designs for AI assistants: element animations and visibility windows, page lengths and transitions, the
//! timeline as `get_design` shows it, and the video checks of `edit_design` (text shown too briefly to read, text in the
//! part of a Reels or Stories video the app covers, a video too long for its format, a soundtrack that stops early).
//!
//! Assistants give times in seconds; designs store milliseconds. A brand kit's `motion` (milliseconds and px, see
//! `routes::brandhub::motion`) supplies what an animation leaves out, so each brand's videos move in its own style.

use serde_json::{Map, Value, json};

use super::view::{num, truthy};

/// Seconds (as given) → ms.
fn ms(s: f64) -> f64 {
    (s * 1000.0).round()
}

/// ms → seconds, rounded to hundredths, as JSON.
pub fn secs(ms: f64) -> Value {
    super::edit::jnum((ms / 10.0).round() / 100.0)
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// A brand kit's motion rules (`kit.motion`), or none.
#[derive(Debug, Clone, Default)]
pub struct Kit {
    pub name: Option<String>,
    pub m: Value,
}

impl Kit {
    pub fn new(name: Option<String>, motion: Value) -> Kit {
        Kit { name, m: if motion.is_object() { motion } else { json!({}) } }
    }

    /// The kit's defaults for an entrance: `text` for text, `draw` for drawing lines, else `enter`.
    fn enter_defaults(&self, is_text: bool, draw: bool) -> Value {
        let mut d = self.m.get("enter").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        let over = if draw { self.m.get("draw") } else if is_text { self.m.get("text") } else { None };
        if let Some(Value::Object(o)) = over {
            for (k, v) in o {
                d[k.as_str()] = v.clone();
            }
        }
        d
    }

    fn no_bounce(&self) -> bool {
        self.m.get("bounce") == Some(&Value::Bool(false))
    }

    fn rule(&self) -> String {
        match &self.name {
            Some(n) => format!("the brand kit \"{n}\""),
            None => "the brand kit".into(),
        }
    }

    /// Seconds between elements that appear one after another (kit `stagger` ms; default 0.15 s).
    pub fn stagger_s(&self) -> f64 {
        f(&self.m, "stagger").map_or(0.15, |v| v / 1000.0)
    }
}

/// A line's travel direction from its first to its last point (what `draw` reveals along).
fn line_direction(el: &Value) -> Option<&'static str> {
    let pts = el["points"].as_array()?;
    let (a, b) = (pts.first()?, pts.last()?);
    let (dx, dy) = (num(&b[0]) - num(&a[0]), num(&b[1]) - num(&a[1]));
    Some(if dx.abs() >= dy.abs() {
        if dx >= 0.0 { "right" } else { "left" }
    } else if dy >= 0.0 {
        "down"
    } else {
        "up"
    })
}

/// `animation.in` → the element's `anim` (ElementAnim JSON). `el` is the element as it will be (type, points).
pub fn anim_in(spec: &Value, el: &Value, kit: &Kit, extra_delay_s: f64) -> Result<Value, String> {
    let ty_el = el["type"].as_str().unwrap_or("");
    let is_text = ty_el == "text";
    let asked = s(spec, "type");
    let draw = asked == Some("draw") || (asked.is_none() && ty_el == "line" && kit.m.get("draw").is_some());
    let d = kit.enter_defaults(is_text, draw);
    let kind = asked.or_else(|| if draw { Some("draw") } else { s(&d, "type") }).unwrap_or("fade");
    if kit.no_bounce() && kind == "pop" {
        return Err(format!("{} rules out bouncing animations (bounce: false); use scale, zoom or fade instead of pop.", kit.rule()));
    }
    if kind == "typewriter" && !is_text {
        return Err("typewriter is for text elements.".into());
    }
    let (preset, mut direction) = match kind {
        "slide-left" => ("slide", Some("left")),
        "slide-right" => ("slide", Some("right")),
        "slide-up" => ("slide", Some("up")),
        "slide-down" => ("slide", Some("down")),
        "scale" => ("shrink", None),
        "draw" => ("draw", if ty_el == "line" { line_direction(el) } else { None }),
        k => (k, None),
    };
    if let Some(dir) = s(spec, "direction") {
        direction = Some(match dir {
            "left" => "left",
            "right" => "right",
            "up" => "up",
            _ => "down",
        });
    }
    let duration = f(spec, "duration").map(ms).or_else(|| f(&d, "duration")).unwrap_or(if preset == "draw" { 600.0 } else { 500.0 });
    let delay = ms(f(spec, "delay").unwrap_or(0.0) + extra_delay_s);
    let mut a = Map::new();
    a.insert("preset".into(), json!(preset));
    a.insert("durationMs".into(), super::edit::jnum(duration));
    a.insert("delayMs".into(), super::edit::jnum(delay.max(0.0)));
    if let Some(dir) = direction {
        a.insert("direction".into(), json!(dir));
    }
    if let Some(e) = s(spec, "easing").or_else(|| s(&d, "easing")) {
        a.insert("easing".into(), json!(e));
    }
    if ["rise", "drop", "slide"].contains(&preset)
        && let Some(px) = f(spec, "distance").or_else(|| f(&d, "distance"))
    {
        a.insert("distance".into(), super::edit::jnum(px));
    }
    if let Some(by) = s(spec, "by").filter(|b| *b != "element") {
        if !is_text {
            return Err("by (line, word, letter) is for text elements.".into());
        }
        a.insert("by".into(), json!(if by == "letter" { "char" } else { by }));
        a.insert("staggerMs".into(), super::edit::jnum(f(spec, "stagger").map_or(if by == "letter" { 40.0 } else { 120.0 }, ms)));
    } else if kind == "typewriter" {
        a.insert("staggerMs".into(), super::edit::jnum(f(spec, "stagger").map_or(40.0, ms)));
    }
    Ok(Value::Object(a))
}

/// `animation.out` → (`animOut` JSON, when it starts in page seconds if given).
fn anim_out(spec: &Value, kit: &Kit) -> (Value, Option<f64>) {
    let d = kit.m.get("exit").cloned().unwrap_or_else(|| json!({}));
    let preset = match s(spec, "type").or_else(|| s(&d, "type")).unwrap_or("fade") {
        "scale" => "shrink",
        k => k,
    };
    let duration = f(spec, "duration").map(ms).or_else(|| f(&d, "duration")).unwrap_or(400.0);
    (json!({ "preset": preset, "durationMs": super::edit::jnum(duration) }), f(spec, "at"))
}

/// The time fields of an `add_*`, `update` or `animate` operation as an element patch (`anim`, `animOut`, `timing`;
/// `null` removes). `el` is the element as stored (or about to be added); `extra_delay_s` staggers `animate`.
pub fn element_patch(op: &Value, el: &Value, kit: &Kit, extra_delay_s: f64) -> Result<Map<String, Value>, String> {
    let mut p = Map::new();
    let anim = op.get("animation");
    let (spec_in, spec_out) = match anim {
        Some(Value::Null) => (Some(&Value::Null), Some(&Value::Null)),
        Some(a) => (a.get("in"), a.get("out")),
        None => (op.get("in"), op.get("out")),
    };
    match spec_in {
        Some(Value::Null) => {
            p.insert("anim".into(), Value::Null);
        }
        Some(spec) => {
            p.insert("anim".into(), anim_in(spec, el, kit, extra_delay_s)?);
        }
        None => {}
    }
    let mut out_end: Option<f64> = None;
    match spec_out {
        Some(Value::Null) => {
            p.insert("animOut".into(), Value::Null);
        }
        Some(spec) => {
            let (o, at) = anim_out(spec, kit);
            if let Some(at) = at {
                out_end = Some(ms(at) + num(&o["durationMs"]));
            }
            p.insert("animOut".into(), o);
        }
        None => {}
    }
    let from = op.get("visibleFrom");
    let to = op.get("visibleTo");
    if out_end.is_some() && to.is_some_and(|t| !t.is_null()) {
        return Err("Give either visibleTo or animation.out.at, not both: the exit ends where the element disappears.".into());
    }
    if from.is_some() || to.is_some() || out_end.is_some() {
        let cur = &el["timing"];
        let start = match from {
            Some(Value::Null) => 0.0,
            Some(v) => ms(num(v)),
            None => f(cur, "startMs").unwrap_or(0.0),
        };
        let end = match (out_end, to) {
            (Some(e), _) => Some(e),
            (None, Some(Value::Null)) => None,
            (None, Some(v)) => Some(ms(num(v))),
            (None, None) => f(cur, "endMs"),
        };
        if let Some(e) = end
            && e <= start
        {
            return Err(format!("The element would disappear ({} s) before it appears ({} s).", secs(e), secs(start)));
        }
        let timing = match end {
            None if start <= 0.0 => Value::Null,
            None => json!({ "startMs": super::edit::jnum(start) }),
            Some(e) => json!({ "startMs": super::edit::jnum(start), "endMs": super::edit::jnum(e) }),
        };
        p.insert("timing".into(), timing);
    }
    Ok(p)
}

/// Whether an operation sets any time field of an element.
pub fn has_element_time(op: &Value) -> bool {
    ["animation", "visibleFrom", "visibleTo"].iter().any(|k| op.get(*k).is_some())
}

/// `duration` and `transition` of `add_page` / `update_page` as a page patch (`null` removes).
pub fn page_patch(op: &Value, kit: &Kit) -> Map<String, Value> {
    let mut p = Map::new();
    match op.get("duration") {
        Some(Value::Null) => {
            p.insert("durationMs".into(), Value::Null);
        }
        Some(v) => {
            p.insert("durationMs".into(), super::edit::jnum(ms(num(v))));
        }
        None => {}
    }
    match op.get("transition") {
        Some(Value::Null) => {
            p.insert("transition".into(), Value::Null);
        }
        Some(t) => {
            let d = kit.m.get("transition").cloned().unwrap_or_else(|| json!({}));
            let kind = s(t, "type").or_else(|| s(&d, "type")).unwrap_or("fade");
            if kind == "none" {
                p.insert("transition".into(), Value::Null);
            } else {
                let dur = f(t, "duration").map(ms).or_else(|| f(&d, "duration")).unwrap_or(400.0);
                let mut tr = json!({ "kind": kind, "durationMs": super::edit::jnum(dur) });
                if let Some(dir) = s(t, "direction") {
                    tr["direction"] = json!(dir);
                }
                p.insert("transition".into(), tr);
            }
        }
        None => {}
    }
    p
}

// ---------------------------------------------------------------- reading the timeline

/// An element's animation and visibility as assistants write them (seconds), for `get_design`.
pub fn element_view(el: &Value) -> Map<String, Value> {
    let mut o = Map::new();
    let a = &el["anim"];
    let out = &el["animOut"];
    let mut anim = Map::new();
    if a.is_object() {
        let preset = s(a, "preset").unwrap_or("");
        let dir = s(a, "direction");
        let name = match (preset, dir) {
            ("slide", Some(d)) => format!("slide-{d}"),
            ("slide", None) => "slide-left".into(),
            ("shrink", _) => "scale".into(),
            (p, _) => p.to_owned(),
        };
        let mut i = Map::new();
        i.insert("type".into(), json!(name));
        i.insert("delay".into(), secs(num(&a["delayMs"])));
        i.insert("duration".into(), secs(num(&a["durationMs"])));
        if let Some(e) = s(a, "easing") {
            i.insert("easing".into(), json!(e));
        }
        if let Some(d) = f(a, "distance") {
            i.insert("distance".into(), super::edit::jnum(d));
        }
        if let Some(d) = dir.filter(|_| preset != "slide") {
            i.insert("direction".into(), json!(d));
        }
        if let Some(by) = s(a, "by").filter(|b| *b != "element") {
            i.insert("by".into(), json!(if by == "char" { "letter" } else { by }));
        }
        if truthy(a.get("onClick")) {
            i.insert("onClick".into(), a["onClick"].clone());
        }
        anim.insert("in".into(), Value::Object(i));
    }
    if out.is_object() {
        let preset = s(out, "preset").unwrap_or("");
        let mut x = Map::new();
        x.insert("type".into(), json!(if preset == "shrink" { "scale" } else { preset }));
        x.insert("duration".into(), secs(num(&out["durationMs"])));
        if let Some(end) = f(&el["timing"], "endMs") {
            x.insert("at".into(), secs(end - num(&out["durationMs"])));
        }
        anim.insert("out".into(), Value::Object(x));
    }
    if !anim.is_empty() {
        o.insert("animation".into(), Value::Object(anim));
    }
    if let Some(st) = f(&el["timing"], "startMs").filter(|v| *v > 0.0) {
        o.insert("visibleFrom".into(), secs(st));
    }
    if let Some(e) = f(&el["timing"], "endMs") {
        o.insert("visibleTo".into(), secs(e));
    }
    if truthy(el.get("emphasis")) {
        o.insert("emphasis".into(), el["emphasis"]["preset"].clone());
    }
    o
}

/// A page's length and transition (seconds), for `get_design`.
pub fn page_view(p: &Value, length_ms: f64) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("duration".into(), secs(length_ms));
    if !p["durationMs"].is_number() {
        o.insert("durationAuto".into(), json!(true));
    }
    if let Some(t) = p.get("transition").filter(|t| t.is_object()) {
        let mut tr = json!({ "type": t["kind"], "duration": secs(num(&t["durationMs"])) });
        if let Some(d) = s(t, "direction") {
            tr["direction"] = json!(d);
        }
        o.insert("transition".into(), tr);
    }
    o
}

/// Whether a design uses time at all (page lengths, transitions, animations, sound, captions or video).
pub fn is_timed(doc: &Value) -> bool {
    if doc.pointer("/meta/surface").and_then(Value::as_str) == Some("video") {
        return true;
    }
    if doc["meta"]["audio"].as_array().is_some_and(|a| !a.is_empty()) || doc["meta"]["captions"]["cues"].as_array().is_some_and(|c| !c.is_empty()) {
        return true;
    }
    let pages = doc["pages"].as_object().into_iter().flatten();
    if pages.map(|(_, p)| p).any(|p| p["durationMs"].is_number() || p["transition"].is_object() || p["anim"].is_object()) {
        return true;
    }
    doc["elements"].as_object().into_iter().flatten().any(|(_, e)| e["deleted"] != true && (e["anim"].is_object() || e["animOut"].is_object() || e["timing"].is_object() || e["type"] == "video"))
}

/// The design's timeline (seconds): page spans, soundtrack and captions.
pub fn timeline_view(doc: &Value, typed: &oc_schema::DesignDoc) -> Value {
    let tl = oc_schema::timeline::timeline_of(typed, false);
    let order = super::view::sorted_page_ids(typed);
    let pages: Vec<Value> = tl
        .spans
        .iter()
        .map(|sp| {
            let n = order.iter().position(|p| *p == *sp.page.id).map_or(0, |i| i + 1);
            json!({ "page": n, "id": &*sp.page.id, "start": secs(sp.start_ms), "duration": secs(sp.duration_ms) })
        })
        .collect();
    let mut out = json!({ "total": secs(tl.total_ms), "pages": pages });
    let audio: Vec<Value> = doc["meta"]["audio"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let mut a = json!({
                "id": c["id"], "asset": c["asset"],
                "start": secs(num(&c["startMs"])), "duration": secs(num(&c["durationMs"])), "trim": secs(num(&c["trimStartMs"])),
                "volume": c["volume"], "fadeIn": secs(num(&c["fadeInMs"])), "fadeOut": secs(num(&c["fadeOutMs"])),
            });
            if truthy(c.get("name")) {
                a["name"] = c["name"].clone();
            }
            if c["loop"] == true {
                a["loop"] = json!(true);
            }
            a
        })
        .collect();
    if !audio.is_empty() {
        out["audio"] = Value::Array(audio);
    }
    let cap = &doc["meta"]["captions"];
    if let Some(cues) = cap["cues"].as_array().filter(|c| !c.is_empty()) {
        let st = &cap["style"];
        out["captions"] = json!({
            "visible": cap["visible"] != false,
            "style": { "font": st["family"], "size": st["size"], "color": st["color"], "background": st["background"], "position": st["position"] },
            "cues": cues.iter().map(|c| json!({ "start": secs(num(&c["startMs"])), "end": secs(num(&c["endMs"])), "text": c["text"] })).collect::<Vec<_>>(),
        });
    }
    out
}

// ---------------------------------------------------------------- video checks

/// Words a viewer reads per second in an ad, and the moment it takes to notice new text.
const WORDS_PER_SECOND: f64 = 3.0;
const NOTICE_S: f64 = 0.5;

/// The format a video is checked against, from its size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    /// 9:16: Reels and Stories.
    Vertical,
    /// 4:5 or 1:1: feed.
    Feed,
    Other,
}

pub fn format_of(w: f64, h: f64) -> Format {
    let r = w / h.max(1.0);
    if (r - 9.0 / 16.0).abs() < 0.02 {
        Format::Vertical
    } else if (r - 0.8).abs() < 0.02 || (r - 1.0).abs() < 0.02 {
        Format::Feed
    } else {
        Format::Other
    }
}

fn words(text: &str) -> usize {
    text.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

fn text_of(el: &Value) -> String {
    el["body"]["paragraphs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["runs"].as_array().into_iter().flatten().filter_map(|r| r["text"].as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn snippet(t: &str) -> String {
    let t = t.trim();
    if t.chars().count() > 40 { format!("{}…", t.chars().take(38).collect::<String>().trim_end()) } else { t.to_owned() }
}

/// When an element's entrance has finished, in page ms after its window opens (text by unit: every unit shown).
fn entrance_end(el: &Value) -> f64 {
    let a = &el["anim"];
    if !a.is_object() {
        return 0.0;
    }
    let delay = num(&a["delayMs"]);
    let by = s(a, "by").filter(|b| *b != "element").or(if s(a, "preset") == Some("typewriter") { Some("char") } else { None });
    let units = by.map(|b| {
        let t = text_of(el);
        match b {
            "line" => el["body"]["paragraphs"].as_array().map_or(1, Vec::len) as f64,
            "word" => t.split_whitespace().count() as f64,
            _ => t.chars().filter(|c| !c.is_whitespace()).count() as f64,
        }
    });
    match units {
        Some(n) if el["type"] == "text" => delay + n * f(a, "staggerMs").unwrap_or(40.0),
        _ => delay + num(&a["durationMs"]),
    }
}

fn issue(id: &str, problem: &str, message: String, page: Option<&str>) -> Value {
    let mut o = json!({ "id": id, "problem": problem, "message": message });
    if let Some(p) = page {
        o["page"] = json!(p);
    }
    o
}

/// Video checks of the given pages (all when `None`) and of the whole video: text shown too briefly to read, text in the
/// part of a 9:16 video Reels or Stories cover, elements timed outside their page, a video too long for its format, and
/// a soundtrack that stops before the end.
pub fn video_checks(doc: &Value, typed: &oc_schema::DesignDoc, pages: Option<&[String]>) -> Vec<Value> {
    if !is_timed(doc) {
        return Vec::new();
    }
    let tl = oc_schema::timeline::timeline_of(typed, false);
    let many = tl.spans.len() > 1;
    let (w, h) = (num(&doc["meta"]["dims"]["w"]), num(&doc["meta"]["dims"]["h"]));
    let format = format_of(w, h);
    let mut out = Vec::new();
    for sp in &tl.spans {
        let pid: &str = &sp.page.id;
        if pages.is_some_and(|ps| !ps.iter().any(|p| p == pid)) {
            continue;
        }
        let page = if many { Some(pid) } else { None };
        let len = sp.duration_ms;
        for id in oc_schema::tree::descendants(&typed.elements, &sp.page.id).iter().filter_map(|e| e.id()) {
            let el = &doc["elements"][id.to_string().as_str()];
            if el["deleted"] == true || el["visible"] == false {
                continue;
            }
            let start = f(&el["timing"], "startMs").unwrap_or(0.0);
            let end = f(&el["timing"], "endMs").unwrap_or(len).min(len);
            if start >= len {
                out.push(issue(id, "never_shown", format!("It appears at {} s but the page lasts {} s, so it is never seen; lower visibleFrom or lengthen the page.", secs(start), secs(len)), page));
                continue;
            }
            if f(&el["timing"], "endMs").is_some_and(|e| e > len + 1.0) {
                out.push(issue(id, "cut_by_page_end", format!("It should stay until {} s but the page ends at {} s, so its exit is cut off.", secs(num(&el["timing"]["endMs"])), secs(len)), page));
            }
            let role = s(el, "role");
            if el["type"] == "text" {
                let t = text_of(el);
                let n = words(&t);
                if n > 0 {
                    let readable = (end - start - entrance_end(el)) / 1000.0;
                    let need = NOTICE_S + n as f64 / WORDS_PER_SECOND;
                    if readable < need - 0.05 {
                        out.push(issue(
                            id,
                            "text_too_brief",
                            format!(
                                "\"{}\" ({n} word{}) can be read for {} s once it is in, but needs about {} s; give it more time (page duration, visibleTo, a shorter entrance) or fewer words.",
                                snippet(&t),
                                if n == 1 { "" } else { "s" },
                                secs(readable.max(0.0) * 1000.0),
                                secs(need * 1000.0)
                            ),
                            page,
                        ));
                    }
                }
            }
            // Reels covers the top 14% and the bottom 35% with its interface (Stories: 14% at each end).
            let key = el["type"] == "text" || el["type"] == "qr" || matches!(role, Some("cta" | "logo" | "price" | "headline" | "subhead" | "body" | "caption" | "legal" | "quote"));
            if format == Format::Vertical && key && !matches!(role, Some("background" | "decor" | "hero-image")) {
                let (_, y) = super::view::page_pos(doc, el).unwrap_or_else(|| {
                    let b = super::view::world_bounds(typed, id);
                    (b.0, b.1)
                });
                let bottom = y + num(&el["xf"]["h"]);
                let (top_safe, bottom_safe) = (h * 0.14, h * 0.65);
                if y < top_safe - 1.0 || bottom > bottom_safe + 1.0 {
                    let where_ = if y < top_safe - 1.0 { "top" } else { "bottom" };
                    out.push(issue(
                        id,
                        "outside_safe_zone",
                        format!(
                            "It reaches into the {where_} of the video (y {}–{}), which Reels covers with its interface: keep text, logos and buttons between y {} and {} (Stories: between {} and {}).",
                            js_round(y),
                            js_round(bottom),
                            js_round(top_safe),
                            js_round(bottom_safe),
                            js_round(top_safe),
                            js_round(h * 0.86)
                        ),
                        page,
                    ));
                }
            }
        }
    }
    // The whole video, whichever pages were edited.
    {
        let total = tl.total_ms / 1000.0;
        let limit = match format {
            Format::Vertical => Some((60.0, "Stories play at most 60 s per card, and most Reels ads run 15–30 s")),
            Format::Feed => Some((15.0, "Meta recommends feed video ads of 15 s or less")),
            Format::Other => None,
        };
        if let Some((max, why)) = limit
            && total > max + 0.01
        {
            out.push(issue("design", "video_too_long", format!("The video lasts {} s; {why}. Shorten pages or remove some.", secs(tl.total_ms)), None));
        }
        let audio_end = doc["meta"]["audio"].as_array().into_iter().flatten().map(|c| num(&c["startMs"]) + num(&c["durationMs"])).fold(None, |m: Option<f64>, e| Some(m.map_or(e, |m| m.max(e))));
        if let Some(end) = audio_end
            && end < tl.total_ms - 500.0
        {
            out.push(issue("audio", "audio_ends_early", format!("The soundtrack stops at {} s but the video runs to {} s; lengthen it (update_audio fit: true) or loop it.", secs(end), secs(tl.total_ms)), None));
        }
        let caps = &doc["meta"]["captions"];
        if caps["visible"] != false {
            for c in caps["cues"].as_array().into_iter().flatten() {
                let t = c["text"].as_str().unwrap_or("");
                let n = words(t);
                let shown = (num(&c["endMs"]) - num(&c["startMs"])) / 1000.0;
                let need = NOTICE_S + n as f64 / WORDS_PER_SECOND;
                if n > 0 && shown < need - 0.05 {
                    out.push(issue("captions", "caption_too_brief", format!("The caption \"{}\" shows for {} s but needs about {} s to read.", snippet(t), secs(shown * 1000.0), secs(need * 1000.0)), None));
                }
            }
        }
    }
    out
}

fn js_round(v: f64) -> Value {
    super::edit::jnum(v.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit(m: Value) -> Kit {
        Kit::new(Some("Mosaik".into()), m)
    }

    #[test]
    fn kit_motion_fills_what_the_animation_leaves_out() {
        let k = kit(json!({ "text": { "type": "rise", "distance": 8, "duration": 300, "easing": "ease-out" }, "draw": { "duration": 400 }, "enter": { "type": "fade", "duration": 150 }, "bounce": false }));
        let text = json!({ "type": "text" });
        let a = anim_in(&json!({ "delay": 0.2 }), &text, &k, 0.0).unwrap();
        assert_eq!(a, json!({ "preset": "rise", "durationMs": 300, "delayMs": 200, "easing": "ease-out", "distance": 8 }));
        // Lines draw along their direction, in the kit's draw time.
        let line = json!({ "type": "line", "points": [[0, 120], [0, 0]] });
        let a = anim_in(&json!({}), &line, &k, 0.0).unwrap();
        assert_eq!((a["preset"].clone(), a["direction"].clone(), a["durationMs"].clone()), (json!("draw"), json!("up"), json!(400)));
        // Shapes take the default entrance; asked values win.
        let a = anim_in(&json!({ "type": "slide-left", "duration": 0.25 }), &json!({ "type": "shape" }), &k, 0.1).unwrap();
        assert_eq!(a, json!({ "preset": "slide", "durationMs": 250, "delayMs": 100, "direction": "left" }));
        assert!(anim_in(&json!({ "type": "pop" }), &text, &k, 0.0).unwrap_err().contains("bounce: false"));
        assert!(anim_in(&json!({ "type": "typewriter" }), &json!({ "type": "shape" }), &Kit::default(), 0.0).is_err());
    }

    #[test]
    fn exits_and_windows_become_timing() {
        let el = json!({ "type": "text" });
        let p = element_patch(&json!({ "animation": { "in": { "type": "fade" }, "out": { "type": "fade", "at": 2.2 } }, "visibleFrom": 0.5 }), &el, &Kit::default(), 0.0).unwrap();
        assert_eq!(p["timing"], json!({ "startMs": 500, "endMs": 2600 }));
        assert_eq!(p["animOut"], json!({ "preset": "fade", "durationMs": 400 }));
        // visibleTo alone; null clears.
        let p = element_patch(&json!({ "visibleTo": 1.5 }), &el, &Kit::default(), 0.0).unwrap();
        assert_eq!(p["timing"], json!({ "startMs": 0, "endMs": 1500 }));
        let cur = json!({ "type": "text", "timing": { "startMs": 500, "endMs": 1500 } });
        let p = element_patch(&json!({ "visibleTo": null, "visibleFrom": null }), &cur, &Kit::default(), 0.0).unwrap();
        assert_eq!(p["timing"], Value::Null);
        assert!(element_patch(&json!({ "visibleFrom": 2, "visibleTo": 1 }), &el, &Kit::default(), 0.0).is_err());
        assert!(element_patch(&json!({ "visibleTo": 2, "animation": { "out": { "at": 1 } } }), &el, &Kit::default(), 0.0).is_err());
        let p = element_patch(&json!({ "animation": null }), &cur, &Kit::default(), 0.0).unwrap();
        assert_eq!((p["anim"].clone(), p["animOut"].clone(), p.contains_key("timing")), (Value::Null, Value::Null, false));
    }

    #[test]
    fn pages_take_lengths_and_transitions() {
        let k = kit(json!({ "transition": { "type": "fade", "duration": 150 } }));
        let p = page_patch(&json!({ "duration": 2.5, "transition": { "type": "slide", "direction": "left" } }), &k);
        assert_eq!(Value::Object(p), json!({ "durationMs": 2500, "transition": { "kind": "slide", "durationMs": 150, "direction": "left" } }));
        let p = page_patch(&json!({ "transition": { "type": "none" }, "duration": null }), &k);
        assert_eq!(Value::Object(p), json!({ "durationMs": null, "transition": null }));
    }

    #[test]
    fn element_view_reads_back_what_was_written() {
        let el = json!({ "type": "text", "anim": { "preset": "shrink", "durationMs": 400, "delayMs": 200, "easing": "ease-out" }, "animOut": { "preset": "fade", "durationMs": 300 }, "timing": { "startMs": 500, "endMs": 2600 } });
        assert_eq!(
            Value::Object(element_view(&el)),
            json!({ "animation": { "in": { "type": "scale", "delay": 0.2, "duration": 0.4, "easing": "ease-out" }, "out": { "type": "fade", "duration": 0.3, "at": 2.3 } }, "visibleFrom": 0.5, "visibleTo": 2.6 })
        );
    }

    #[test]
    fn formats_from_size() {
        assert_eq!(format_of(1080.0, 1920.0), Format::Vertical);
        assert_eq!(format_of(1080.0, 1350.0), Format::Feed);
        assert_eq!(format_of(1080.0, 1080.0), Format::Feed);
        assert_eq!(format_of(1920.0, 1080.0), Format::Other);
    }
}
