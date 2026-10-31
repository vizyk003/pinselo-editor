//! Request validation with zod 4 semantics (the TS routes call `z.object({...}).parse(req.body)` and the error
//! handler answers `400 {"error":"invalid_request","issues": err.issues}`).
//!
//! The issue objects (codes, key order, messages, paths) are byte-compatible with zod 4.6 for the schema
//! features the API uses: objects (unknown keys stripped), strings (`min`/`max` in UTF-16 units, `email`,
//! `uuid`, `regex`), numbers (`int`, `min`/`max`, `positive`, `nonnegative`), booleans, literals, enums, arrays
//! (`min`/`max`), unions (zod 4's `invalid_union` / single non-aborted option rule), loose objects
//! (`z.looseObject`: unknown keys kept), `optional`, `nullable` and `default`. Checks run in declaration order and do not abort, except a
//! failed `int` (zod's safe-integer format aborts the remaining number checks).
//!
//! ```ignore
//! let b = obj([("email", str().email().max(200)), ("name", str().max(80).optional())]).parse(req.body())?;
//! ```

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use crate::http::error::ApiError;

pub const EMAIL_PATTERN: &str = r"^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";
pub const DATETIME_PATTERN: &str = r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z))$";
pub const UUID_PATTERN: &str = r"^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$";

static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(EMAIL_PATTERN).expect("email regex"));
static DATETIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(DATETIME_PATTERN).expect("datetime regex"));
/// `.datetime({ offset: true })`: `Z` or a `±hh:mm` offset.
pub const DATETIME_OFFSET_PATTERN: &str = r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z|([+-](?:[01]\d|2[0-3]):[0-5]\d)))$";
static DATETIME_OFFSET: LazyLock<Regex> = LazyLock::new(|| Regex::new(DATETIME_OFFSET_PATTERN).expect("datetime regex"));
static UUID: LazyLock<Regex> = LazyLock::new(|| Regex::new(UUID_PATTERN).expect("uuid regex"));

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

#[derive(Clone, Debug)]
enum StrCheck {
    Min(usize),
    Max(usize),
    Email,
    Uuid,
    Datetime,
    /// `.url()` (WHATWG URL parse of the trimmed string; the value becomes the trimmed string)
    Url,
    /// (JS source `/…/`, compiled Rust regex)
    Regex(String, Regex),
    /// `.datetime({ offset: true })`
    DatetimeOffset,
    /// `.toLowerCase()` (an overwrite: later checks see the lower-cased value)
    Lower,
    /// `.trim()` (an overwrite, JS `String.prototype.trim`)
    Trim,
    /// `.regex(/js/)` whose JS pattern has no Rust `regex` equivalent (lookaround): a predicate instead.
    RegexFn(String, fn(&str) -> bool),
}

#[derive(Clone, Debug)]
enum NumCheck {
    Int,
    Min(f64, bool),
    Max(f64, bool),
}

#[derive(Clone, Debug)]
enum Kind {
    Any,
    Str(Vec<StrCheck>),
    Num(Vec<NumCheck>),
    Bool,
    Literal(Value),
    Enum(Vec<String>),
    /// (item, min, max)
    Array(Box<Schema>, Option<usize>, Option<usize>),
    /// (fields, loose: unknown keys are kept after the declared ones)
    Object(Vec<(String, Schema)>, bool),
    /// `z.union([...])`
    Union(Vec<Schema>),
    /// `z.record(z.string(), value)`
    Record(Box<Schema>),
}

/// A zod schema.
#[derive(Clone, Debug)]
pub struct Schema {
    kind: Kind,
    optional: bool,
    nullable: bool,
    default: Option<Value>,
    /// `z.coerce.number()`: the input goes through JS `Number(input)` first.
    coerce: bool,
}

fn schema(kind: Kind) -> Schema {
    Schema { kind, optional: false, nullable: false, default: None, coerce: false }
}

