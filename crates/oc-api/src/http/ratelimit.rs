//! `@fastify/rate-limit` 11 with the in-memory store, as services/api registers it:
//!
//! * global: `{ max: 600, timeWindow: '1 minute' }` on every route without its own `config.rateLimit`;
//! * per route (`config: { rateLimit: { max, timeWindow } }`): its OWN counter (`store.child`) instead of
//!   the global one (a strict route is not also counted globally);
//! * the key is the client IP (`request.ip`, IPv6 collapsed to its /64), counted in the `onRequest` hook,
//!   i.e. before body parsing and authentication; unmatched URLs (the 404 handler) are not limited.
//!
//! Every limited response carries `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset`
//! (seconds, rounded up); a request over the limit also gets `retry-after` and
//! `429 {"error":"Rate limit exceeded, retry in 1 minute"}`.
//!
//! A route module declares its own limits in `pub const LIMITS: &[RouteLimit]`; `router.rs` collects them.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};

/// A route with its own `config.rateLimit`.
#[derive(Debug, Clone, Copy)]
pub struct RouteLimit {
    pub method: &'static str,
    /// The route path exactly as registered with axum (`/api/auth/login`, `/api/designs/{id}`).
    pub path: &'static str,
    pub max: u32,
    pub window_ms: i64,
}

pub const MINUTE: i64 = 60_000;

/// `{ config: { rateLimit: { max: 10, timeWindow: '1 minute' } } }` (the auth routes' `strict`).
pub const fn strict(method: &'static str, path: &'static str) -> RouteLimit {
    RouteLimit { method, path, max: 10, window_ms: MINUTE }
}

pub const GLOBAL_MAX: u32 = 600;
const CACHE: usize = 5000;

#[derive(Debug, Clone, Copy)]
struct Entry {
    current: u32,
    ttl: i64,
    start: i64,
    used: u64,
}

/// One `LocalStore` (an LRU of 5000 keys).
pub struct Limiter {
    pub max: u32,
    pub window_ms: i64,
    store: Mutex<(HashMap<String, Entry>, u64)>,
}

/// The outcome of counting one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Count {
    pub max: u32,
    pub remaining: u32,
    pub ttl_seconds: i64,
    pub exceeded: bool,
}

impl Limiter {
    pub fn new(max: u32, window_ms: i64) -> Limiter {
        Limiter { max, window_ms, store: Mutex::new((HashMap::new(), 0)) }
    }

    /// `LocalStore.incr` + the handler's arithmetic.
    pub fn incr(&self, key: &str, now: i64) -> Count {
        let mut guard = self.store.lock().expect("rate limit lock");
        let (map, tick) = &mut *guard;
        *tick += 1;
        let e = match map.get(key).copied() {
            None => Entry { current: 1, ttl: self.window_ms, start: now, used: *tick },
            Some(e) if e.start + self.window_ms <= now => Entry { current: 1, ttl: self.window_ms, start: now, used: *tick },
            Some(e) => Entry { current: e.current + 1, ttl: self.window_ms - (now - e.start), start: e.start, used: *tick },
        };
        if !map.contains_key(key)
            && map.len() >= CACHE
            && let Some(oldest) = map.iter().min_by_key(|(_, v)| v.used).map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
        map.insert(key.to_owned(), e);
        Count {
            max: self.max,
            remaining: self.max.saturating_sub(e.current),
            ttl_seconds: (e.ttl as f64 / 1000.0).ceil() as i64,
            exceeded: e.current > self.max,
        }
    }
}

/// The global limiter plus every route's own one.
pub struct RateLimits {
    pub global: Arc<Limiter>,
    routes: HashMap<(String, String), Arc<Limiter>>,
}

impl RateLimits {
    pub fn new(route_limits: &[RouteLimit]) -> RateLimits {
        let routes = route_limits.iter().map(|l| ((l.method.to_owned(), l.path.to_owned()), Arc::new(Limiter::new(l.max, l.window_ms)))).collect();
        RateLimits { global: Arc::new(Limiter::new(GLOBAL_MAX, MINUTE)), routes }
    }

    /// The limiter that counts `method path` (the matched route pattern).
    pub fn for_route(&self, method: &str, path: &str) -> &Arc<Limiter> {
        // Fastify registers HEAD for every GET route; it shares the GET route's config.
        let m = if method == "HEAD" { "GET" } else { method };
        self.routes.get(&(m.to_owned(), path.to_owned())).unwrap_or(&self.global)
    }
}

/// `normalizeIP(ip)`: IPv4 lower-cased as is, IPv4-mapped IPv6 as IPv4, other IPv6 as the start of its /64.
pub fn normalize_ip(ip: &str) -> String {
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.to_string();
            }
            let seg = v6.segments();
            Ipv6Addr::new(seg[0], seg[1], seg[2], seg[3], 0, 0, 0, 0).to_string()
        }
        _ => ip.to_lowercase(),
    }
}

/// `@lukeed/ms` `format(ms, true)`.
pub fn format_ms(ms: i64) -> String {
    const SEC: f64 = 1e3;
    const MIN: f64 = SEC * 60.0;
    const HOUR: f64 = MIN * 60.0;
    const DAY: f64 = HOUR * 24.0;
    const YEAR: f64 = DAY * 365.25;
    let n = ms as f64;
    let (pfx, abs) = if n < 0.0 { ("-", -n) } else { ("", n) };
    let fmt = |val: f64, unit: &str| {
        let num = if val.fract() == 0.0 { val } else { (val + 0.5).trunc() };
        format!("{pfx}{} {unit}{}", oc_js::number_to_string(num), if num != 1.0 { "s" } else { "" })
    };
    if abs < SEC {
        format!("{ms} ms")
    } else if abs < MIN {
        fmt(abs / SEC, "second")
    } else if abs < HOUR {
        fmt(abs / MIN, "minute")
    } else if abs < DAY {
        fmt(abs / HOUR, "hour")
    } else if abs < YEAR {
        fmt(abs / DAY, "day")
    } else {
        fmt(abs / YEAR, "year")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_like_local_store() {
        let l = Limiter::new(2, MINUTE);
        assert_eq!(l.incr("a", 1000), Count { max: 2, remaining: 1, ttl_seconds: 60, exceeded: false });
        assert_eq!(l.incr("a", 1500), Count { max: 2, remaining: 0, ttl_seconds: 60, exceeded: false });
        assert_eq!(l.incr("a", 31_000), Count { max: 2, remaining: 0, ttl_seconds: 30, exceeded: true });
        assert_eq!(l.incr("a", 61_000), Count { max: 2, remaining: 1, ttl_seconds: 60, exceeded: false });
    }

    #[test]
    fn formats() {
        assert_eq!(format_ms(60_000), "1 minute");
        assert_eq!(format_ms(30_000), "30 seconds");
        assert_eq!(format_ms(600_000), "10 minutes");
        assert_eq!(normalize_ip("::ffff:10.0.0.1"), "10.0.0.1");
        assert_eq!(normalize_ip("2001:db8:1:2:3:4:5:6"), "2001:db8:1:2::");
    }
}
