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

/// `animation.loop` → the element's `emphasis`: a small movement that repeats once the entrance has finished.
fn anim_loop(spec: &Value, kit: &Kit) -> Result<Value, String> {
    let kind = s(spec, "type").unwrap_or("pulse");
    if kit.no_bounce() && kind == "bounce" {
        return Err(format!("{} rules out bouncing animations (bounce: false); loop with pulse or float instead.", kit.rule()));
    }
    let period = f(spec, "period").map(ms).unwrap_or(if kind == "spin" { 3000.0 } else { 1600.0 });
    let mut e = json!({ "preset": kind, "periodMs": super::edit::jnum(period) });
    if let Some(k) = f(spec, "intensity") {
        e["intensity"] = super::edit::jnum(k);
    }
    Ok(e)
}

/// Whether an element holds a picture that can pan and zoom inside its box.
fn has_picture(el: &Value) -> bool {
    match el["type"].as_str() {
        Some("image" | "video") => true,
        Some("frame") => el["content"].is_object(),
        Some("shape") => el["fills"].as_array().is_some_and(|fs| fs.iter().any(|p| p["k"] == "image")),
        _ => false,
    }
}

/// Zoom of a `panZoom` that only pans: enough spare picture either side to travel across.
const PAN_ZOOM: f64 = 0.15;
/// How far a `panZoom` zooms when no amount is given: slow enough to read as a camera move, not an effect.
const ZOOM: f64 = 0.12;

/// `panZoom` → the element's `panZoom` (the picture moves inside its box for as long as the element shows).
pub fn pan_zoom(spec: &Value, el: &Value) -> Result<Value, String> {
    if !has_picture(el) {
        return Err("panZoom is for images, videos and frames or shapes holding a photo: it moves the picture inside its box. Animate other elements with animation.".into());
    }
    let kind = s(spec, "type").unwrap_or("zoom-in");
    let pan = kind.starts_with("pan-");
    let amount = f(spec, "amount").unwrap_or(if pan { PAN_ZOOM } else { ZOOM });
    let far = super::edit::jnum(((1.0 + amount) * 1000.0).round() / 1000.0);
    let (fx, fy) = (f(&spec["focus"], "x"), f(&spec["focus"], "y"));
    let mut p = Map::new();
    let mut set = |k: &str, v: Option<f64>| {
        if let Some(v) = v.filter(|v| (*v - 0.5).abs() > 1e-9) {
            p.insert(k.into(), super::edit::jnum(v));
        }
    };
    let (from, to) = match kind {
        "zoom-out" => (far, json!(1)),
        "zoom-in" => (json!(1), far),
        // A pan holds its zoom and carries the view across the picture, to the side it is named after.
        _ => (far.clone(), far),
    };
    match kind {
        "pan-left" => {
            set("fromX", Some(1.0));
            set("toX", Some(0.0));
            set("fromY", fy);
            set("toY", fy);
        }
        "pan-right" => {
            set("fromX", Some(0.0));
            set("toX", Some(1.0));
            set("fromY", fy);
            set("toY", fy);
        }
        "pan-up" => {
            set("fromY", Some(1.0));
            set("toY", Some(0.0));
            set("fromX", fx);
            set("toX", fx);
        }
        "pan-down" => {
            set("fromY", Some(0.0));
            set("toY", Some(1.0));
            set("fromX", fx);
            set("toX", fx);
        }
        _ => {
            set("fromX", fx);
            set("toX", fx);
            set("fromY", fy);
            set("toY", fy);
        }
    }
    p.insert("from".into(), from);
    p.insert("to".into(), to);
    if s(spec, "easing") == Some("ease") {
        p.insert("easing".into(), json!("ease"));
    }
    Ok(Value::Object(p))
}

/// A stored `panZoom` as assistants write it.
fn pan_zoom_view(pz: &Value) -> Value {
    let (from, to) = (num(&pz["from"]), num(&pz["to"]));
    let at = |k: &str| f(pz, k).unwrap_or(0.5);
    let (fx, tx, fy, ty) = (at("fromX"), at("toX"), at("fromY"), at("toY"));
    let amount = |v: f64| super::edit::jnum(((v - 1.0) * 1000.0).round() / 1000.0);
    let same = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let (kind, amt, focus) = if same(from, to) && same(fy, ty) && !same(fx, tx) {
        (if tx < fx { "pan-left" } else { "pan-right" }, amount(to), (0.5, fy))
    } else if same(from, to) && same(fx, tx) && !same(fy, ty) {
        (if ty < fy { "pan-up" } else { "pan-down" }, amount(to), (fx, 0.5))
    } else if to >= from {
        ("zoom-in", amount(to), (tx, ty))
    } else {
        ("zoom-out", amount(from), (fx, fy))
    };
    let mut o = json!({ "type": kind, "amount": amt });
    if !same(focus.0, 0.5) || !same(focus.1, 0.5) {
        o["focus"] = json!({ "x": super::edit::jnum(focus.0), "y": super::edit::jnum(focus.1) });
    }
    if s(pz, "easing") == Some("ease") {
        o["easing"] = json!("ease");
    }
    o
}

// ---------------------------------------------------------------- keyframes and motion paths
//
// Assistants write keyframes as absolute values in the units of `update` (page px for position, degrees for rotation,
// 0–1 for opacity). Designs store them as offsets from the element's resting place, measured in its own box: rotated
// with it and mirrored by its flips (`oc-render` draws the motion inside the element's transform). `Rest` converts.

/// Eases a key accepts by name. A key's ease shapes the stretch of time that arrives at it.
pub const KEY_EASES: [&str; 12] = ["linear", "ease", "ease-in", "ease-out", "ease-in-out", "back", "hold", "out-quart", "out-expo", "in-out-quart", "out-back", "anticipate"];

/// Named curves stored as a cubic bezier, with the built-in ease older builds fall back to (`None`: the default).
const BEZIER_EASES: [(&str, [f64; 4], Option<&str>); 5] = [
    ("out-quart", [0.25, 1.0, 0.5, 1.0], Some("ease-out")),
    ("out-expo", [0.16, 1.0, 0.3, 1.0], Some("ease-out")),
    ("in-out-quart", [0.76, 0.0, 0.24, 1.0], None),
    ("out-back", [0.34, 1.56, 0.64, 1.0], Some("back")),
    ("anticipate", [0.36, 0.0, 0.66, -0.56], Some("ease-in")),
];

/// How far a custom curve's y control points may reach (overshoot and anticipation, not wild swings).
pub const BEZIER_Y: (f64, f64) = (-2.0, 3.0);

/// How many keys a track takes.
pub const MAX_KEYS: usize = 64;

