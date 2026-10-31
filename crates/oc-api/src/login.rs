//! Port of services/api/src/login.ts — finishing a sign-in, shared by every login method (password, magic link,
//! passkey, OIDC, Supabase bridge): the 2FA gate, the session cookie, the new-device email and the audit entry.
//! Also the public user shape and the open-redirect rule for `next` (plan 12 §C4).

use serde_json::{Map, Value, json};

use crate::auth::{UserFull, create_session, issue_token};
use crate::http::cookie;
use crate::http::error::{ApiError, ApiResult};
use crate::http::reply::{self, Reply};
use crate::http::req::Req;
use crate::state::AppState;

pub const TERMS_VERSION: &str = "2026-10-01";
pub const MIN_AGE: i64 = 13;
pub const DISABLED_MESSAGE: &str = "This account is disabled. Contact support if you think this is a mistake.";

/// `safePath(n)`: a same-site path to resume at after signing in. Accepted only when it starts with one `/` not
/// followed by `/` or `\`, and has no backslash, control character or whitespace; anything else becomes `/`.
pub fn safe_path(n: Option<&Value>) -> String {
    let Some(Value::String(n)) = n else { return "/".into() };
    safe_path_str(n)
}

pub fn safe_path_str(n: &str) -> String {
    if crate::js::len(n) > 2000 {
        return "/".into();
    }
    let mut it = n.chars();
    let ok_start = it.next() == Some('/') && !matches!(it.next(), Some('/' | '\\'));
    let bad = n.chars().any(|c| c == '\\' || crate::js::is_ws(c) || (c as u32) <= 0x1f || c as u32 == 0x7f);
    if ok_start && !bad { n.to_owned() } else { "/".into() }
}

/// `publicUser(u)`.
pub fn public_user(u: &UserFull) -> Value {
    json!({
        "id": u.id.to_string(), "email": u.email, "name": u.name, "color": u.color,
        "verified": u.email_verified_at.is_some(), "mfa": u.totp_secret.as_deref().is_some_and(|t| !t.is_empty()), "staff": u.is_staff, "locale": u.locale,
    })
}

/// `deviceLabel(ua)`.
pub fn device_label(ua: &str) -> String {
    let b = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if ua.contains("Mac OS X") {
        "macOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "iOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "unknown OS"
    };
    format!("{b} on {os}")
}

/// `audit({ actorId, action, target, meta, ip })` at the app clock.
pub async fn audit(s: &AppState, actor: Option<uuid::Uuid>, action: &str, target: Option<&str>, meta: Option<Value>, ip: &str) -> Result<(), ApiError> {
    oc_audit::audit_at(
        &s.pool,
        oc_audit::AuditEvent { actor_id: actor, action: action.into(), target: target.map(str::to_owned), meta, ip: Some(ip.to_owned()), ..Default::default() },
        s.now(),
    )
    .await?;
    Ok(())
}

/// The options of `completeLogin`.
#[derive(Default)]
pub struct LoginOpts {
    pub skip_mfa: bool,
    /// Merged into the JSON reply (e.g. `{ next }`).
    pub extra: Map<String, Value>,
    /// Merged into the `auth.login` audit entry's meta.
    pub audit_meta: Map<String, Value>,
}

fn merged(mut base: Map<String, Value>, extra: &Map<String, Value>) -> Value {
    for (k, v) in extra {
        base.insert(k.clone(), v.clone());
    }
    Value::Object(base)
}

/// `completeLogin(req, reply, u, method, opts)`: 2FA gate, session, new-device email, audit.
pub async fn complete_login(s: &AppState, req: &Req, u: &UserFull, method: &str, opts: LoginOpts) -> ApiResult {
    if u.disabled_at.is_some() {
        return Ok(reply::json(403, json!({ "error": "disabled", "message": DISABLED_MESSAGE })));
    }
    if u.totp_secret.as_deref().is_some_and(|t| !t.is_empty()) && !opts.skip_mfa {
        let ticket = issue_token(s, "mfa", Some(u.id), None, Some(json!({ "method": method })), 10).await?;
        let mut m = Map::new();
        m.insert("mfaRequired".into(), json!(true));
        m.insert("ticket".into(), json!(ticket));
        return Ok(Reply::json(merged(m, &opts.extra)));
    }
    let (set_cookie, new_device) = create_session(s, req, u.id, true).await?;
    let mut meta = Map::new();
    meta.insert("method".into(), json!(method));
    audit(s, Some(u.id), "auth.login", None, Some(merged(meta, &opts.audit_meta)), &req.ip).await?;
    if new_device {
        let n: Option<i32> = sqlx::query_scalar("SELECT count(*)::int AS n FROM sessions WHERE user_id = $1").bind(u.id).fetch_optional(&s.pool).await?;
        if n.unwrap_or(0) > 1 {
            let t = oc_mail::Template::NewLogin { device: device_label(&req.user_agent()), ip: req.ip.clone(), when: cookie::utc_string(s.now_ms()) };
            oc_mail::send_email(&s.pool, &u.email, &t, None).await?;
        }
    }
    let mut m = Map::new();
    m.insert("user".into(), public_user(u));
    Ok(Reply::json(merged(m, &opts.extra)).cookie(set_cookie))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths() {
        for (i, o) in [("/a?b=1", "/a?b=1"), ("//evil.com", "/"), ("/\\evil.com", "/"), ("javascript:x", "/"), ("/a b", "/"), ("/a\u{7f}", "/"), ("/", "/")] {
            assert_eq!(safe_path_str(i), o, "{i}");
        }
    }
}