/// `z.string()`
pub fn str() -> Schema {
    schema(Kind::Str(vec![]))
}
/// `z.number()`
pub fn num() -> Schema {
    schema(Kind::Num(vec![]))
}
/// `z.boolean()`
pub fn bool() -> Schema {
    schema(Kind::Bool)
}
/// `z.any()` / `z.unknown()`
pub fn any() -> Schema {
    schema(Kind::Any)
}
/// `z.literal(v)`
pub fn literal(v: Value) -> Schema {
    schema(Kind::Literal(v))
}
/// `z.enum([...])`
pub fn enumeration(values: &[&str]) -> Schema {
    schema(Kind::Enum(values.iter().map(|s| (*s).to_owned()).collect()))
}
/// `z.array(item)`
pub fn array(item: Schema) -> Schema {
    schema(Kind::Array(Box::new(item), None, None))
}
/// `z.record(z.string(), value)`
pub fn record(value: Schema) -> Schema {
    schema(Kind::Record(Box::new(value)))
}
/// `z.coerce.number()`
pub fn coerce_num() -> Schema {
    Schema { coerce: true, ..num() }
}

/// JS `Number(v)` of a JSON value (`z.coerce.number()`, `Number(req.query.x)`).
pub fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js_string_to_number(s),
        Some(Value::Array(a)) => match a.as_slice() {
            [] => 0.0,
            [x] => js_number(Some(x)),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// JS `Number(string)` (StringToNumber).
pub fn js_string_to_number(s: &str) -> f64 {
    let t = s.trim_matches(oc_js::is_js_whitespace);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |p: &str, r: u32| -> Option<f64> {
        let d = t.strip_prefix(p).or_else(|| t.strip_prefix(&p.to_uppercase()))?;
        if d.is_empty() || !d.chars().all(|c| c.is_digit(r)) {
            return Some(f64::NAN);
        }
        Some(d.chars().fold(0.0, |acc, c| acc * f64::from(r) + f64::from(c.to_digit(r).unwrap_or(0))))
    };
    if let Some(v) = radix("0x", 16).or_else(|| radix("0o", 8)).or_else(|| radix("0b", 2)) {
        return v;
    }
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    if body == "Infinity" {
        return if t.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    static DEC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$").expect("decimal regex"));
    if DEC.is_match(t) { t.parse().unwrap_or(f64::NAN) } else { f64::NAN }
}

/// `z.object({...})`
pub fn obj<const N: usize>(fields: [(&str, Schema); N]) -> Schema {
    schema(Kind::Object(fields.into_iter().map(|(k, s)| (k.to_owned(), s)).collect(), false))
}

/// `z.object({...})` from a list built at run time.
pub fn obj_vec(fields: Vec<(&str, Schema)>) -> Schema {
    schema(Kind::Object(fields.into_iter().map(|(k, s)| (k.to_owned(), s)).collect(), false))
}

/// `z.looseObject({...})`: like [`obj`], but unknown keys are kept.
pub fn loose_obj<const N: usize>(fields: [(&str, Schema); N]) -> Schema {
    schema(Kind::Object(fields.into_iter().map(|(k, s)| (k.to_owned(), s)).collect(), true))
}

/// `z.union([...])`: the first option that parses; otherwise the issues of the one option whose issues are all
/// non-aborting checks, or `invalid_union` with every option's issues (paths relative to the union).
pub fn union(options: Vec<Schema>) -> Schema {
    schema(Kind::Union(options))
}

/// A zod issue that aborts its schema (no `continue: true`): type and value mismatches, nested unions, and an
/// oversized array's size issue (marked with [`ABORT`]).
fn aborts(issue: &Value) -> bool {
    issue.get(ABORT).is_some() || matches!(issue.get("code").and_then(Value::as_str), Some("invalid_type" | "invalid_value" | "invalid_union"))
}

/// Internal key on an issue that zod reports with `continue: false` although its code normally continues. Removed
/// by [`strip_abort`] before the issues leave this module.
const ABORT: &str = "~abort";

fn strip_abort(issues: &mut [Value]) {
    for iss in issues {
        if let Value::Object(m) = iss {
            m.remove(ABORT);
            if let Some(Value::Array(errs)) = m.get_mut("errors") {
                for e in errs {
                    if let Value::Array(inner) = e {
                        strip_abort(inner);
                    }
                }
            }
        }
    }
}

impl Schema {
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }
    pub fn default(mut self, v: Value) -> Self {
        self.default = Some(v);
        self
    }
    fn push_str(mut self, c: StrCheck) -> Self {
        if let Kind::Str(v) = &mut self.kind {
            v.push(c);
        }
        self
    }
    fn push_num(mut self, c: NumCheck) -> Self {
        if let Kind::Num(v) = &mut self.kind {
            v.push(c);
        }
        self
    }
    /// String/array `.min(n)`, number `.min(n)`.
    pub fn min(mut self, n: f64) -> Self {
        match &mut self.kind {
            Kind::Str(_) => self.push_str(StrCheck::Min(n as usize)),
            Kind::Array(_, m, _) => {
                *m = Some(n as usize);
                self
            }
            _ => self.push_num(NumCheck::Min(n, true)),
        }
    }
    /// String/array `.max(n)`, number `.max(n)`.
    pub fn max(mut self, n: f64) -> Self {
        match &mut self.kind {
            Kind::Str(_) => self.push_str(StrCheck::Max(n as usize)),
            Kind::Array(_, _, m) => {
                *m = Some(n as usize);
                self
            }
            _ => self.push_num(NumCheck::Max(n, true)),
        }
    }
    pub fn email(self) -> Self {
        self.push_str(StrCheck::Email)
    }
    pub fn uuid(self) -> Self {
        self.push_str(StrCheck::Uuid)
    }
    pub fn url(self) -> Self {
        self.push_str(StrCheck::Url)
    }
    /// `.datetime()` (ISO 8601, UTC `Z` only, seconds required).
    pub fn datetime(self) -> Self {
        self.push_str(StrCheck::Datetime)
    }
    /// `.regex(/js/)`: `js` is the JS source between the slashes; `rust` an equivalent Rust pattern.
    pub fn regex(self, js: &str, rust: &str) -> Self {
        self.push_str(StrCheck::Regex(format!("/{js}/"), Regex::new(rust).expect("zod regex")))
    }
    /// `.regex(/js/)` checked by a predicate (for JS patterns the `regex` crate cannot express).
    pub fn regex_fn(self, js: &str, test: fn(&str) -> bool) -> Self {
        self.push_str(StrCheck::RegexFn(format!("/{js}/"), test))
    }
    /// `.datetime({ offset: true })` (ISO 8601 with `Z` or `±hh:mm`, seconds required).
    pub fn datetime_offset(self) -> Self {
        self.push_str(StrCheck::DatetimeOffset)
    }
    /// `.toLowerCase()`.
    pub fn to_lower_case(self) -> Self {
        self.push_str(StrCheck::Lower)
    }
    /// `.trim()`.
    pub fn trim(self) -> Self {
        self.push_str(StrCheck::Trim)
    }
    pub fn int(self) -> Self {
        self.push_num(NumCheck::Int)
    }
    pub fn positive(self) -> Self {
        self.push_num(NumCheck::Min(0.0, false))
    }
    pub fn nonnegative(self) -> Self {
        self.push_num(NumCheck::Min(0.0, true))
    }

    /// `schema.parse(input)` (`None` = `undefined`). Errors become the 400 `invalid_request` reply.
    pub fn parse(&self, input: Option<&Value>) -> Result<Value, ApiError> {
        let mut issues = Vec::new();
        let out = self.run(input, &mut Vec::new(), &mut issues);
        strip_abort(&mut issues);
        if issues.is_empty() { Ok(out.unwrap_or(Value::Null)) } else { Err(ApiError::Invalid(issues)) }
    }

    /// `schema.safeParse(input)`: the parsed value or the issues.
    pub fn safe_parse(&self, input: Option<&Value>) -> Result<Value, Vec<Value>> {
        let mut issues = Vec::new();
        let out = self.run(input, &mut Vec::new(), &mut issues);
        strip_abort(&mut issues);
        if issues.is_empty() { Ok(out.unwrap_or(Value::Null)) } else { Err(issues) }
    }

    fn run(&self, input: Option<&Value>, path: &mut Vec<Value>, issues: &mut Vec<Value>) -> Option<Value> {
        match input {
            None => {
                if let Some(d) = &self.default {
                    return Some(d.clone());
                }
                if self.optional {
                    return None;
                }
            }
            Some(Value::Null) if self.nullable => return Some(Value::Null),
            _ => {}
        }
        match &self.kind {
            Kind::Any => input.cloned(),
            Kind::Str(checks) => {
                let Some(Value::String(s)) = input else {
                    issues.push(invalid_type("string", input, path));
                    return None;
                };
                let mut s = s.clone();
                for c in checks {
                    let len = s.encode_utf16().count();
                    match c {
                        StrCheck::Url => {
                            let t = s.trim().to_owned();
                            if url::Url::parse(&t).is_ok() {
                                s = t;
                            } else {
                                issues.push(json_obj([("code", json!("invalid_format")), ("format", json!("url")), ("path", Value::Array(path.clone())), ("message", json!("Invalid URL"))]));
                            }
                        }
                        StrCheck::Min(n) if len < *n => issues.push(too_small("string", *n as f64, true, path)),
                        StrCheck::Max(n) if len > *n => issues.push(too_big("string", *n as f64, true, path)),
                        StrCheck::Email if !EMAIL.is_match(&s) => issues.push(format_issue("email", &format!("/{EMAIL_PATTERN}/"), "Invalid email address", path)),
                        StrCheck::Uuid if !UUID.is_match(&s) => issues.push(format_issue("uuid", &format!("/{UUID_PATTERN}/"), "Invalid UUID", path)),
                        StrCheck::Datetime if !DATETIME.is_match(&s) => issues.push(format_issue("datetime", &format!("/{DATETIME_PATTERN}/"), "Invalid ISO datetime", path)),
                        StrCheck::Regex(js, re) if !re.is_match(&s) => {
                            issues.push(format_issue("regex", js, &format!("Invalid string: must match pattern {js}"), path))
                        }
                        StrCheck::RegexFn(js, test) if !test(&s) => {
                            issues.push(format_issue("regex", js, &format!("Invalid string: must match pattern {js}"), path))
                        }
                        StrCheck::DatetimeOffset if !DATETIME_OFFSET.is_match(&s) => {
                            issues.push(format_issue("datetime", &format!("/{DATETIME_OFFSET_PATTERN}/"), "Invalid ISO datetime", path))
                        }
                        StrCheck::Lower => s = s.to_lowercase(),
                        StrCheck::Trim => s = crate::js::trim(&s).to_owned(),
                        _ => {}
                    }
                }
                Some(Value::String(s))
            }
            Kind::Num(checks) => {
                let coerced;
                let input = if self.coerce {
                    let f = js_number(input);
                    if !f.is_finite() {
                        let r = if f.is_nan() { "NaN" } else if f > 0.0 { "Infinity" } else { "-Infinity" };
                        issues.push(json_obj([
                            ("expected", json!("number")),
                            ("code", json!("invalid_type")),
                            ("received", json!(r)),
                            ("path", Value::Array(path.clone())),
                            ("message", json!(format!("Invalid input: expected number, received {r}"))),
                        ]));
                        return None;
                    }
                    coerced = crate::http::jsjson::number_value(f);
                    coerced.as_ref()
                } else {
                    input
                };
                let Some(Value::Number(n)) = input else {
                    issues.push(invalid_type("number", input, path));
                    return None;
                };
                let f = n.as_f64().unwrap_or(f64::NAN);
                for c in checks {
                    match c {
                        NumCheck::Int => {
                            if f.fract() != 0.0 {
                                issues.push(json_obj([
                                    ("expected", json!("int")),
                                    ("format", json!("safeint")),
                                    ("code", json!("invalid_type")),
                                    ("path", Value::Array(path.clone())),
                                    ("message", json!("Invalid input: expected int, received number")),
                                ]));
                                return None;
                            }
                            if f > MAX_SAFE {
                                issues.push(json_obj([
                                    ("code", json!("too_big")),
                                    ("maximum", json!(9_007_199_254_740_991_i64)),
                                    ("note", json!("Integers must be within the safe integer range.")),
                                    ("origin", json!("int")),
                                    ("inclusive", json!(true)),
                                    ("path", Value::Array(path.clone())),
                                    ("message", json!("Too big: expected int to be <=9007199254740991")),
                                ]));
                            } else if f < -MAX_SAFE {
                                issues.push(json_obj([
                                    ("code", json!("too_small")),
                                    ("minimum", json!(-9_007_199_254_740_991_i64)),
                                    ("note", json!("Integers must be within the safe integer range.")),
                                    ("origin", json!("int")),
                                    ("inclusive", json!(true)),
                                    ("path", Value::Array(path.clone())),
                                    ("message", json!("Too small: expected int to be >=-9007199254740991")),
                                ]));
                            }
                        }
                        NumCheck::Min(m, inclusive) => {
                            if if *inclusive { f < *m } else { f <= *m } {
                                issues.push(too_small("number", *m, *inclusive, path));
                            }
                        }
                        NumCheck::Max(m, inclusive) => {
                            if if *inclusive { f > *m } else { f >= *m } {
                                issues.push(too_big("number", *m, *inclusive, path));
                            }
                        }
                    }
                }
                Some(Value::Number(n.clone()))
            }
            Kind::Bool => match input {
                Some(Value::Bool(b)) => Some(Value::Bool(*b)),
                _ => {
                    issues.push(invalid_type("boolean", input, path));
                    None
                }
            },
            Kind::Literal(v) => {
                if input == Some(v) {
                    Some(v.clone())
                } else {
                    issues.push(invalid_value(std::slice::from_ref(v), path));
                    None
                }
            }
            Kind::Enum(values) => match input {
                Some(Value::String(s)) if values.contains(s) => Some(Value::String(s.clone())),
                _ => {
                    let vals: Vec<Value> = values.iter().map(|v| Value::String(v.clone())).collect();
                    issues.push(invalid_value(&vals, path));
                    None
                }
            },
            Kind::Array(item, min, max) => {
                let Some(Value::Array(a)) = input else {
                    issues.push(invalid_type("array", input, path));
                    return None;
                };
                // zod >= 4.6.6: an array over its max fails that check first, aborting, and no item is parsed.
                if let Some(m) = max
                    && a.len() > *m
                {
                    let mut iss = too_big("array", *m as f64, true, path);
                    if let Value::Object(o) = &mut iss {
                        o.insert(ABORT.into(), Value::Bool(true));
                    }
                    issues.push(iss);
                    return None;
                }
                let mut out = Vec::with_capacity(a.len());
                for (i, x) in a.iter().enumerate() {
                    path.push(json!(i));
                    if let Some(v) = item.run(Some(x), path, issues) {
                        out.push(v);
                    }
                    path.pop();
                }
                if let Some(m) = min
                    && a.len() < *m
                {
                    issues.push(too_small("array", *m as f64, true, path));
                }
                Some(Value::Array(out))
            }
            Kind::Union(options) => {
                let mut results = Vec::with_capacity(options.len());
                for o in options {
                    let mut own = Vec::new();
                    let v = o.run(input, &mut Vec::new(), &mut own);
                    if own.is_empty() {
                        return v;
                    }
                    results.push(own);
                }
                let nonaborted: Vec<&Vec<Value>> = results.iter().filter(|r| !r.iter().any(aborts)).collect();
                if let [only] = nonaborted.as_slice() {
                    for iss in only.iter() {
                        let mut iss = iss.clone();
                        if let Some(Value::Array(p)) = iss.get_mut("path") {
                            let mut full = path.clone();
                            full.append(p);
                            *p = full;
                        }
                        issues.push(iss);
                    }
                } else {
                    issues.push(json_obj([
                        ("code", json!("invalid_union")),
                        ("errors", Value::Array(results.into_iter().map(Value::Array).collect())),
                        ("path", Value::Array(path.clone())),
                        ("message", json!("Invalid input")),
                    ]));
                }
                None
            }
            Kind::Object(fields, loose) => {
                let Some(Value::Object(m)) = input else {
                    issues.push(invalid_type("object", input, path));
                    return None;
                };
                let mut out = Map::new();
                for (k, s) in fields {
                    path.push(Value::String(k.clone()));
                    // zod 4: a missing key of a required `z.any()`/`z.unknown()` field is `expected nonoptional`.
                    if matches!(s.kind, Kind::Any) && !s.optional && s.default.is_none() && !m.contains_key(k) {
                        issues.push(json_obj([
                            ("code", json!("invalid_type")),
                            ("expected", json!("nonoptional")),
                            ("path", Value::Array(path.clone())),
                            ("message", json!("Invalid input: expected nonoptional, received undefined")),
                        ]));
                        path.pop();
                        continue;
                    }
                    if let Some(v) = s.run(m.get(k), path, issues) {
                        out.insert(k.clone(), v);
                    }
                    path.pop();
                }
                if *loose {
                    for (k, v) in m {
                        if !fields.iter().any(|(f, _)| f == k) {
                            out.insert(k.clone(), v.clone());
                        }
                    }
                }
                Some(Value::Object(out))
            }
            Kind::Record(value) => {
                let Some(Value::Object(m)) = input else {
                    issues.push(invalid_type("record", input, path));
                    return None;
                };
                let mut out = Map::new();
                for (k, x) in m {
                    path.push(Value::String(k.clone()));
                    if let Some(v) = value.run(Some(x), path, issues) {
                        out.insert(k.clone(), v);
                    }
                    path.pop();
                }
                Some(Value::Object(out))
            }
        }
    }
}

fn json_obj<const N: usize>(kv: [(&str, Value); N]) -> Value {
    Value::Object(kv.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// zod's `parsedType`.
fn received(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        Some(Value::String(_)) => "string",
        Some(Value::Number(_)) => "number",
        Some(Value::Bool(_)) => "boolean",
    }
}

fn invalid_type(expected: &str, input: Option<&Value>, path: &[Value]) -> Value {
    json_obj([
        ("expected", json!(expected)),
        ("code", json!("invalid_type")),
        ("path", Value::Array(path.to_vec())),
        ("message", json!(format!("Invalid input: expected {expected}, received {}", received(input)))),
    ])
}

fn bound(n: f64) -> Value {
    crate::http::jsjson::number_value(n).unwrap_or(Value::Null)
}

fn unit(origin: &str) -> &'static str {
    match origin {
        "string" => "characters",
        "array" => "items",
        _ => "",
    }
}

fn describe(origin: &str, op: &str, n: f64) -> String {
    let num = oc_js::number_to_string(n);
    match origin {
        "string" | "array" => format!("expected {origin} to have {op}{num} {}", unit(origin)),
        _ => format!("expected {origin} to be {op}{num}"),
    }
}

fn too_small(origin: &str, min: f64, inclusive: bool, path: &[Value]) -> Value {
    let op = if inclusive { ">=" } else { ">" };
    json_obj([
        ("origin", json!(origin)),
        ("code", json!("too_small")),
        ("minimum", bound(min)),
        ("inclusive", json!(inclusive)),
        ("path", Value::Array(path.to_vec())),
        ("message", json!(format!("Too small: {}", describe(origin, op, min)))),
    ])
}

fn too_big(origin: &str, max: f64, inclusive: bool, path: &[Value]) -> Value {
    let op = if inclusive { "<=" } else { "<" };
    json_obj([
        ("origin", json!(origin)),
        ("code", json!("too_big")),
        ("maximum", bound(max)),
        ("inclusive", json!(inclusive)),
        ("path", Value::Array(path.to_vec())),
        ("message", json!(format!("Too big: {}", describe(origin, op, max)))),
    ])
}

fn format_issue(format: &str, pattern: &str, message: &str, path: &[Value]) -> Value {
    json_obj([
        ("origin", json!("string")),
        ("code", json!("invalid_format")),
        ("format", json!(format)),
        ("pattern", json!(pattern)),
        ("path", Value::Array(path.to_vec())),
        ("message", json!(message)),
    ])
}

/// zod's `stringifyPrimitive`.
fn prim(v: &Value) -> String {
    match v {
        Value::String(s) => oc_js::stringify_str(s),
        other => crate::http::jsjson::stringify(other),
    }
}

fn invalid_value(values: &[Value], path: &[Value]) -> Value {
    let message = if values.len() == 1 {
        format!("Invalid input: expected {}", prim(&values[0]))
    } else {
        format!("Invalid option: expected one of {}", values.iter().map(prim).collect::<Vec<_>>().join("|"))
    };
    json_obj([("code", json!("invalid_value")), ("values", Value::Array(values.to_vec())), ("path", Value::Array(path.to_vec())), ("message", json!(message))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues(s: &Schema, v: Value) -> String {
        crate::http::jsjson::stringify(&Value::Array(s.safe_parse(Some(&v)).unwrap_err()))
    }

    /// Expected strings printed by zod 4.6.5 (`schema.safeParse(v).error.issues`).
    #[test]
    fn matches_zod() {
        let signup = obj([
            ("email", str().email().max(200.0)),
            ("password", str().max(200.0)),
            ("name", str().min(1.0).max(80.0)),
            ("birthYear", num().int().min(1900.0).max(2026.0)),
            ("acceptTerms", literal(json!(true))),
        ]);
        assert_eq!(
            issues(&signup, json!({ "email": "x", "password": 5, "name": "", "birthYear": 1.5, "acceptTerms": false })),
            r#"[{"origin":"string","code":"invalid_format","format":"email","pattern":"/^(?:[A-Za-z0-9_'+\\-]+\\.)*[A-Za-z0-9_'+\\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\\-]*\\.)+[A-Za-z]{2,}$/","path":["email"],"message":"Invalid email address"},{"expected":"string","code":"invalid_type","path":["password"],"message":"Invalid input: expected string, received number"},{"origin":"string","code":"too_small","minimum":1,"inclusive":true,"path":["name"],"message":"Too small: expected string to have >=1 characters"},{"expected":"int","format":"safeint","code":"invalid_type","path":["birthYear"],"message":"Invalid input: expected int, received number"},{"code":"invalid_value","values":[true],"path":["acceptTerms"],"message":"Invalid input: expected true"}]"#
        );
        let q = obj([("filter", enumeration(&["recent", "starred"]).default(json!("recent"))), ("workspace", str().uuid().optional())]);
        assert_eq!(
            issues(&q, json!({ "filter": "zzz", "workspace": "nope" })),
            r#"[{"code":"invalid_value","values":["recent","starred"],"path":["filter"],"message":"Invalid option: expected one of \"recent\"|\"starred\""},{"origin":"string","code":"invalid_format","format":"uuid","pattern":"/^([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$/","path":["workspace"],"message":"Invalid UUID"}]"#
        );
        assert_eq!(q.safe_parse(Some(&json!({}))).unwrap(), json!({ "filter": "recent" }));
        let n = obj([("w", num().positive().max(20000.0).optional()), ("n", num().nonnegative()), ("s", num().min(0.1).max(4.0).default(json!(1)))]);
        assert_eq!(
            issues(&n, json!({ "w": -1, "n": -2, "s": 5 })),
            r#"[{"origin":"number","code":"too_small","minimum":0,"inclusive":false,"path":["w"],"message":"Too small: expected number to be >0"},{"origin":"number","code":"too_small","minimum":0,"inclusive":true,"path":["n"],"message":"Too small: expected number to be >=0"},{"origin":"number","code":"too_big","maximum":4,"inclusive":true,"path":["s"],"message":"Too big: expected number to be <=4"}]"#
        );
        let a = obj([("a", array(str()).max(1.0))]);
        assert_eq!(
            issues(&a, json!({ "a": [1, 2] })),
            // zod >= 4.6.6: the max check runs first and aborts, so the items are not parsed.
            r#"[{"origin":"array","code":"too_big","maximum":1,"inclusive":true,"path":["a"],"message":"Too big: expected array to have <=1 items"}]"#
        );
        let big = obj([("f", num().int().max(10.0))]);
        assert_eq!(
            issues(&big, json!({ "f": 1e300 })),
            r#"[{"code":"too_big","maximum":9007199254740991,"note":"Integers must be within the safe integer range.","origin":"int","inclusive":true,"path":["f"],"message":"Too big: expected int to be <=9007199254740991"},{"origin":"number","code":"too_big","maximum":10,"inclusive":true,"path":["f"],"message":"Too big: expected number to be <=10"}]"#
        );
        let lit = obj([("c", literal(json!("DELETE"))), ("e", enumeration(&["a"]))]);
        assert_eq!(
            issues(&lit, json!({ "c": "x", "e": "b" })),
            r#"[{"code":"invalid_value","values":["DELETE"],"path":["c"],"message":"Invalid input: expected \"DELETE\""},{"code":"invalid_value","values":["a"],"path":["e"],"message":"Invalid input: expected \"a\""}]"#
        );
        let re = obj([("c", str().regex("^#[0-9a-fA-F]{6}$", "^#[0-9a-fA-F]{6}$"))]);
        assert_eq!(
            issues(&re, json!({ "c": "x" })),
            r#"[{"origin":"string","code":"invalid_format","format":"regex","pattern":"/^#[0-9a-fA-F]{6}$/","path":["c"],"message":"Invalid string: must match pattern /^#[0-9a-fA-F]{6}$/"}]"#
        );
        assert_eq!(
            crate::http::jsjson::stringify(&Value::Array(obj([("x", str())]).safe_parse(None).unwrap_err())),
            r#"[{"expected":"object","code":"invalid_type","path":[],"message":"Invalid input: expected object, received undefined"}]"#
        );
        let nul = obj([("q", num().min(10.0).default(json!(90))), ("f", str().uuid().nullable().optional())]);
        assert_eq!(
            issues(&nul, json!({ "q": null, "f": null })),
            r#"[{"expected":"number","code":"invalid_type","path":["q"],"message":"Invalid input: expected number, received null"}]"#
        );
    }
}