/// Where an element rests, which stored offsets are measured from: its page-space top-left (`None` inside a rotated
/// group, where page and box directions cannot be related here), its own rotation (degrees), flips and opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rest {
    pub page: Option<(f64, f64)>,
    pub rot: f64,
    pub flip_x: bool,
    pub flip_y: bool,
    pub opacity: f64,
}

impl Rest {
    /// The rest of a stored element (`xf`, `opacity`) at page position `page`.
    pub fn of(el: &Value, page: Option<(f64, f64)>) -> Rest {
        let xf = &el["xf"];
        Rest {
            page,
            rot: f(xf, "rot").unwrap_or(0.0),
            flip_x: xf["flipX"] == true,
            flip_y: xf["flipY"] == true,
            opacity: f(el, "opacity").unwrap_or(1.0),
        }
    }

    fn rotated(&self) -> bool {
        (self.rot % 360.0).abs() > 1e-9
    }

    /// -1 when exactly one flip mirrors the box (a keyed turn then reads the other way round on the page).
    fn mirror(&self) -> f64 {
        if self.flip_x != self.flip_y { -1.0 } else { 1.0 }
    }

    /// A page-space move → the stored offset (box space): turned back by the rotation, then mirrored by the flips.
    fn to_box(self, dx: f64, dy: f64) -> (f64, f64) {
        let r = -self.rot.to_radians();
        let (c, s) = (r.cos(), r.sin());
        let (x, y) = (dx * c - dy * s, dx * s + dy * c);
        (if self.flip_x { -x } else { x }, if self.flip_y { -y } else { y })
    }

    /// A stored offset (box space) → the page-space move.
    fn to_page(self, bx: f64, by: f64) -> (f64, f64) {
        let (x, y) = (if self.flip_x { -bx } else { bx }, if self.flip_y { -by } else { by });
        let r = self.rot.to_radians();
        let (c, s) = (r.cos(), r.sin());
        (x * c - y * s, x * s + y * c)
    }
}

/// Two decimals, as stored and shown.
fn r2(v: f64) -> Value {
    super::edit::jnum((v * 100.0).round() / 100.0)
}

/// Four decimals (opacity, scale).
fn r4(v: f64) -> Value {
    super::edit::jnum((v * 10000.0).round() / 10000.0)
}

/// A key's ease as stored: a built-in name ("ease", also "ease-in-out", is the default and is left out) and a
/// cubic bezier that wins over it where builds know it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Ease {
    name: Option<&'static str>,
    bezier: Option<[f64; 4]>,
}

fn stored_ease(e: &Value) -> Result<Ease, String> {
    if let Some(a) = e.as_array() {
        let b: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
        let b: [f64; 4] = b.try_into().map_err(|_| "ease: a curve is four numbers [x1, y1, x2, y2].".to_string())?;
        if !oc_schema::keyframes::bezier_ok(&b) {
            return Err(format!("ease [{}, {}, {}, {}]: x1 and x2 must be between 0 and 1 (as in CSS cubic-bezier).", b[0], b[1], b[2], b[3]));
        }
        if !(BEZIER_Y.0..=BEZIER_Y.1).contains(&b[1]) || !(BEZIER_Y.0..=BEZIER_Y.1).contains(&b[3]) {
            return Err(format!("ease [{}, {}, {}, {}]: y1 and y2 must be between {} and {}.", b[0], b[1], b[2], b[3], BEZIER_Y.0, BEZIER_Y.1));
        }
        // Older builds fall back to the built-in ease nearest in spirit: overshoot reads as back.
        let name = if b[1] > 1.0 || b[3] > 1.0 { Some("back") } else { None };
        return Ok(Ease { name, bezier: Some(b) });
    }
    let Some(n) = e.as_str() else { return Ok(Ease::default()) };
    if let Some((_, b, fallback)) = BEZIER_EASES.iter().find(|(k, _, _)| *k == n) {
        return Ok(Ease { name: *fallback, bezier: Some(*b) });
    }
    let name = match n {
        "linear" => Some("linear"),
        "ease-in" => Some("ease-in"),
        "ease-out" => Some("ease-out"),
        "back" => Some("back"),
        "hold" => Some("hold"),
        _ => None,
    };
    Ok(Ease { name, bezier: None })
}

/// A stored key's ease as assistants write it: a name where one fits, else the curve.
fn ease_view(k: &oc_schema::elements::Keyframe) -> Option<Value> {
    if let Some(b) = k.bezier.filter(oc_schema::keyframes::bezier_ok) {
        let near = |p: &[f64; 4]| p.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6);
        return Some(match BEZIER_EASES.iter().find(|(_, p, _)| near(p)) {
            Some((n, _, _)) => json!(n),
            None => json!(b.iter().map(|v| super::edit::jnum(*v)).collect::<Vec<_>>()),
        });
    }
    k.ease.as_ref().map(|e| e.as_str()).filter(|e| *e != "ease").map(|e| json!(e))
}

/// A scalar track: (assistant name, stored name, assistant value → stored value).
type ToStored<'a> = (&'static str, &'static str, &'a dyn Fn(f64) -> Result<Value, String>);
/// A scalar track: (assistant name, stored name, stored value → assistant value).
type FromStored<'a> = (&'static str, &'static str, &'a dyn Fn(f64) -> Value);

/// One assistant key: (page ms, value, ease).
struct Key<'a> {
    t: f64,
    v: &'a Value,
    ease: Ease,
}

fn keys_of<'a>(name: &str, spec: &'a Value) -> Result<Vec<Key<'a>>, String> {
    let items = spec.as_array().map(Vec::as_slice).unwrap_or(&[]);
    if items.len() > MAX_KEYS {
        return Err(format!("keyframes.{name} has {} keys; at most {MAX_KEYS}.", items.len()));
    }
    let mut out: Vec<Key<'a>> = Vec::with_capacity(items.len());
    for k in items {
        let t = ms(num(&k["at"]));
        if let Some(prev) = out.last()
            && t <= prev.t
        {
            return Err(format!("keyframes.{name}: keys must be in time order, each later than the one before ({} s after {} s).", secs(t), secs(prev.t)));
        }
        let ease = stored_ease(&k["ease"]).map_err(|e| format!("keyframes.{name} at {} s: {e}", secs(t)))?;
        out.push(Key { t, v: &k["value"], ease });
    }
    Ok(out)
}

fn stored_key(t: f64, v: Value, ease: Ease) -> Value {
    let mut k = json!({ "t": super::edit::jnum(t), "v": v });
    if let Some(e) = ease.name {
        k["ease"] = json!(e);
    }
    if let Some(b) = ease.bezier {
        k["bezier"] = json!(b.iter().map(|v| super::edit::jnum(*v)).collect::<Vec<_>>());
    }
    k
}

fn pair(v: &Value) -> (f64, f64) {
    (num(&v[0]), num(&v[1]))
}

