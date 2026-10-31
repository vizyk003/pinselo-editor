//! Port of services/api/src/insights.ts — insights for published websites and public view links (WS-010). No
//! cookies, no third-party trackers, no external geo lookups:
//!  - unique visitors = a hash of IP + user agent with a salt that changes every day (and differs per design), so
//!    visitors can't be followed across days or sites; the hashes are pruned after a day;
//!  - country comes from a CDN country header when one is present, else the Accept-Language region;
//!  - Do Not Track / Global Privacy Control requests are not counted at all;
//!  - only per-day aggregates are stored.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

use axum::http::HeaderMap;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::state::AppState;

/// `'site' | 'view'`.
pub type Scope = &'static str;

/// `h(headers, name)`: the first value of a header, `''` when absent.
pub fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

/// `doNotTrack(headers)`: `DNT: 1` or `Sec-GPC: 1`.
pub fn do_not_track(headers: &HeaderMap) -> bool {
    crate::js::trim(header(headers, "dnt")) == "1" || crate::js::trim(header(headers, "sec-gpc")) == "1"
}

static BOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)bot|crawl|spider|slurp|preview|facebookexternalhit|headless|lighthouse|curl|wget|python-requests|httpclient").expect("regex"));
static TABLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)ipad|tablet|kindle|silk|playbook").expect("regex"));
static MOBILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)mobi|iphone|ipod|android|blackberry|opera mini|iemobile").expect("regex"));
static LANG_REGION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[a-z]{2,3}(?:-[A-Za-z]{4})?-([A-Za-z]{2})(?-u:\b)").expect("regex"));

pub fn is_bot(ua: &str) -> bool {
    ua.is_empty() || BOT.is_match(ua)
}

/// `/android(?!.*mobile)/i`: an "android" not followed (on the same line) by "mobile".
fn android_not_mobile(ua: &str) -> bool {
    let lower = ua.to_lowercase();
    lower.match_indices("android").any(|(i, _)| {
        let rest = &lower[i + "android".len()..];
        let line = rest.split(['\n', '\r', '\u{2028}', '\u{2029}']).next().unwrap_or("");
        !line.contains("mobile")
    })
}

pub fn device_of(ua: &str) -> &'static str {
    if TABLET.is_match(ua) || android_not_mobile(ua) {
        "tablet"
    } else if MOBILE.is_match(ua) {
        "mobile"
    } else {
        "desktop"
    }
}

/// ISO country code from a proxy/CDN header, else the Accept-Language region; `''` when unknown.
pub fn country_of(headers: &HeaderMap) -> String {
    for name in ["cf-ipcountry", "x-vercel-ip-country", "cloudfront-viewer-country", "x-country-code", "x-geo-country"] {
        let v = crate::js::trim(header(headers, name)).to_uppercase();
        if v.len() == 2 && v.bytes().all(|c| c.is_ascii_uppercase()) && v != "XX" && v != "T1" {
            return v;
        }
    }
    for part in header(headers, "accept-language").split(',') {
        if let Some(c) = LANG_REGION.captures(part) {
            return c[1].to_uppercase();
        }
    }
    String::new()
}

/// Referring site (hostname without `www.`), `''` for direct visits and internal navigation.
pub fn referrer_of(referer: &str, own_hosts: &[String]) -> String {
    if referer.is_empty() {
        return String::new();
    }
    let Ok(u) = url::Url::parse(referer) else { return String::new() };
    let host = u.host_str().unwrap_or("").to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    if own_hosts.iter().any(|o| {
        let o = o.to_lowercase();
        o.strip_prefix("www.").unwrap_or(&o) == host
    }) {
        String::new()
    } else {
        host
    }
}

/// `utcDay(d)`: `YYYY-MM-DD` of an epoch-ms instant.
pub fn utc_day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// Daily-salted visitor hash. The salt is derived from the server secret and the day.
pub fn visitor_hash(secret: &str, day: &str, design_id: &str, scope: &str, ip: &str, ua: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("insights:{day}").as_bytes());
    let salt = mac.finalize().into_bytes();
    let mut h = Sha256::new();
    h.update(salt);
    h.update(format!("{scope}|{design_id}|{ip}|{ua}").as_bytes());
    let s = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize());
    s[..22].to_owned()
}

