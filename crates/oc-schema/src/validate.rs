// SPDX-License-Identifier: Apache-2.0
//! Document validation: a port of the zod schemas in `validate.ts`, run on raw JSON.
//!
//! Objects are "loose": unknown fields are allowed (and preserved by the typed model). Issue paths and messages
//! reproduce zod 4's output as flattened by `validateDoc` (`path` joined with `.`, top-level issue messages only),
//! including zod's union rules (a single non-aborted branch reports its own issues, otherwise "Invalid input") and
//! its check-scheduling quirks (length checks run even after a type error).

use std::sync::LazyLock;

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::design::{CURRENT_SCHEMA, DesignDoc};
use crate::elements::ELEMENT_TYPES;
use crate::js;
use crate::svg::{MAX_SVG_COLORS, SVG_COLOR_KEY_SOURCE, SVG_COLOR_VALUE_SOURCE, re};

/// One validation problem (`{ path, message }` in TS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
enum Seg {
    Key(String),
    Idx(usize),
}

impl Seg {
    fn as_string(&self) -> String {
        match self {
            Seg::Key(k) => k.clone(),
            Seg::Idx(i) => i.to_string(),
        }
    }
}

/// zod issue continuation: `None` = implicit abort (type errors), `Some(true)` = continuable check,
/// `Some(false)` = explicit abort (`.int()`).
#[derive(Debug, Clone)]
struct Iss {
    path: Vec<Seg>,
    message: String,
    cont: Option<bool>,
}

impl Iss {
    fn new(message: impl Into<String>, cont: Option<bool>) -> Self {
        Iss { path: vec![], message: message.into(), cont }
    }
}

fn aborted(issues: &[Iss]) -> bool {
    issues.iter().any(|i| i.cont != Some(true))
}

fn explicitly_aborted(issues: &[Iss]) -> bool {
    issues.iter().any(|i| i.cont == Some(false))
}

fn prefix(mut issues: Vec<Iss>, seg: Seg) -> Vec<Iss> {
    for i in &mut issues {
        i.path.insert(0, seg.clone());
    }
    issues
}

