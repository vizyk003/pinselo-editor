//! Port of services/api/src/print/store.ts — print order persistence: rows, status changes (state machine + history
//! + customer emails), signed print-file links.

use std::sync::LazyLock;

use hmac::{Hmac, Mac};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::Sha256;
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::pg;
use crate::print::states::{TransitionError, assert_transition};
use crate::state::AppState;

/// An order row (`SELECT * FROM print_orders`) as node-postgres returns it.
pub type Order = Map<String, Value>;

/// Field access on an order row.
pub trait OrderExt {
    fn str_of(&self, k: &str) -> Option<&str>;
    fn num_of(&self, k: &str) -> f64;
    fn text(&self, k: &str) -> String {
        self.str_of(k).unwrap_or("").to_owned()
    }
    fn id(&self) -> String {
        self.text("id")
    }
    fn status(&self) -> String {
        self.text("status")
    }
}

impl OrderExt for Order {
    fn str_of(&self, k: &str) -> Option<&str> {
        self.get(k).and_then(Value::as_str)
    }
    fn num_of(&self, k: &str) -> f64 {
        self.get(k).and_then(Value::as_f64).unwrap_or(0.0)
    }
}

/// `setStatus` failures: an illegal transition (409 in the routes) or anything else.
#[derive(Debug)]
pub enum StatusError {
    Transition(TransitionError),
    Api(ApiError),
}

impl From<ApiError> for StatusError {
    fn from(e: ApiError) -> Self {
        StatusError::Api(e)
    }
}

impl From<sqlx::Error> for StatusError {
    fn from(e: sqlx::Error) -> Self {
        StatusError::Api(e.into())
    }
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusError::Transition(t) => write!(f, "{t}"),
            StatusError::Api(ApiError::Internal(m)) => write!(f, "{m}"),
            StatusError::Api(e) => write!(f, "{e:?}"),
        }
    }
}

static ORDER_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f-]{36}$").expect("regex"));