/// `keyframes` of an operation → the element's stored `keyframes` (`Null`: none left). Tracks not named keep their
/// keys; a track given replaces the stored one, `null` removes it. `rest` is where the element rests once the
/// operation applies.
pub fn keyframes_patch(spec: &Value, el: &Value, rest: &Rest) -> Result<Value, String> {
    if spec.is_null() {
        return Ok(Value::Null);
    }
    let mut out = match &el["keyframes"] {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    let given = |k: &str| spec.get(k).is_some();
    if given("position") as u8 + given("offset") as u8 > 1 || ((given("position") || given("offset")) && (given("x") || given("y"))) {
        return Err("keyframes: give position, offset, or x and y, not more than one of them; they move the same thing.".into());
    }
    let needs_page = ["position", "offset", "x", "y"].iter().any(|k| given(k));
    let page = match rest.page {
        Some(p) => p,
        None if needs_page => return Err("keyframes: moving an element inside a rotated group is not supported; animate the group, or move the element out of it.".into()),
        None => (0.0, 0.0),
    };
    let mut set = |key: &str, track: Option<Vec<Value>>| match track {
        Some(t) if !t.is_empty() => {
            out.insert(key.into(), Value::Array(t));
        }
        _ => {
            out.remove(key);
        }
    };
    // Position: page-space top-left (`position`, `x`, `y`) or a move from the resting place (`offset`).
    for (name, from) in [("position", Some(page)), ("offset", None)] {
        match spec.get(name) {
            None => {}
            Some(Value::Null) => {
                set("x", None);
                set("y", None);
            }
            Some(track) => {
                let keys = keys_of(name, track)?;
                let (mut xs, mut ys) = (Vec::new(), Vec::new());
                for k in &keys {
                    let (vx, vy) = pair(k.v);
                    let (ox, oy) = from.unwrap_or((0.0, 0.0));
                    let (bx, by) = rest.to_box(vx - ox, vy - oy);
                    xs.push(stored_key(k.t, r2(bx), k.ease));
                    ys.push(stored_key(k.t, r2(by), k.ease));
                }
                set("x", Some(xs));
                set("y", Some(ys));
            }
        }
    }
    for (name, origin, flip) in [("x", page.0, rest.flip_x), ("y", page.1, rest.flip_y)] {
        match spec.get(name) {
            None => {}
            Some(Value::Null) => set(name, None),
            Some(track) => {
                if rest.rotated() {
                    return Err(format!("keyframes.{name}: the element is rotated, so moving it along the page moves both of its own axes; key position (or offset) instead."));
                }
                let keys = keys_of(name, track)?;
                let sign = if flip { -1.0 } else { 1.0 };
                set(name, Some(keys.iter().map(|k| stored_key(k.t, r2(sign * (num(k.v) - origin)), k.ease)).collect()));
            }
        }
    }
    let rot_sign = rest.mirror();
    let opacity = rest.opacity;
    let scalar: [ToStored; 4] = [
        ("scale", "scale", &|v| Ok(r4(v))),
        ("rotation", "rotate", &|v| Ok(r2(rot_sign * (v - rest.rot)))),
        (
            "opacity",
            "opacity",
            &|v| {
                if opacity <= 0.0 {
                    return Err("keyframes.opacity: the element's own opacity is 0, so nothing would show; set opacity on the element first (update opacity: 1).".into());
                }
                if v > opacity + 1e-9 {
                    return Err(format!("keyframes.opacity: a key of {v} is above the element's own opacity ({opacity}), which keys can only lower; set opacity: 1 in the same update and key the fade."));
                }
                Ok(r4(v / opacity))
            },
        ),
        ("blur", "blur", &|v| Ok(r2(v))),
    ];
    for (name, key, conv) in scalar {
        match spec.get(name) {
            None => {}
            Some(Value::Null) => set(key, None),
            Some(track) => {
                let keys = keys_of(name, track)?;
                let mut t = Vec::with_capacity(keys.len());
                for k in &keys {
                    t.push(stored_key(k.t, conv(num(k.v))?, k.ease));
                }
                set(key, Some(t));
            }
        }
    }
    Ok(if out.is_empty() { Value::Null } else { Value::Object(out) })
}

/// A stored track, or none.
fn track(k: &Value, name: &str) -> Option<Vec<oc_schema::elements::Keyframe>> {
    serde_json::from_value::<Vec<oc_schema::elements::Keyframe>>(k.get(name)?.clone()).ok().filter(|t| !t.is_empty())
}

fn key_view(t: f64, value: Value, ease: Option<Value>) -> Value {
    let mut k = json!({ "at": secs(t), "value": value });
    if let Some(e) = ease {
        k["ease"] = e;
    }
    k
}

/// Stored keyframes as assistants write them (absolute values, seconds).
pub fn keyframes_view(el: &Value, rest: &Rest) -> Option<Value> {
    use oc_schema::keyframes::keyframe_value;
    let k = el.get("keyframes").filter(|k| k.is_object())?;
    let mut o = Map::new();
    let (xs, ys) = (track(k, "x"), track(k, "y"));
    let (px, py) = rest.page.unwrap_or((0.0, 0.0));
    let same_times = |a: &[oc_schema::elements::Keyframe], b: &[oc_schema::elements::Keyframe]| a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.t - q.t).abs() < 0.5);
    let positional = |times: Vec<(f64, Option<Value>)>, x: &dyn Fn(f64) -> f64, y: &dyn Fn(f64) -> f64| -> Value {
        Value::Array(
            times
                .into_iter()
                .map(|(t, e)| {
                    let (dx, dy) = rest.to_page(x(t), y(t));
                    // Inside a rotated group the page position is unknown: show the move instead.
                    let v = if rest.page.is_some() { json!([r2(px + dx), r2(py + dy)]) } else { json!([r2(dx), r2(dy)]) };
                    key_view(t, v, e)
                })
                .collect(),
        )
    };
    let name = if rest.page.is_some() { "position" } else { "offset" };
    match (&xs, &ys) {
        (Some(a), Some(b)) if same_times(a, b) => {
            let times = a.iter().map(|k| (k.t, ease_view(k))).collect();
            o.insert(name.into(), positional(times, &|t| keyframe_value(Some(a), t, 0.0), &|t| keyframe_value(Some(b), t, 0.0)));
        }
        (a, b) if rest.page.is_some() && !rest.rotated() => {
            // Unrotated: each axis reads on its own.
            for (n, tr, origin, flip) in [("x", a, px, rest.flip_x), ("y", b, py, rest.flip_y)] {
                if let Some(tr) = tr {
                    let sign = if flip { -1.0 } else { 1.0 };
                    o.insert(n.into(), Value::Array(tr.iter().map(|k| key_view(k.t, r2(origin + sign * k.v), ease_view(k))).collect()));
                }
            }
        }
        (a, b) if a.is_some() || b.is_some() => {
            // Rotated with keys at different times on each axis: both axes at every key time.
            let mut times: Vec<(f64, Option<Value>)> = a.iter().chain(b.iter()).flatten().map(|k| (k.t, ease_view(k))).collect();
            times.sort_by(|p, q| p.0.total_cmp(&q.0));
            times.dedup_by(|p, q| (p.0 - q.0).abs() < 0.5);
            o.insert(name.into(), positional(times, &|t| keyframe_value(a.as_deref(), t, 0.0), &|t| keyframe_value(b.as_deref(), t, 0.0)));
        }
        _ => {}
    }
    let scalar: [FromStored; 4] = [
        ("scale", "scale", &r4),
        ("rotation", "rotate", &|v| r2(rest.rot + rest.mirror() * v)),
        ("opacity", "opacity", &|v| r4(v * rest.opacity)),
        ("blur", "blur", &r2),
    ];
    for (name, key, conv) in scalar {
        if let Some(tr) = track(k, key) {
            o.insert(name.into(), Value::Array(tr.iter().map(|k| key_view(k.t, conv(k.v), ease_view(k))).collect()));
        }
    }
    if o.is_empty() { None } else { Some(Value::Object(o)) }
}