fn received(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn invalid_type(expected: &str, v: Option<&Value>) -> Iss {
    Iss::new(format!("Invalid input: expected {expected}, received {}", received(v)), None)
}

type Pred = fn(f64) -> bool;
type RecordRefine = Option<(fn(&Map<String, Value>) -> bool, &'static str)>;
type KeyPattern = Option<(&'static LazyLock<Regex>, &'static str)>;

enum StrCheck {
    Min(usize),
    Max(usize),
    Url,
    Regex(&'static LazyLock<Regex>, &'static str),
}

enum Sch {
    Num(Vec<(Pred, Option<&'static str>)>),
    Int { min: Option<f64> },
    Str(Vec<StrCheck>),
    Bool,
    Enum(&'static [&'static str]),
    Lit(&'static str),
    Arr { item: Box<Sch>, min: Option<usize> },
    Tuple(Vec<Sch>),
    Record { key: KeyPattern, val: Box<Sch>, refine: RecordRefine },
    Obj(Vec<(&'static str, Sch, bool)>),
    Union(Vec<Sch>),
    /// `zLink` (SEC-01): anything, except a `url` link whose scheme is not allowed (`unsafe link URL` at `url`).
    Link,
}

/// Length-based checks (`.min()` / `.max()`) run on any value with a length (strings and arrays), even after a
/// type error, unless an earlier issue aborted explicitly.
fn length_check(v: Option<&Value>, n: usize, min: bool, issues: &[Iss]) -> Option<Iss> {
    if explicitly_aborted(issues) {
        return None;
    }
    let (len, origin, unit) = match v {
        Some(Value::String(s)) => (js::utf16_len(s), "string", "characters"),
        Some(Value::Array(a)) => (a.len(), "array", "items"),
        _ => return None,
    };
    if min && len < n {
        Some(Iss::new(format!("Too small: expected {origin} to have >={n} {unit}"), Some(true)))
    } else if !min && len > n {
        Some(Iss::new(format!("Too big: expected {origin} to have <={n} {unit}"), Some(true)))
    } else {
        None
    }
}

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

impl Sch {
    fn check(&self, v: Option<&Value>) -> Vec<Iss> {
        let mut out: Vec<Iss> = Vec::new();
        match self {
            Sch::Num(refines) => match v {
                Some(Value::Number(n)) => {
                    let x = n.as_f64().unwrap_or(f64::NAN);
                    for (pred, msg) in refines {
                        if !pred(x) {
                            out.push(Iss::new(msg.unwrap_or("Invalid input"), Some(true)));
                        }
                    }
                }
                _ => out.push(invalid_type("number", v)),
            },
            Sch::Int { min } => match v {
                Some(Value::Number(n)) => {
                    let x = n.as_f64().unwrap_or(f64::NAN);
                    if !js::is_integer(x) {
                        out.push(Iss::new("Invalid input: expected int, received number", Some(false)));
                    } else {
                        if x > MAX_SAFE {
                            out.push(Iss::new("Too big: expected int to be <=9007199254740991", Some(true)));
                        } else if x < -MAX_SAFE {
                            out.push(Iss::new("Too small: expected int to be >=-9007199254740991", Some(true)));
                        }
                        if let Some(m) = min
                            && x < *m
                        {
                            out.push(Iss::new(format!("Too small: expected number to be >={}", js::num_to_string(*m)), Some(true)));
                        }
                    }
                }
                _ => out.push(invalid_type("number", v)),
            },
            Sch::Str(checks) => {
                let s = match v {
                    Some(Value::String(s)) => Some(s.as_str()),
                    _ => {
                        out.push(invalid_type("string", v));
                        None
                    }
                };
                for c in checks {
                    match c {
                        StrCheck::Min(n) => out.extend(length_check(v, *n, true, &out)),
                        StrCheck::Max(n) => out.extend(length_check(v, *n, false, &out)),
                        StrCheck::Url => {
                            if let Some(s) = s
                                && !aborted(&out)
                                && url::Url::parse(js::trim(s)).is_err()
                            {
                                out.push(Iss::new("Invalid URL", Some(true)));
                            }
                        }
                        StrCheck::Regex(r, src) => {
                            if let Some(s) = s
                                && !aborted(&out)
                                && !r.is_match(s).unwrap_or(false)
                            {
                                out.push(Iss::new(format!("Invalid string: must match pattern {src}"), Some(true)));
                            }
                        }
                    }
                }
            }
            Sch::Bool => {
                if !matches!(v, Some(Value::Bool(_))) {
                    out.push(invalid_type("boolean", v));
                }
            }
            Sch::Enum(values) => {
                if !matches!(v, Some(Value::String(s)) if values.contains(&s.as_str())) {
                    let list = values.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join("|");
                    out.push(Iss::new(format!("Invalid option: expected one of {list}"), None));
                }
            }
            Sch::Lit(l) => {
                if !matches!(v, Some(Value::String(s)) if s == l) {
                    out.push(Iss::new(format!("Invalid input: expected \"{l}\""), None));
                }
            }
            Sch::Arr { item, min } => {
                match v {
                    Some(Value::Array(a)) => {
                        for (i, x) in a.iter().enumerate() {
                            out.extend(prefix(item.check(Some(x)), Seg::Idx(i)));
                        }
                    }
                    _ => out.push(invalid_type("array", v)),
                }
                if let Some(n) = min {
                    out.extend(length_check(v, *n, true, &out));
                }
            }
            Sch::Tuple(items) => match v {
                Some(Value::Array(a)) => {
                    // zod: too short → only `too_small`; too long → `too_big`, then the fixed items are checked.
                    let n = items.len();
                    if a.len() < n {
                        out.push(Iss::new(format!("Too small: expected array to have >={n} items"), None));
                    } else {
                        if a.len() > n {
                            out.push(Iss::new(format!("Too big: expected array to have <={n} items"), None));
                        }
                        for (i, (s, x)) in items.iter().zip(a).enumerate() {
                            out.extend(prefix(s.check(Some(x)), Seg::Idx(i)));
                        }
                    }
                }
                _ => out.push(invalid_type("tuple", v)),
            },
            Sch::Record { key, val, refine } => match v {
                Some(Value::Object(m)) => {
                    for (k, x) in m {
                        if let Some((r, _)) = key
                            && !r.is_match(k).unwrap_or(false)
                        {
                            let mut i = Iss::new("Invalid key in record", None);
                            i.path.push(Seg::Key(k.clone()));
                            out.push(i);
                            continue;
                        }
                        out.extend(prefix(val.check(Some(x)), Seg::Key(k.clone())));
                    }
                    if let Some((pred, msg)) = refine
                        && !aborted(&out)
                        && !pred(m)
                    {
                        out.push(Iss::new(*msg, Some(true)));
                    }
                }
                _ => out.push(invalid_type("record", v)),
            },
            Sch::Obj(fields) => match v {
                Some(Value::Object(m)) => {
                    for (k, s, optional) in fields {
                        let x = m.get(*k);
                        if *optional && x.is_none() {
                            continue;
                        }
                        out.extend(prefix(s.check(x), Seg::Key((*k).to_string())));
                    }
                }
                _ => out.push(invalid_type("object", v)),
            },
            Sch::Link => {
                if !crate::links::is_safe_link(v) {
                    let mut i = Iss::new("unsafe link URL", Some(true));
                    i.path.push(Seg::Key("url".into()));
                    out.push(i);
                }
            }
            Sch::Union(options) => {
                let results: Vec<Vec<Iss>> = options.iter().map(|o| o.check(v)).collect();
                if results.iter().any(Vec::is_empty) {
                    return out;
                }
                let non_aborted: Vec<&Vec<Iss>> = results.iter().filter(|r| !aborted(r)).collect();
                if non_aborted.len() == 1 {
                    return non_aborted[0].clone();
                }
                out.push(Iss::new("Invalid input", None));
            }
        }
        out
    }
}

// ------------------------------------------------------------------ schemas

fn finite(v: f64) -> bool {
    v.is_finite()
}

fn num() -> Sch {
    Sch::Num(vec![(finite, Some("must be finite"))])
}

fn num_r(p: Pred, msg: Option<&'static str>) -> Sch {
    Sch::Num(vec![(finite, Some("must be finite")), (p, msg)])
}

fn s() -> Sch {
    Sch::Str(vec![])
}

fn s_min1() -> Sch {
    Sch::Str(vec![StrCheck::Min(1)])
}

fn arr(item: Sch) -> Sch {
    Sch::Arr { item: Box::new(item), min: None }
}

fn arr_min(item: Sch, n: usize) -> Sch {
    Sch::Arr { item: Box::new(item), min: Some(n) }
}

fn obj(fields: Vec<(&'static str, Sch)>) -> Sch {
    Sch::Obj(fields.into_iter().map(|(k, s)| (k, s, false)).collect())
}

/// Object with optional fields marked by a trailing `?` in the key.
fn objo(fields: Vec<(&'static str, Sch)>) -> Sch {
    Sch::Obj(
        fields
            .into_iter()
            .map(|(k, s)| match k.strip_suffix('?') {
                Some(base) => (base, s, true),
                None => (k, s, false),
            })
            .collect(),
    )
}

fn z_color() -> Sch {
    Sch::Union(vec![
        obj(vec![("space", Sch::Enum(&["srgb", "display-p3"])), ("r", num()), ("g", num()), ("b", num()), ("a", num())]),
        obj(vec![("space", Sch::Lit("cmyk")), ("c", num()), ("m", num()), ("y", num()), ("k", num()), ("a", num())]),
        obj(vec![("space", Sch::Lit("spot")), ("swatch", s()), ("tint", num()), ("a", num())]),
    ])
}

fn z_stop() -> Sch {
    obj(vec![("t", num()), ("color", z_color())])
}

fn z_paint() -> Sch {
    Sch::Union(vec![
        obj(vec![("k", Sch::Lit("solid")), ("color", z_color())]),
        obj(vec![("k", Sch::Lit("linear")), ("stops", arr_min(z_stop(), 1)), ("angle", num())]),
        obj(vec![("k", Sch::Lit("radial")), ("stops", arr_min(z_stop(), 1))]),
        obj(vec![("k", Sch::Lit("image")), ("asset", s()), ("fit", Sch::Enum(&["fill", "fit", "tile"]))]),
    ])
}

fn z_stroke() -> Sch {
    objo(vec![
        ("paint", z_paint()),
        ("width", num_r(|v| v >= 0.0, None)),
        ("align", Sch::Enum(&["center", "inside", "outside"])),
        ("cap", Sch::Enum(&["butt", "round", "square"])),
        ("join", Sch::Enum(&["miter", "round", "bevel"])),
        ("dash?", arr(num())),
    ])
}

fn z_xf() -> Sch {
    objo(vec![
        ("x", num()),
        ("y", num()),
        ("w", num_r(|v| v >= 0.01, Some("w must be >= 0.01"))),
        ("h", num_r(|v| v >= 0.01, Some("h must be >= 0.01"))),
        ("rot", num()),
        ("flipX?", Sch::Bool),
        ("flipY?", Sch::Bool),
    ])
}

fn z_rect() -> Sch {
    obj(vec![("x", num()), ("y", num()), ("w", num()), ("h", num())])
}

fn z_media_clip() -> Sch {
    obj(vec![
        ("trimStartMs", num()),
        ("trimEndMs", num()),
        ("speed", num_r(|v| v > 0.0, None)),
        ("volume", num()),
        ("muted", Sch::Bool),
        ("loop", Sch::Bool),
    ])
}

fn z_run_style() -> Sch {
    objo(vec![
        ("family", s_min1()),
        ("weight", num()),
        ("italic", Sch::Bool),
        ("size", num_r(|v| v > 0.0, None)),
        ("fill", z_paint()),
        ("tracking", num()),
        ("link?", Sch::Link),
    ])
}

fn z_text_body() -> Sch {
    let run = objo(vec![("text", s()), ("style", objo(vec![("link?", Sch::Link)])), ("link?", Sch::Link)]);
    let paragraph = obj(vec![
        ("runs", arr(run)),
        ("align", Sch::Enum(&["start", "center", "end", "justify"])),
        ("lineHeight", num()),
        ("spaceAfter", num()),
    ]);
    obj(vec![("paragraphs", arr_min(paragraph, 1)), ("defaultRun", z_run_style())])
}

fn base_fields() -> Vec<(&'static str, Sch)> {
    vec![
        ("id", s_min1()),
        ("parentId", s_min1()),
        ("z", s_min1()),
        ("xf", z_xf()),
        ("opacity", num_r(|v| (0.0..=1.0).contains(&v), Some("opacity in 0..1"))),
        ("link?", Sch::Link),
    ]
}

static SVG_KEY_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^(currentColor|#[0-9a-f]{6}([0-9a-f]{2})?)$"));
static SVG_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^#[0-9a-f]{6}([0-9a-f]{2})?$"));

fn url_str(max: usize) -> Sch {
    Sch::Str(vec![StrCheck::Url, StrCheck::Max(max)])
}

fn element_schema(t: &str) -> Sch {
    let mut f = base_fields();
    let rest: Vec<(&'static str, Sch)> = match t {
        "shape" => vec![
            ("kind", s()),
            ("params", Sch::Record { key: None, val: Box::new(num()), refine: None }),
            ("fills", arr(z_paint())),
            ("strokes", arr(z_stroke())),
        ],
        "line" => vec![
            ("points", arr_min(Sch::Tuple(vec![num(), num()]), 2)),
            ("stroke", z_stroke()),
            ("startMarker", s()),
            ("endMarker", s()),
        ],
        "image" => vec![
            ("asset", s()),
            ("crop", z_rect()),
            (
                "svgColors?",
                Sch::Record {
                    key: Some((&SVG_KEY_RE, SVG_COLOR_KEY_SOURCE)),
                    val: Box::new(Sch::Str(vec![StrCheck::Regex(&SVG_VALUE_RE, SVG_COLOR_VALUE_SOURCE)])),
                    refine: Some((|m| m.len() <= MAX_SVG_COLORS, "too many colours")),
                },
            ),
        ],
        "frame" => vec![
            ("clip", s()),
            ("placeholderFill", z_paint()),
            ("clipPath?", s_min1()),
            ("overlay?", arr(obj(vec![("d", s_min1()), ("fill", z_paint())]))),
            ("content?", objo(vec![("asset", s()), ("crop", z_rect()), ("video?", z_media_clip())])),
        ],
        "group" => vec![],
        "text" => vec![
            ("body", z_text_body()),
            ("frame", obj(vec![("mode", Sch::Enum(&["autoHeight", "fixed"]))])),
            ("autoFit", Sch::Enum(&["none", "shrink"])),
            ("valign?", Sch::Enum(&["top", "middle", "bottom"])),
        ],
        "qr" => vec![("data", s()), ("ecc", Sch::Enum(&["L", "M", "Q", "H"])), ("fg", z_color()), ("bg", z_color())],
        "video" => vec![("asset", s()), ("crop", z_rect()), ("clip", z_media_clip())],
        "draw" => vec![
            ("points", arr(num())),
            ("base", obj(vec![("w", num()), ("h", num())])),
            ("brush", Sch::Enum(&["pen", "marker", "highlighter"])),
            ("color", z_color()),
            ("size", num()),
        ],
        "table" => {
            let body = obj(vec![("paragraphs", arr(obj(vec![("runs", arr(obj(vec![("text", s())])))])))]);
            let cell = objo(vec![
                ("text", s()),
                ("span?", obj(vec![("rows", Sch::Int { min: Some(1.0) }), ("cols", Sch::Int { min: Some(1.0) })])),
                ("valign?", Sch::Enum(&["top", "middle", "bottom"])),
                ("padding?", num()),
                ("body?", body),
            ]);
            vec![
                ("rows", arr_min(num(), 1)),
                ("cols", arr_min(num(), 1)),
                ("cells", Sch::Record { key: None, val: Box::new(cell), refine: None }),
                ("style", obj(vec![("family", s()), ("size", num())])),
            ]
        }
        "embed" => vec![
            ("url", url_str(2000)),
            ("provider", Sch::Enum(&["youtube", "vimeo", "spotify", "soundcloud", "maps", "loom", "figma", "codepen"])),
            ("src", url_str(2000)),
            ("title", Sch::Str(vec![StrCheck::Max(300)])),
        ],
        "form" => vec![
            ("title", s()),
            ("fields", arr(obj(vec![("id", s()), ("kind", s()), ("label", s())]))),
            ("submitLabel", s()),
            ("thanks", s()),
            ("style", obj(vec![("family", s()), ("size", num())])),
        ],
        "grid" => vec![
            ("cells", arr_min(objo(vec![("rect", z_rect()), ("video?", z_media_clip())]), 1)),
            ("gap", num()),
            ("radius", num()),
            ("placeholderFill", z_paint()),
        ],
        "chart" => vec![
            (
                "kind",
                Sch::Enum(&[
                    "bar", "column", "line", "area", "pie", "donut", "stacked-column", "stacked-bar", "combo", "scatter",
                    "bubble", "radar", "funnel", "progress-ring", "progress-bar", "gauge",
                ]),
            ),
            ("labels", arr(s())),
            ("series", arr(obj(vec![("name", s()), ("values", arr(num())), ("color", z_color())]))),
            ("style", obj(vec![("family", s())])),
        ],
        _ => vec![],
    };
    f.extend(rest);
    objo(f)
}

static ELEMENT_SCHEMAS: LazyLock<Vec<(&'static str, Sch)>> =
    LazyLock::new(|| ELEMENT_TYPES.iter().map(|t| (*t, element_schema(t))).collect());

static PAGE_SCHEMA: LazyLock<Sch> =
    LazyLock::new(|| obj(vec![("id", s()), ("order", s_min1()), ("background", arr(z_paint()))]));

static META_SCHEMA: LazyLock<Sch> = LazyLock::new(|| {
    obj(vec![
        ("schemaVersion", Sch::Int { min: None }),
        ("minClientSchema", Sch::Int { min: None }),
        ("layoutVersion", Sch::Int { min: None }),
        ("surface", s()),
        (
            "dims",
            obj(vec![
                ("w", num_r(|v| v > 0.0, None)),
                ("h", num_r(|v| v > 0.0, None)),
                ("unit", Sch::Enum(&["px", "mm", "cm", "in", "pt"])),
                ("dpi", num()),
            ]),
        ),
        ("lang", s()),
        ("title", s()),
        ("epoch", Sch::Int { min: None }),
    ])
});

/// Properties every JS object inherits (the TS `parent in pages` check sees them).
const OBJECT_PROTOTYPE_KEYS: &[&str] = &[
    "constructor", "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable", "toLocaleString", "toString", "valueOf",
    "__proto__", "__defineGetter__", "__defineSetter__", "__lookupGetter__", "__lookupSetter__",
];

/// JS `Object.entries(x ?? {})` for a JSON value.
fn entries(v: Option<&Value>) -> Vec<(String, &Value)> {
    match v {
        Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
        _ => vec![],
    }
}

fn push_all(out: &mut Vec<ValidationIssue>, pre: &str, issues: Vec<Iss>) {
    for i in issues {
        let mut parts = vec![pre.to_string()];
        parts.extend(i.path.iter().map(Seg::as_string));
        out.push(ValidationIssue { path: parts.join("."), message: i.message });
    }
}

/// Validates a design snapshot (raw JSON). `Ok(())` when it is well-formed.
pub fn validate_doc(doc: &Value) -> Result<(), Vec<ValidationIssue>> {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let d = match doc {
        Value::Object(m) => Some(m),
        Value::Array(_) => None,
        _ => return Err(vec![ValidationIssue { path: String::new(), message: "not an object".into() }]),
    };
    let get = |k: &str| d.and_then(|m| m.get(k));
    let meta = get("meta");
    let m = META_SCHEMA.check(meta);
    if !m.is_empty() {
        push_all(&mut issues, "meta", m);
    } else if let Some(sv) = meta.and_then(|m| m.get("schemaVersion")).and_then(Value::as_f64)
        && sv > CURRENT_SCHEMA as f64
    {
        issues.push(ValidationIssue {
            path: "meta.schemaVersion".into(),
            message: format!("document is schema {}, this build reads {CURRENT_SCHEMA}", js::num_to_string(sv)),
        });
    }
    let pages = entries(get("pages"));
    if pages.is_empty() {
        issues.push(ValidationIssue { path: "pages".into(), message: "at least one page is required".into() });
    }
    for (id, p) in &pages {
        let r = PAGE_SCHEMA.check(Some(p));
        if !r.is_empty() {
            push_all(&mut issues, &format!("pages.{id}"), r);
        } else if p.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            issues.push(ValidationIssue { path: format!("pages.{id}.id"), message: "id does not match key".into() });
        }
    }
    let elements = entries(get("elements"));
    let has_key = |list: &[(String, &Value)], k: &str| list.iter().any(|(id, _)| id == k) || OBJECT_PROTOTYPE_KEYS.contains(&k);
    for (id, e) in &elements {
        let t = e.get("type").and_then(Value::as_str);
        let Some((_, schema)) = ELEMENT_SCHEMAS.iter().find(|(name, _)| Some(*name) == t) else { continue };
        let r = schema.check(Some(e));
        if !r.is_empty() {
            push_all(&mut issues, &format!("elements.{id}"), r);
        } else if e.get("id").and_then(Value::as_str) != Some(id.as_str()) {
            issues.push(ValidationIssue { path: format!("elements.{id}.id"), message: "id does not match key".into() });
        }
        if let Some(parent) = e.get("parentId").filter(|p| crate::tree::truthy(p)) {
            let ps = js::value_to_js_string(parent);
            if !has_key(&pages, &ps) && !has_key(&elements, &ps) {
                issues.push(ValidationIssue { path: format!("elements.{id}.parentId"), message: format!("parent {ps} does not exist") });
            }
        }
    }
    if issues.is_empty() { Ok(()) } else { Err(issues) }
}

/// Validates and then parses into the typed model.
pub fn parse_doc(doc: &Value) -> Result<DesignDoc, Vec<ValidationIssue>> {
    validate_doc(doc)?;
    serde_json::from_value(doc.clone()).map_err(|e| vec![ValidationIssue { path: String::new(), message: e.to_string() }])
}