/// What a page view contributes, or `None` when it must not be counted (DNT/GPC, bots).
pub fn visit_facts(headers: &HeaderMap, own_hosts: &[String]) -> Option<(Vec<(&'static str, String)>, String)> {
    let ua = header(headers, "user-agent").to_owned();
    if do_not_track(headers) || is_bot(&ua) {
        return None;
    }
    let facts = vec![("view", String::new()), ("referrer", referrer_of(header(headers, "referer"), own_hosts)), ("country", country_of(headers)), ("device", device_of(&ua).to_owned())];
    Some((facts, ua))
}

/// Validate a client beacon event; links are kept as host + path (no query strings or fragments).
pub fn clean_event(e: &Value) -> Option<(&'static str, String)> {
    let Value::Object(o) = e else { return None };
    let Some(Value::String(v)) = o.get("v") else { return None };
    let stripped: String = v.chars().filter(|c| (*c as u32) > 0x1f).collect();
    let val = crate::js::trim(&stripped);
    match o.get("k").and_then(Value::as_str) {
        Some(k @ ("section" | "form")) => {
            if val.is_empty() {
                None
            } else {
                Some((if k == "section" { "section" } else { "form" }, crate::js::take(val, 80)))
            }
        }
        Some("click") => {
            let u = url::Url::parse(val).ok()?;
            let scheme = u.scheme();
            if !matches!(scheme, "http" | "https" | "mailto" | "tel") {
                return None;
            }
            let key = if scheme.starts_with("http") {
                let host = u.host_str().unwrap_or("");
                let host = host.strip_prefix("www.").unwrap_or(host);
                let path = u.path();
                format!("{host}{}", if path == "/" { "" } else { path })
            } else {
                scheme.to_owned()
            };
            Some(("click", crate::js::take(&key, 120)))
        }
        _ => None,
    }
}

/// `String.prototype.localeCompare` on ASCII keys (ICU root collation: punctuation, then digits, then letters
/// compared case-insensitively; lowercase before uppercase on a tie). Non-ASCII keys compare by code point.
fn locale_cmp(a: &str, b: &str) -> Ordering {
    fn primary(c: char) -> (u8, char) {
        if c.is_ascii_digit() {
            (1, c)
        } else if c.is_ascii_alphabetic() {
            (2, c.to_ascii_lowercase())
        } else if c.is_ascii() {
            (0, c)
        } else {
            (3, c)
        }
    }
    let pa: Vec<(u8, char)> = a.chars().map(primary).collect();
    let pb: Vec<(u8, char)> = b.chars().map(primary).collect();
    pa.cmp(&pb).then_with(|| {
        let ta: Vec<bool> = a.chars().map(|c| c.is_ascii_uppercase()).collect();
        let tb: Vec<bool> = b.chars().map(|c| c.is_ascii_uppercase()).collect();
        ta.cmp(&tb)
    })
}

/// `Date.parse('YYYY-MM-DDT00:00:00Z')` of a `YYYY-MM-DD` string (V8 accepts days 1–31 in any month and rolls
/// over); `None` when invalid.
pub fn parse_day(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !s.bytes().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return None;
    }
    let (y, m, d): (i32, u32, u32) = (s[..4].parse().ok()?, s[5..7].parse().ok()?, s[8..].parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let first = chrono::NaiveDate::from_ymd_opt(y, m, 1)?;
    Some(first.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() + i64::from(d - 1) * 86_400_000)
}

fn days(from: &str, to: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (Some(start), Some(end)) = (parse_day(from), parse_day(to)) else { return out };
    let mut t = start;
    while t <= end && out.len() < 400 {
        out.push(utc_day(t));
        t += 86_400_000;
    }
    out
}

/// One `insight_daily` row.
pub struct DailyRow {
    pub day: String,
    pub kind: String,
    pub key: String,
    pub n: i64,
}

/// Daily rows → totals, a zero-filled daily series and top lists. Visitors over a range are the sum of daily
/// unique visitors (the daily salt makes cross-day de-duplication impossible by design).
pub fn summarize(rows: &[DailyRow], from: &str, to: &str, limit: usize) -> Value {
    let order = days(from, to);
    let mut by_day: HashMap<String, (i64, i64)> = order.iter().map(|d| (d.clone(), (0, 0))).collect();
    let kinds = ["referrer", "country", "device", "section", "click", "form"];
    let mut tops: HashMap<&str, Vec<(String, i64)>> = kinds.iter().map(|k| (*k, Vec::new())).collect();
    let (mut views, mut visitors, mut clicks, mut forms) = (0i64, 0i64, 0i64, 0i64);
    for r in rows {
        if r.day.as_str() < from || r.day.as_str() > to {
            continue;
        }
        let n = r.n;
        match r.kind.as_str() {
            "view" => {
                views += n;
                if let Some(d) = by_day.get_mut(&r.day) {
                    d.0 += n;
                }
                continue;
            }
            "visitor" => {
                visitors += n;
                if let Some(d) = by_day.get_mut(&r.day) {
                    d.1 += n;
                }
                continue;
            }
            "click" => clicks += n,
            "form" => forms += n,
            _ => {}
        }
        if let Some(m) = tops.get_mut(r.kind.as_str()) {
            match m.iter_mut().find(|(k, _)| *k == r.key) {
                Some(e) => e.1 += n,
                None => m.push((r.key.clone(), n)),
            }
        }
    }
    let top = |k: &str| -> Value {
        let mut v = tops[k].clone();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_cmp(&a.0, &b.0)));
        Value::Array(v.into_iter().take(limit).map(|(key, n)| json!({ "key": key, "n": n })).collect())
    };
    let series: Vec<Value> = order.iter().map(|d| json!({ "day": d, "views": by_day[d].0, "visitors": by_day[d].1 })).collect();
    let mut out = Map::new();
    out.insert("from".into(), json!(from));
    out.insert("to".into(), json!(to));
    out.insert("totals".into(), json!({ "views": views, "visitors": visitors, "clicks": clicks, "forms": forms }));
    out.insert("series".into(), Value::Array(series));
    out.insert("referrers".into(), top("referrer"));
    out.insert("countries".into(), top("country"));
    out.insert("devices".into(), top("device"));
    out.insert("sections".into(), top("section"));
    out.insert("clicks".into(), top("click"));
    out.insert("forms".into(), top("form"));
    Value::Object(out)
}