/// Points per segment of a smooth path (Catmull-Rom), and the most a path stores.
const SMOOTH_STEPS: usize = 8;
const MAX_PATH_POINTS: usize = 256;

/// Through-points → a polyline through them, curved (Catmull-Rom) when `smooth`.
fn path_points(via: &[(f64, f64)], smooth: bool) -> Vec<(f64, f64)> {
    if !smooth || via.len() < 3 {
        return via.to_vec();
    }
    let segs = via.len() - 1;
    let steps = (MAX_PATH_POINTS.saturating_sub(1) / segs).clamp(2, SMOOTH_STEPS);
    let at = |i: isize| via[i.clamp(0, via.len() as isize - 1) as usize];
    let mut out = Vec::with_capacity(segs * steps + 1);
    for i in 0..segs as isize {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        for s in 0..steps {
            let t = s as f64 / steps as f64;
            let (t2, t3) = (t * t, t * t * t);
            let c = |a: f64, b: f64, c: f64, d: f64| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
            out.push((c(p0.0, p1.0, p2.0, p3.0), c(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out.push(via[via.len() - 1]);
    out
}

/// `motionPath` → the element's stored `motionPath`: the box's top-left travels through `via` (page px) over
/// `duration` seconds, starting `delay` seconds after the element appears.
pub fn motion_path_patch(spec: &Value, rest: &Rest) -> Result<Value, String> {
    if spec.is_null() {
        return Ok(Value::Null);
    }
    let Some(page) = rest.page else {
        return Err("motionPath: moving an element inside a rotated group is not supported; animate the group, or move the element out of it.".into());
    };
    let via: Vec<(f64, f64)> = spec["via"].as_array().into_iter().flatten().map(pair).collect();
    let mut points = Vec::new();
    for (x, y) in path_points(&via, spec["smooth"] == true) {
        let (bx, by) = rest.to_box(x - page.0, y - page.1);
        points.push(r2(bx));
        points.push(r2(by));
    }
    Ok(json!({
        "points": points,
        "durationMs": super::edit::jnum(ms(num(&spec["duration"]))),
        "delayMs": super::edit::jnum(ms(f(spec, "delay").unwrap_or(0.0))),
        "easing": if s(spec, "easing") == Some("ease") { "ease" } else { "linear" },
    }))
}

/// A stored motion path as assistants write it.
fn motion_path_view(mp: &Value, rest: &Rest) -> Value {
    let (px, py) = rest.page.unwrap_or((0.0, 0.0));
    let pts: Vec<f64> = mp["points"].as_array().into_iter().flatten().map(num).collect();
    let via: Vec<Value> = (0..pts.len() / 2)
        .map(|i| {
            let (dx, dy) = rest.to_page(pts[2 * i], pts[2 * i + 1]);
            json!([r2(px + dx), r2(py + dy)])
        })
        .collect();
    let mut o = json!({ "via": via, "duration": secs(num(&mp["durationMs"])) });
    if let Some(d) = f(mp, "delayMs").filter(|d| *d > 0.0) {
        o["delay"] = secs(d);
    }
    if s(mp, "easing") == Some("ease") {
        o["easing"] = json!("ease");
    }
    o
}

/// The time fields of an `add_*`, `update` or `animate` operation as an element patch (`anim`, `animOut`, `emphasis`,
/// `panZoom`, `timing`, `keyframes`, `motionPath`; `null` removes). `el` is the element as stored (or about to be added);
/// `rest` where it rests once the operation applies (needed for keyframes and motion paths); `extra_delay_s` staggers
/// `animate`.
pub fn element_patch(op: &Value, el: &Value, kit: &Kit, extra_delay_s: f64, rest: Option<&Rest>) -> Result<Map<String, Value>, String> {
    let mut p = Map::new();
    let fallback = Rest::of(el, None);
    let rest = rest.unwrap_or(&fallback);
    if let Some(spec) = op.get("keyframes") {
        p.insert("keyframes".into(), keyframes_patch(spec, el, rest)?);
    }
    if let Some(spec) = op.get("motionPath") {
        p.insert("motionPath".into(), motion_path_patch(spec, rest)?);
    }
    let anim = op.get("animation");
    let (spec_in, spec_out, spec_loop) = match anim {
        Some(Value::Null) => (Some(&Value::Null), Some(&Value::Null), Some(&Value::Null)),
        Some(a) => (a.get("in"), a.get("out"), a.get("loop")),
        None => (op.get("in"), op.get("out"), None),
    };
    match spec_loop {
        Some(Value::Null) => {
            p.insert("emphasis".into(), Value::Null);
        }
        Some(spec) => {
            p.insert("emphasis".into(), anim_loop(spec, kit)?);
        }
        None => {}
    }
    match op.get("panZoom") {
        Some(Value::Null) => {
            p.insert("panZoom".into(), Value::Null);
        }
        Some(spec) => {
            p.insert("panZoom".into(), pan_zoom(spec, el)?);
        }
        None => {}
    }
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
    ["animation", "panZoom", "visibleFrom", "visibleTo", "keyframes", "motionPath"].iter().any(|k| op.get(*k).is_some())
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
    match op.get("entrance") {
        Some(Value::Null) => {
            p.insert("anim".into(), Value::Null);
        }
        Some(e) => {
            let preset = match s(e, "type").unwrap_or("fade") {
                "scale" => "shrink",
                k => k,
            };
            p.insert("anim".into(), json!({ "preset": preset, "durationMs": super::edit::jnum(f(e, "duration").map_or(500.0, ms)) }));
        }
        None => {}
    }
    p
}

/// Entrances a whole page can give its elements (none that need a direction).
pub const PAGE_ENTRANCES: [&str; 8] = ["fade", "rise", "drop", "scale", "zoom", "pop", "wipe", "blur"];

// ---------------------------------------------------------------- reading the timeline

/// An element's animation and visibility as assistants write them (seconds), for `get_design`. `rest` is where it
/// rests, which keyframes and motion paths are measured from.
pub fn element_view(el: &Value, rest: &Rest) -> Map<String, Value> {
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
    if let Some(em) = el.get("emphasis").filter(|e| e.is_object()) {
        let mut l = json!({ "type": em["preset"], "period": secs(num(&em["periodMs"])) });
        if let Some(k) = f(em, "intensity") {
            l["intensity"] = super::edit::jnum(k);
        }
        anim.insert("loop".into(), l);
    }
    if !anim.is_empty() {
        o.insert("animation".into(), Value::Object(anim));
    }
    if let Some(pz) = el.get("panZoom").filter(|p| p.is_object()) {
        o.insert("panZoom".into(), pan_zoom_view(pz));
    }
    if let Some(k) = keyframes_view(el, rest) {
        o.insert("keyframes".into(), k);
    }
    if let Some(mp) = el.get("motionPath").filter(|p| p["points"].as_array().is_some_and(|a| a.len() >= 4)) {
        o.insert("motionPath".into(), motion_path_view(mp, rest));
    }
    if let Some(m) = el.get("mask").filter(|m| m["id"].is_string()) {
        o.insert("mask".into(), json!({ "id": m["id"], "mode": m.get("mode").cloned().unwrap_or(json!("alpha")) }));
    }
    if let Some(st) = f(&el["timing"], "startMs").filter(|v| *v > 0.0) {
        o.insert("visibleFrom".into(), secs(st));
    }
    if let Some(e) = f(&el["timing"], "endMs") {
        o.insert("visibleTo".into(), secs(e));
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
    if let Some(a) = p.get("anim").filter(|a| a.is_object()) {
        let preset = s(a, "preset").unwrap_or("fade");
        o.insert("entrance".into(), json!({ "type": if preset == "shrink" { "scale" } else { preset }, "duration": secs(num(&a["durationMs"])) }));
    }
    o
}

/// Element fields that make it move or come and go over time.
const MOTION_FIELDS: [&str; 7] = ["anim", "animOut", "timing", "panZoom", "keyframes", "motionPath", "emphasis"];

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
    doc["elements"].as_object().into_iter().flatten().any(|(_, e)| e["deleted"] != true && (MOTION_FIELDS.iter().any(|k| e[*k].is_object()) || e["type"] == "video"))
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
            "style": caption_style_view(st),
            "cues": cues.iter().map(|c| json!({ "start": secs(num(&c["startMs"])), "end": secs(num(&c["endMs"])), "text": c["text"] })).collect::<Vec<_>>(),
        });
    }
    out
}

fn caption_style_view(st: &Value) -> Value {
    let mut o = json!({ "font": st["family"], "size": st["size"], "color": st["color"], "background": st["background"], "position": st["position"] });
    if st["highlight"].is_string() {
        o["highlight"] = st["highlight"].clone();
    }
    o
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
            keyframe_checks(doc, el, id, (start, end, len), (w, h), page, &mut out);
            let role = s(el, "role");
            if el["type"] == "text" {
                let t = text_of(el);
                let n = words(&t);
                if n > 0 {
                    let faded = faded_ms(el, start + entrance_end(el), end);
                    let readable = (end - start - entrance_end(el) - faded) / 1000.0;
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

/// The stored keyframe tracks of an element, by stored name.
fn stored_tracks(el: &Value) -> Vec<(&'static str, Vec<oc_schema::elements::Keyframe>)> {
    let k = &el["keyframes"];
    oc_schema::elements::KEYFRAME_PROPS.iter().filter_map(|p| track(k, p).map(|t| (*p, t))).collect()
}

/// How an assistant names a stored track.
fn track_name(stored: &str) -> &'static str {
    match stored {
        "x" | "y" => "position",
        "rotate" => "rotation",
        "scale" => "scale",
        "opacity" => "opacity",
        _ => "blur",
    }
}

/// Stored tracks an entrance preset also drives.
fn entrance_tracks(preset: &str) -> &'static [&'static str] {
    match preset {
        "fade" | "flip" => &["opacity"],
        "rise" | "drop" | "pan" | "float" | "bounce" | "slide" => &["opacity", "x", "y"],
        "pop" | "zoom" | "shrink" | "stomp" | "breathe" => &["opacity", "scale"],
        "spin" | "tumble" => &["opacity", "rotate", "scale"],
        "blur" => &["opacity", "blur"],
        _ => &[],
    }
}

/// How far from neutral a keyed value must be to count as moving.
fn off_neutral(stored: &str, v: f64) -> bool {
    match stored {
        "scale" => (v - 1.0).abs() > 0.005,
        "opacity" => (v - 1.0).abs() > 0.01,
        _ => v.abs() > 0.5,
    }
}

/// How long (ms) keyed opacity holds text below half between `from` and `to` (page ms): too faint to read.
fn faded_ms(el: &Value, from: f64, to: f64) -> f64 {
    let Some(op) = track(&el["keyframes"], "opacity") else { return 0.0 };
    let own = f(el, "opacity").unwrap_or(1.0);
    let step = 50.0;
    let mut t = from;
    let mut n = 0.0;
    while t < to {
        if oc_schema::keyframes::keyframe_value(Some(&op), t, 1.0).clamp(0.0, 1.0) * own < 0.5 {
            n += step;
        }
        t += step;
    }
    n
}

/// Keyframe checks of one top-level element: keys after the page ends or outside the element's window, keys closer
/// than a frame, keyframes fighting its entrance, and keys that keep it off the page.
fn keyframe_checks(doc: &Value, el: &Value, id: &str, (start, end, len): (f64, f64, f64), (pw, ph): (f64, f64), page: Option<&str>, out: &mut Vec<Value>) {
    use oc_schema::keyframes::keyframe_value;
    let tracks = stored_tracks(el);
    if tracks.is_empty() {
        return;
    }
    let mut times: Vec<f64> = tracks.iter().flat_map(|(_, t)| t.iter().map(|k| k.t)).collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    let last = *times.last().unwrap_or(&0.0);
    if last > len + 1.0 {
        out.push(issue(
            id,
            "keyframe_after_page_end",
            format!("Its last keyframe is at {} s but the page lasts {} s, so what comes after the end never plays; move the keys earlier or lengthen the page.", secs(last), secs(len)),
            page,
        ));
    } else {
        let outside: Vec<String> = times.iter().filter(|t| **t < start - 1.0 || **t > end + 1.0).map(|t| secs(*t).to_string()).collect();
        if !outside.is_empty() {
            out.push(issue(
                id,
                "keyframe_outside_window",
                format!(
                    "It shows from {} to {} s but has keyframes at {} s; keys are page seconds and only play while the element shows, so move them inside its time or change visibleFrom / visibleTo.",
                    secs(start),
                    secs(end),
                    outside.join(", ")
                ),
                page,
            ));
        }
    }
    if let Some((name, a, b)) = tracks.iter().find_map(|(n, t)| t.windows(2).find(|w| w[1].t - w[0].t < 1000.0 / 30.0).map(|w| (*n, w[0].t, w[1].t))) {
        out.push(issue(
            id,
            "keys_closer_than_a_frame",
            format!(
                "Its {} keys at {} and {} s are less than a frame apart (30 frames a second), so the change between them is lost; space them out, or for a jump use ease \"hold\" on the later key.",
                track_name(name),
                secs(a),
                secs(b)
            ),
            page,
        ));
    }
    if let Some(a) = el.get("anim").filter(|a| a.is_object()) {
        let preset = s(a, "preset").unwrap_or("");
        let (a0, a1) = (start + num(&a["delayMs"]), start + num(&a["delayMs"]) + num(&a["durationMs"]));
        let hit: Vec<&str> = tracks
            .iter()
            .filter(|(n, _)| entrance_tracks(preset).contains(n))
            .filter(|(n, t)| (0..=4).any(|i| off_neutral(n, keyframe_value(Some(t), a0 + (a1 - a0) * i as f64 / 4.0, 0.0))))
            .map(|(n, _)| track_name(n))
            .fold(Vec::new(), |mut v, n| {
                if !v.contains(&n) {
                    v.push(n);
                }
                v
            });
        if !hit.is_empty() {
            let shown = if preset == "shrink" { "scale" } else { preset };
            out.push(issue(
                id,
                "entrance_conflict",
                format!(
                    "Its entrance ({shown}) and its {} keyframes both act at {}–{} s, and they combine (moves add up, opacity and scale multiply). Remove the entrance (animation: {{ in: null }}) and key the whole move, or start the keyframes after the entrance.",
                    hit.join(" and "),
                    secs(a0),
                    secs(a1)
                ),
                page,
            ));
        }
    }
    // Keys that hold it entirely off one side of the page.
    let (xs, ys) = (tracks.iter().find(|(n, _)| *n == "x").map(|(_, t)| t), tracks.iter().find(|(n, _)| *n == "y").map(|(_, t)| t));
    if (xs.is_some() || ys.is_some())
        && let Some((px, py)) = super::view::rest_point(doc, el)
    {
        let rest = Rest::of(el, Some((px, py)));
        let (w, h) = (num(&el["xf"]["w"]), num(&el["xf"]["h"]));
        let side = |t: f64| -> Option<&'static str> {
            let (dx, dy) = rest.to_page(keyframe_value(xs.map(Vec::as_slice), t, 0.0), keyframe_value(ys.map(Vec::as_slice), t, 0.0));
            let (x, y) = (px + dx, py + dy);
            if x + w <= 0.0 {
                Some("left")
            } else if x >= pw {
                Some("right")
            } else if y + h <= 0.0 {
                Some("top")
            } else if y >= ph {
                Some("bottom")
            } else {
                None
            }
        };
        let sides: Vec<Option<&str>> = times.iter().map(|t| side(*t)).collect();
        if let Some(Some(first)) = sides.first()
            && sides.iter().all(|s| *s == Some(*first))
        {
            out.push(issue(id, "never_on_screen", format!("Its keyframes keep it off the {first} of the page the whole time, so it is never seen."), page));
        }
    }
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
        let p = element_patch(&json!({ "animation": { "in": { "type": "fade" }, "out": { "type": "fade", "at": 2.2 } }, "visibleFrom": 0.5 }), &el, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["timing"], json!({ "startMs": 500, "endMs": 2600 }));
        assert_eq!(p["animOut"], json!({ "preset": "fade", "durationMs": 400 }));
        // visibleTo alone; null clears.
        let p = element_patch(&json!({ "visibleTo": 1.5 }), &el, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["timing"], json!({ "startMs": 0, "endMs": 1500 }));
        let cur = json!({ "type": "text", "timing": { "startMs": 500, "endMs": 1500 } });
        let p = element_patch(&json!({ "visibleTo": null, "visibleFrom": null }), &cur, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["timing"], Value::Null);
        assert!(element_patch(&json!({ "visibleFrom": 2, "visibleTo": 1 }), &el, &Kit::default(), 0.0, None).is_err());
        assert!(element_patch(&json!({ "visibleTo": 2, "animation": { "out": { "at": 1 } } }), &el, &Kit::default(), 0.0, None).is_err());
        let p = element_patch(&json!({ "animation": null }), &cur, &Kit::default(), 0.0, None).unwrap();
        assert_eq!((p["anim"].clone(), p["animOut"].clone(), p.contains_key("timing")), (Value::Null, Value::Null, false));
    }

    #[test]
    fn pictures_pan_and_zoom_and_elements_loop() {
        let img = json!({ "type": "image" });
        let p = element_patch(&json!({ "panZoom": {} }), &img, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["panZoom"], json!({ "from": 1, "to": 1.12 }));
        let p = element_patch(&json!({ "panZoom": { "type": "zoom-out", "amount": 0.2, "focus": { "x": 0.3, "y": 0.5 } } }), &img, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["panZoom"], json!({ "fromX": 0.3, "toX": 0.3, "from": 1.2, "to": 1 }));
        let p = element_patch(&json!({ "panZoom": { "type": "pan-left", "easing": "ease" } }), &json!({ "type": "video" }), &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["panZoom"], json!({ "fromX": 1, "toX": 0, "from": 1.15, "to": 1.15, "easing": "ease" }));
        // Only elements holding a picture; null removes.
        assert!(element_patch(&json!({ "panZoom": {} }), &json!({ "type": "text" }), &Kit::default(), 0.0, None).unwrap_err().contains("images, videos"));
        assert!(element_patch(&json!({ "panZoom": {} }), &json!({ "type": "frame" }), &Kit::default(), 0.0, None).is_err());
        assert!(element_patch(&json!({ "panZoom": {} }), &json!({ "type": "frame", "content": { "asset": "a" } }), &Kit::default(), 0.0, None).is_ok());
        assert!(element_patch(&json!({ "panZoom": {} }), &json!({ "type": "shape", "fills": [{ "k": "image", "asset": "a" }] }), &Kit::default(), 0.0, None).is_ok());
        assert_eq!(element_patch(&json!({ "panZoom": null }), &img, &Kit::default(), 0.0, None).unwrap()["panZoom"], Value::Null);
        // Loops become the emphasis; animation null clears it with the rest.
        let p = element_patch(&json!({ "animation": { "loop": { "type": "float", "period": 2.4, "intensity": 0.5 } } }), &img, &Kit::default(), 0.0, None).unwrap();
        assert_eq!(p["emphasis"], json!({ "preset": "float", "periodMs": 2400, "intensity": 0.5 }));
        assert!(!p.contains_key("anim"));
        assert_eq!(element_patch(&json!({ "animation": null }), &img, &Kit::default(), 0.0, None).unwrap()["emphasis"], Value::Null);
        assert!(element_patch(&json!({ "animation": { "loop": { "type": "bounce" } } }), &img, &kit(json!({ "bounce": false })), 0.0, None).is_err());
        // And read back as written.
        for spec in [json!({ "type": "zoom-in", "amount": 0.12 }), json!({ "type": "zoom-out", "amount": 0.2, "focus": { "x": 0.3, "y": 0.5 } }), json!({ "type": "pan-left", "amount": 0.15, "easing": "ease" }), json!({ "type": "pan-down", "amount": 0.15, "focus": { "x": 0.2, "y": 0.5 } })] {
            let stored = pan_zoom(&spec, &img).unwrap();
            assert_eq!(pan_zoom_view(&stored), spec);
        }
        let el = json!({ "type": "image", "emphasis": { "preset": "pulse", "periodMs": 1600 }, "panZoom": { "from": 1, "to": 1.12 } });
        assert_eq!(Value::Object(element_view(&el, &Rest::of(&el, None))), json!({ "animation": { "loop": { "type": "pulse", "period": 1.6 } }, "panZoom": { "type": "zoom-in", "amount": 0.12 } }));
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
            Value::Object(element_view(&el, &Rest::of(&el, None))),
            json!({ "animation": { "in": { "type": "scale", "delay": 0.2, "duration": 0.4, "easing": "ease-out" }, "out": { "type": "fade", "duration": 0.3, "at": 2.3 } }, "visibleFrom": 0.5, "visibleTo": 2.6 })
        );
    }

    /// What the renderer draws for a stored move and turn: `local_matrix(xf)` then the motion matrix about the centre
    /// (`oc-render` `Resolver::emit`), as a page-space centre and the page direction of the box's x axis.
    fn drawn(xf: &oc_schema::Xf, dx: f64, dy: f64, rot: f64) -> ([f64; 2], f64) {
        use oc_geometry::matrix::{apply, apply_vector, local_matrix, multiply, rotate, translate};
        let (cx, cy) = (xf.w / 2.0, xf.h / 2.0);
        let motion = multiply(&translate(cx + dx, cy + dy), &multiply(&rotate(rot), &translate(-cx, -cy)));
        let m = multiply(&local_matrix(xf), &motion);
        let axis = apply_vector(&m, [1.0, 0.0]);
        (apply(&m, [cx, cy]), axis[1].atan2(axis[0]).to_degrees())
    }

    #[test]
    fn stored_moves_and_turns_match_the_renderer_for_rotated_and_flipped_elements() {
        for rot in [0.0, 30.0, 90.0, -135.0] {
            for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                let xf = oc_schema::Xf { flip_x: Some(fx), flip_y: Some(fy), ..oc_schema::Xf::new(100.0, 200.0, 80.0, 40.0, rot) };
                let rest = Rest { page: Some((0.0, 0.0)), rot, flip_x: fx, flip_y: fy, opacity: 1.0 };
                // A page move of (30, -20) is drawn as exactly that move.
                let (bx, by) = rest.to_box(30.0, -20.0);
                let (still, _) = drawn(&xf, 0.0, 0.0, 0.0);
                let (moved, _) = drawn(&xf, bx, by, 0.0);
                assert!((moved[0] - still[0] - 30.0).abs() < 1e-9 && (moved[1] - still[1] + 20.0).abs() < 1e-9, "rot {rot} flips {fx} {fy}");
                assert_eq!(rest.to_page(bx, by).0.round(), 30.0);
                // A key of rotation 50 draws the box as an element rotated 50 with the same flips.
                let stored = rest.mirror() * (50.0 - rot);
                let (_, turned) = drawn(&xf, 0.0, 0.0, stored);
                let target = oc_schema::Xf { rot: 50.0, ..xf.clone() };
                let (_, want) = drawn(&target, 0.0, 0.0, 0.0);
                assert!(((turned - want + 540.0) % 360.0 - 180.0).abs() < 1e-6, "rot {rot} flips {fx} {fy}: {turned} vs {want}");
            }
        }
    }

    fn box_el(rot: f64, opacity: f64) -> Value {
        json!({ "type": "shape", "xf": { "x": 100, "y": 200, "w": 80, "h": 40, "rot": rot }, "opacity": opacity })
    }

    #[test]
    fn keyframes_store_offsets_and_read_back_as_written() {
        let el = box_el(0.0, 1.0);
        let rest = Rest::of(&el, Some((100.0, 200.0)));
        let spec = json!({
            "position": [{ "at": 0, "value": [-200, 200] }, { "at": 0.6, "value": [100, 200], "ease": "ease-out" }],
            "scale": [{ "at": 0, "value": 0.5 }, { "at": 0.6, "value": 1, "ease": "back" }],
            "rotation": [{ "at": 1, "value": 0 }, { "at": 1.5, "value": 90, "ease": "hold" }],
            "opacity": [{ "at": 0, "value": 0 }, { "at": 0.3, "value": 1, "ease": "linear" }],
        });
        let stored = keyframes_patch(&spec, &el, &rest).unwrap();
        assert_eq!(stored["x"], json!([{ "t": 0, "v": -300 }, { "t": 600, "v": 0, "ease": "ease-out" }]));
        assert_eq!(stored["y"], json!([{ "t": 0, "v": 0 }, { "t": 600, "v": 0, "ease": "ease-out" }]));
        assert_eq!(stored["rotate"], json!([{ "t": 1000, "v": 0 }, { "t": 1500, "v": 90, "ease": "hold" }]));
        let mut with = el.clone();
        with["keyframes"] = stored;
        assert_eq!(keyframes_view(&with, &rest).unwrap(), spec);
        // Rotated: position still reads back in page space.
        let el = box_el(90.0, 0.5);
        let rest = Rest::of(&el, Some((120.0, 180.0)));
        let spec = json!({ "position": [{ "at": 0, "value": [20, 180] }, { "at": 1, "value": [120, 180] }], "opacity": [{ "at": 0, "value": 0 }, { "at": 1, "value": 0.5 }] });
        let stored = keyframes_patch(&spec, &el, &rest).unwrap();
        assert_eq!((stored["x"][0]["v"].clone(), stored["y"][0]["v"].clone()), (json!(0), json!(100)));
        assert_eq!(stored["opacity"][1]["v"], json!(1));
        let mut with = el.clone();
        with["keyframes"] = stored;
        assert_eq!(keyframes_view(&with, &rest).unwrap(), spec);
    }

    #[test]
    fn keyframe_curves_store_a_bezier_with_a_fallback_and_read_back() {
        let el = box_el(0.0, 1.0);
        let rest = Rest::of(&el, Some((100.0, 200.0)));
        let spec = json!({
            "scale": [{ "at": 0, "value": 0 }, { "at": 0.5, "value": 1, "ease": "out-back" }],
            "rotation": [{ "at": 0, "value": 0 }, { "at": 1, "value": 45, "ease": [0.2, 0.7, 0.4, 1] }],
            "opacity": [{ "at": 0, "value": 0 }, { "at": 1, "value": 1, "ease": "in-out-quart" }],
        });
        let stored = keyframes_patch(&spec, &el, &rest).unwrap();
        // Older builds read `ease` (back, the default), newer ones the curve.
        assert_eq!(stored["scale"][1], json!({ "t": 500, "v": 1, "ease": "back", "bezier": [0.34, 1.56, 0.64, 1] }));
        assert_eq!(stored["rotate"][1], json!({ "t": 1000, "v": 45, "bezier": [0.2, 0.7, 0.4, 1] }));
        assert_eq!(stored["opacity"][1], json!({ "t": 1000, "v": 1, "bezier": [0.76, 0, 0.24, 1] }));
        let mut with = el.clone();
        with["keyframes"] = stored;
        assert_eq!(keyframes_view(&with, &rest).unwrap(), spec);
        // Curves that are not a function of time, or swing too far, are refused.
        for bad in [json!([1.2, 0, 0.5, 1]), json!([0.2, 5, 0.5, 1]), json!([0.2, 0.5])] {
            let e = keyframes_patch(&json!({ "scale": [{ "at": 0, "value": 1, "ease": bad }] }), &el, &rest).unwrap_err();
            assert!(e.starts_with("keyframes.scale at 0 s: ease"), "{e}");
        }
    }

    #[test]
    fn keyframe_tracks_replace_merge_and_clear() {
        let mut el = box_el(0.0, 1.0);
        el["keyframes"] = json!({ "x": [{ "t": 0, "v": 5 }], "blur": [{ "t": 0, "v": 4 }] });
        let rest = Rest::of(&el, Some((100.0, 200.0)));
        // Tracks not named stay; a track given replaces; null removes one, or all.
        let p = keyframes_patch(&json!({ "scale": [{ "at": 0, "value": 2 }], "blur": null }), &el, &rest).unwrap();
        assert_eq!(p, json!({ "x": [{ "t": 0, "v": 5 }], "scale": [{ "t": 0, "v": 2 }] }));
        assert_eq!(keyframes_patch(&json!({ "x": null, "blur": null }), &el, &rest).unwrap(), Value::Null);
        assert_eq!(keyframes_patch(&Value::Null, &el, &rest).unwrap(), Value::Null);
    }

    #[test]
    fn keyframes_refuse_what_cannot_be_shown() {
        let el = box_el(0.0, 0.5);
        let rest = Rest::of(&el, Some((100.0, 200.0)));
        let err = |spec: Value, rest: &Rest| keyframes_patch(&spec, &el, rest).unwrap_err();
        assert!(err(json!({ "opacity": [{ "at": 0, "value": 1 }] }), &rest).contains("above the element's own opacity"));
        assert!(err(json!({ "scale": [{ "at": 1, "value": 1 }, { "at": 1, "value": 2 }] }), &rest).contains("time order"));
        assert!(err(json!({ "position": [{ "at": 0, "value": [0, 0] }], "x": [{ "at": 0, "value": 0 }] }), &rest).contains("not more than one"));
        let rotated = Rest { rot: 30.0, ..rest };
        assert!(err(json!({ "x": [{ "at": 0, "value": 0 }] }), &rotated).contains("key position"));
        let in_rotated_group = Rest { page: None, ..rest };
        assert!(err(json!({ "position": [{ "at": 0, "value": [0, 0] }] }), &in_rotated_group).contains("rotated group"));
        // Scale, rotation and blur need no page position.
        assert!(keyframes_patch(&json!({ "rotation": [{ "at": 0, "value": 10 }] }), &el, &in_rotated_group).is_ok());
    }

    #[test]
    fn motion_paths_go_through_page_points() {
        let el = box_el(0.0, 1.0);
        let rest = Rest::of(&el, Some((100.0, 200.0)));
        let p = motion_path_patch(&json!({ "via": [[100, 200], [300, 100]], "duration": 1.5, "delay": 0.25 }), &rest).unwrap();
        assert_eq!(p, json!({ "points": [0, 0, 200, -100], "durationMs": 1500, "delayMs": 250, "easing": "linear" }));
        assert_eq!(motion_path_view(&p, &rest), json!({ "via": [[100, 200], [300, 100]], "duration": 1.5, "delay": 0.25 }));
        // Smooth paths pass through every point given.
        let p = motion_path_patch(&json!({ "via": [[100, 200], [200, 100], [300, 200]], "duration": 1, "smooth": true, "easing": "ease" }), &rest).unwrap();
        let pts: Vec<f64> = p["points"].as_array().unwrap().iter().map(num).collect();
        assert_eq!(pts.len(), 2 * (2 * SMOOTH_STEPS + 1));
        assert_eq!((pts[2 * SMOOTH_STEPS], pts[2 * SMOOTH_STEPS + 1]), (100.0, -100.0));
        assert_eq!(&pts[pts.len() - 2..], &[200.0, 0.0]);
        assert!(motion_path_patch(&json!({ "via": [[0, 0], [1, 1]], "duration": 1 }), &Rest { page: None, ..rest }).is_err());
    }

    #[test]
    fn pages_take_an_entrance() {
        let p = page_patch(&json!({ "entrance": { "type": "scale", "duration": 0.4 } }), &Kit::default());
        assert_eq!(Value::Object(p.clone()), json!({ "anim": { "preset": "shrink", "durationMs": 400 } }));
        let page = json!({ "anim": p["anim"] });
        assert_eq!(page_view(&page, 3000.0)["entrance"], json!({ "type": "scale", "duration": 0.4 }));
        assert_eq!(Value::Object(page_patch(&json!({ "entrance": null }), &Kit::default())), json!({ "anim": null }));
    }

    #[test]
    fn formats_from_size() {
        assert_eq!(format_of(1080.0, 1920.0), Format::Vertical);
        assert_eq!(format_of(1080.0, 1350.0), Format::Feed);
        assert_eq!(format_of(1080.0, 1080.0), Format::Feed);
        assert_eq!(format_of(1920.0, 1080.0), Format::Other);
    }
}