fn obj(v: Option<Value>) -> Option<Order> {
    match v {
        Some(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// `getOrder(id)`.
pub async fn get_order(s: &AppState, id: &str) -> Result<Option<Order>, ApiError> {
    if !ORDER_ID.is_match(id) {
        return Ok(None);
    }
    Ok(obj(pg::json_one(&s.pool, "SELECT * FROM print_orders WHERE id = $1::uuid", &[pg::text(id)]).await?))
}

/// `getOrderByNumber(n)`.
pub async fn get_order_by_number(s: &AppState, n: &str) -> Result<Option<Order>, ApiError> {
    Ok(obj(pg::json_one(&s.pool, "SELECT * FROM print_orders WHERE number = $1", &[pg::text(n)]).await?))
}

/// Customer-facing order number, e.g. OC-7KX4M2QD (no ambiguous characters; ≤ 32 chars for fulfillers).
pub fn order_number() -> String {
    const A: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    let r = crate::auth::random_bytes(8);
    let s: String = r.iter().map(|b| A[usize::from(*b) * A.len() / 256] as char).collect();
    format!("OC-{s}")
}

const EMAILED: [&str; 6] = ["in_production", "shipped", "delivered", "failed", "cancelled", "returned"];

/// `new Intl.NumberFormat('en', { style: 'currency', currency }).format(cents / 100)`.
pub fn format_money(cents: f64, currency: &str) -> String {
    let code = currency.to_uppercase();
    let (prefix, digits) = match code.as_str() {
        "USD" => ("$".to_owned(), 2),
        "EUR" => ("€".to_owned(), 2),
        "GBP" => ("£".to_owned(), 2),
        "JPY" => ("¥".to_owned(), 0),
        "CAD" => ("CA$".to_owned(), 2),
        "AUD" => ("A$".to_owned(), 2),
        "NZD" => ("NZ$".to_owned(), 2),
        "INR" => ("₹".to_owned(), 2),
        "KRW" => ("₩".to_owned(), 0),
        "BRL" => ("R$".to_owned(), 2),
        "MXN" => ("MX$".to_owned(), 2),
        "HKD" => ("HK$".to_owned(), 2),
        "ILS" => ("₪".to_owned(), 2),
        c if c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()) => (format!("{c}\u{a0}"), 2),
        _ => return format!("{} {code}", js_to_fixed(cents / 100.0, 2)),
    };
    let v = cents / 100.0;
    let neg = v < 0.0;
    let fixed = js_to_fixed(v.abs(), digits);
    let (int, frac) = fixed.split_once('.').map_or((fixed.as_str(), None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let body = match frac {
        Some(f) => format!("{grouped}.{f}"),
        None => grouped,
    };
    format!("{}{prefix}{body}", if neg { "-" } else { "" })
}

/// `n.toFixed(d)` for the amounts money formatting sees.
pub fn js_to_fixed(n: f64, d: usize) -> String {
    format!("{n:.d$}")
}

/// A status change (`StatusChange`).
#[derive(Default)]
pub struct StatusChange {
    pub note: Option<String>,
    pub source: &'static str,
    pub actor_id: Option<Uuid>,
    pub patch: Patch,
    /// Don't email the customer (bulk/staff corrections).
    pub quiet: bool,
}

/// The columns a status change may also set (`undefined` = keep).
#[derive(Default, Clone)]
pub struct Patch {
    pub provider: Option<String>,
    pub provider_order_id: Option<String>,
    pub error: Option<String>,
    pub file_sha: Option<String>,
    pub file_type: Option<String>,
    pub payment_ref: Option<String>,
    pub payment_provider: Option<String>,
    pub refunded: Option<bool>,
    pub tracking: Option<Vec<Value>>,
}

fn merge_tracking(a: &[Value], b: &[Value]) -> Vec<Value> {
    let mut out = a.to_vec();
    let truthy = |v: &Value| matches!(v, Value::String(s) if !s.is_empty()) || matches!(v, Value::Number(_) | Value::Bool(true) | Value::Object(_) | Value::Array(_));
    for t in b {
        let dup = out.iter().any(|x| (truthy(&x["code"]) && x.get("code") == t.get("code")) || (truthy(&x["url"]) && x.get("url") == t.get("url")));
        if !dup {
            out.push(t.clone());
        }
    }
    out
}

/// `[t.carrier, t.code, t.url].filter(Boolean).join(' ')` per entry, one per line.
fn tracking_text(tracking: &Value) -> String {
    tracking
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| {
            ["carrier", "code", "url"]
                .iter()
                .filter_map(|k| match &t[*k] {
                    Value::String(s) if !s.is_empty() => Some(s.clone()),
                    Value::Number(n) => Some(crate::http::jsjson::number_text(n)),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mail_status(to: &str) -> Option<oc_mail::PrintMailStatus> {
    use oc_mail::PrintMailStatus as P;
    Some(match to {
        "in_production" => P::InProduction,
        "shipped" => P::Shipped,
        "delivered" => P::Delivered,
        "failed" => P::Failed,
        "cancelled" => P::Cancelled,
        "returned" => P::Returned,
        "submitted" => P::Submitted,
        _ => return None,
    })
}

/// `setStatus(id, to, change)`: move an order to `to` (state machine checked), record the history entry and email the
/// customer for the statuses they care about. Returns the updated row.
pub async fn set_status(s: &AppState, id: &str, to: &str, ch: StatusChange) -> Result<Order, StatusError> {
    let mut t = s.pool.begin().await?;
    let Some(cur) = obj(pg::json_one(&mut *t, "SELECT * FROM print_orders WHERE id = $1::uuid FOR UPDATE", &[pg::text(id)]).await?) else {
        return Err(StatusError::Api(ApiError::internal("order not found")));
    };
    let from = cur.status();
    if from != to {
        assert_transition(&from, to).map_err(StatusError::Transition)?;
    }
    let p = ch.patch;
    let cur_tracking: Vec<Value> = cur.get("tracking").and_then(Value::as_array).cloned().unwrap_or_default();
    let tracking = match &p.tracking {
        Some(tr) if !tr.is_empty() => merge_tracking(&cur_tracking, tr),
        _ => cur_tracking,
    };
    let row = obj(pg::json_one(
        &mut *t,
        "UPDATE print_orders SET status = $2, provider = COALESCE($3, provider), provider_order_id = COALESCE($4, provider_order_id),
         error = CASE WHEN $2 = 'failed' THEN COALESCE($5, error) ELSE $5 END, file_sha = COALESCE($6, file_sha), file_type = COALESCE($7, file_type),
         payment_ref = COALESCE($8, payment_ref), payment_provider = COALESCE($9, payment_provider), refunded = COALESCE($10, refunded),
         tracking = $11, paid_at = CASE WHEN $2 = 'paid' AND paid_at IS NULL THEN now() ELSE paid_at END, updated_at = now()
       WHERE id = $1::uuid RETURNING *",
        &[
            pg::text(id),
            pg::text(to),
            pg::opt_text(p.provider),
            pg::opt_text(p.provider_order_id),
            pg::opt_text(p.error),
            pg::opt_text(p.file_sha),
            pg::opt_text(p.file_type),
            pg::opt_text(p.payment_ref),
            pg::opt_text(p.payment_provider),
            pg::Arg::Bool(p.refunded),
            pg::Arg::Json(Some(Value::Array(tracking))),
        ],
    )
    .await?)
    .ok_or_else(|| ApiError::internal("order not found"))?;
    let changed = from != to;
    let note = ch.note.clone().unwrap_or_default();
    if changed || !note.is_empty() {
        pg::exec(
            &mut *t,
            "INSERT INTO print_order_events(order_id, status, note, source, actor_id) VALUES ($1::uuid,$2,$3,$4,$5)",
            &[pg::text(id), pg::text(to), pg::text(&note), pg::text(ch.source), pg::Arg::Uuid(ch.actor_id)],
        )
        .await?;
    }
    t.commit().await?;
    if changed
        && !ch.quiet
        && EMAILED.contains(&to)
        && let Some(status) = mail_status(to)
    {
        let o = &row;
        let tpl = oc_mail::Template::PrintStatus {
            number: o.text("number"),
            status,
            tracking: tracking_text(o.get("tracking").unwrap_or(&Value::Null)),
            note: if to == "failed" || to == "cancelled" { note } else { String::new() },
            url: format!("{}/orders/{}", s.cfg.web_origin, o.id()),
        };
        let uid = o.str_of("user_id").and_then(|u| Uuid::parse_str(u).ok());
        let _ = oc_mail::send_email(&s.pool, &o.text("email"), &tpl, uid).await;
    }
    Ok(row)
}

/// Record a note (no status change).
pub async fn add_note(s: &AppState, id: &str, status: &str, note: &str, source: &str, actor_id: Option<Uuid>) -> Result<(), ApiError> {
    pg::exec(
        &s.pool,
        "INSERT INTO print_order_events(order_id, status, note, source, actor_id) VALUES ($1::uuid,$2,$3,$4,$5)",
        &[pg::text(id), pg::text(status), pg::text(note), pg::text(source), pg::Arg::Uuid(actor_id)],
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------- signed print-file links (fulfillers, operator email)

fn sign(s: &AppState, id: &str, exp: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(s.cfg.secret.as_bytes()).expect("hmac key");
    mac.update(format!("print-file:{id}:{exp}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// `signedFileUrl(id, ext, ttlDays = 30)`.
pub fn signed_file_url(s: &AppState, id: &str, ext: &str) -> String {
    let exp = (s.now_ms() as f64 / 1000.0).floor() as i64 + 30 * 86400;
    format!("{}/api/print/files/{id}.{ext}?exp={exp}&sig={}", s.cfg.web_origin, sign(s, id, &exp.to_string()))
}

/// `Buffer.from(s, 'hex')`: pairs of hex digits up to the first invalid one.
fn node_hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let (Some(h), Some(l)) = ((b[i] as char).to_digit(16), (b[i + 1] as char).to_digit(16)) else { break };
        out.push((h * 16 + l) as u8);
        i += 2;
    }
    out
}

/// `verifyFileSignature(id, Number(exp), sig)`.
pub fn verify_file_signature(s: &AppState, id: &str, exp: f64, sig: &str) -> bool {
    // `!(exp > now)`: also false for NaN.
    if exp.partial_cmp(&(s.now_ms() as f64 / 1000.0)) != Some(std::cmp::Ordering::Greater) {
        return false;
    }
    let want = node_hex(&sign(s, id, &oc_js::number_to_string(exp)));
    let got = node_hex(sig);
    got.len() == want.len() && subtle::ConstantTimeEq::ct_eq(got.as_slice(), want.as_slice()).into()
}

/// `customerView(o)`: no file hash or payment reference, a `hasFile` flag instead.
pub fn customer_view(o: &Order) -> Value {
    let mut m = o.clone();
    let has = !matches!(m.get("file_sha"), None | Some(Value::Null)) && m.get("file_sha").and_then(Value::as_str) != Some("");
    m.shift_remove("file_sha");
    m.shift_remove("payment_ref");
    m.insert("hasFile".into(), json!(has));
    Value::Object(m)
}