// ------------------------------------------------------------------ storage

async fn bump(state: &AppState, design_id: &str, scope: Scope, day: &str, facts: &[(&'static str, String)]) -> Result<(), sqlx::Error> {
    if facts.is_empty() {
        return Ok(());
    }
    let kinds: Vec<String> = facts.iter().map(|f| f.0.to_owned()).collect();
    let keys: Vec<String> = facts.iter().map(|f| f.1.clone()).collect();
    sqlx::query(
        "INSERT INTO insight_daily(design_id, scope, day, kind, key, n)
     SELECT $1, $2, $3::date, k, v, count(*)::int FROM unnest($4::text[], $5::text[]) AS t(k, v) GROUP BY k, v
     ON CONFLICT (design_id, scope, day, kind, key) DO UPDATE SET n = insight_daily.n + EXCLUDED.n",
    )
    .bind(design_id)
    .bind(scope)
    .bind(day)
    .bind(&kinds)
    .bind(&keys)
    .execute(&state.pool)
    .await
    .map(|_| ())
}

/// Count a page view (no-op for DNT/GPC and bots). Returns whether it was counted.
pub async fn record_visit(state: &AppState, scope: Scope, design_id: &str, headers: &HeaderMap, ip: &str, own_hosts: &[String]) -> Result<bool, sqlx::Error> {
    let Some((mut facts, ua)) = visit_facts(headers, own_hosts) else { return Ok(false) };
    let day = utc_day(state.now_ms());
    let fresh = sqlx::query("INSERT INTO insight_visitors(design_id, scope, day, hash) VALUES ($1,$2,$3::date,$4) ON CONFLICT DO NOTHING RETURNING 1")
        .bind(design_id)
        .bind(scope)
        .bind(&day)
        .bind(visitor_hash(&state.cfg.secret, &day, design_id, scope, ip, &ua))
        .fetch_all(&state.pool)
        .await?;
    if !fresh.is_empty() {
        facts.push(("visitor", String::new()));
    }
    bump(state, design_id, scope, &day, &facts).await?;
    // `Math.random() < 0.01`: prune hashes from before yesterday now and then, by the same clock that stamped `day`.
    if oc_db::uuid().as_bytes()[0] < 3 {
        let pool = state.pool.clone();
        state.spawn(async move {
            let _ = sqlx::query("DELETE FROM insight_visitors WHERE day < $1::date - 1").bind(day).execute(&pool).await;
        });
    }
    Ok(true)
}

/// Client beacon events (sections seen, outbound clicks, form submissions); at most 50 per request.
pub async fn record_events(state: &AppState, design_id: &str, events: &[Value]) -> Result<usize, sqlx::Error> {
    let facts: Vec<(&'static str, String)> = events.iter().take(50).filter_map(clean_event).collect();
    bump(state, design_id, "site", &utc_day(state.now_ms()), &facts).await?;
    Ok(facts.len())
}

pub async fn load_summary(state: &AppState, design_id: &str, scope: Scope, from: &str, to: &str) -> Result<Value, sqlx::Error> {
    let rows: Vec<(String, String, String, i32)> = sqlx::query_as(
        "SELECT to_char(day, 'YYYY-MM-DD') AS day, kind, key, n FROM insight_daily WHERE design_id = $1 AND scope = $2 AND day BETWEEN $3::date AND $4::date",
    )
    .bind(design_id)
    .bind(scope)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;
    let rows: Vec<DailyRow> = rows.into_iter().map(|(day, kind, key, n)| DailyRow { day, kind, key, n: i64::from(n) }).collect();
    Ok(summarize(&rows, from, to, 10))
}

/// Beacon script injected into served sites (only when analytics are on); it also honours DNT/GPC in the browser.
pub fn beacon_script(design_id: &str) -> String {
    let url = oc_js::stringify_str(&format!("/api/sites/{}/e", crate::js::encode_uri_component(design_id)));
    format!(
        "<script>(function(){{var n=navigator;if(n.doNotTrack==='1'||window.doNotTrack==='1'||n.globalPrivacyControl)return;var u={url},q=[],seen={{}};
function f(){{if(!q.length)return;var b=JSON.stringify({{e:q.splice(0,50)}});try{{if(!(n.sendBeacon&&n.sendBeacon(u,b)))fetch(u,{{method:'POST',body:b,keepalive:true,headers:{{'content-type':'text/plain'}}}})}}catch(e){{}}}}
window.__ocTrack=function(k,v){{q.push({{k:k,v:String(v).slice(0,300)}});if(k!=='section')f()}};
if('IntersectionObserver' in window){{var io=new IntersectionObserver(function(es){{es.forEach(function(e){{var t=e.target;if(seen[t.id]||!e.isIntersecting)return;if(e.intersectionRatio>=0.5||e.intersectionRect.height>=innerHeight*0.5){{seen[t.id]=1;__ocTrack('section',t.getAttribute('aria-label')||t.id)}}}})}},{{threshold:[0,0.25,0.5,0.75]}});document.querySelectorAll('main>section').forEach(function(s){{io.observe(s)}})}}
document.addEventListener('click',function(e){{var a=e.target&&e.target.closest&&e.target.closest('a[href]');if(a&&(a.protocol==='mailto:'||a.protocol==='tel:'||(a.host&&a.host!==location.host)))__ocTrack('click',a.href)}},true);
addEventListener('visibilitychange',function(){{if(document.visibilityState==='hidden')f()}});addEventListener('pagehide',f);setTimeout(f,4000)}})();</script>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies() {
        assert!(is_bot(""));
        assert!(is_bot("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert_eq!(device_of("Mozilla/5.0 (Linux; Android 14; Pixel 8) Mobile Safari"), "mobile");
        assert_eq!(device_of("Mozilla/5.0 (Linux; Android 14; SM-X900) Safari"), "tablet");
        assert_eq!(device_of("Mozilla/5.0 (iPad; CPU OS 17_0)"), "tablet");
        assert_eq!(device_of("Mozilla/5.0 (Macintosh)"), "desktop");
        let mut h = HeaderMap::new();
        h.insert("accept-language", "en-GB,en;q=0.9".parse().unwrap());
        assert_eq!(country_of(&h), "GB");
        h.insert("cf-ipcountry", "xx".parse().unwrap());
        assert_eq!(country_of(&h), "GB");
        h.insert("x-country-code", "de".parse().unwrap());
        assert_eq!(country_of(&h), "DE");
        assert_eq!(referrer_of("https://www.Example.com/a", &["app.example".into()]), "example.com");
        assert_eq!(referrer_of("https://app.example/x", &["app.example".into()]), "");
        assert_eq!(clean_event(&json!({ "k": "click", "v": "https://www.x.test/a?b=1" })), Some(("click", "x.test/a".into())));
        assert_eq!(clean_event(&json!({ "k": "click", "v": "mailto:a@b.c" })), Some(("click", "mailto".into())));
        assert_eq!(clean_event(&json!({ "k": "click", "v": "ftp://x" })), None);
        assert_eq!(clean_event(&json!({ "k": "section", "v": " \u{1}Hero " })), Some(("section", "Hero".into())));
        assert_eq!(parse_day("2025-02-30"), parse_day("2025-03-02"));
        assert_eq!(parse_day("2025-02-32"), None);
    }
}
