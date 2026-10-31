//! Sign-up control (plan 15 §5): `OC_SIGNUP=invite` limits who can create an account. Existing accounts never come
//! through here, so call [`may_create`] only once the account is known to be new.

use crate::prelude::*;

const CLOSED_MESSAGE: &str = "New accounts are by invitation only. Use the email address your invitation was sent to.";

/// The refusal's body. It never says what would have been accepted.
pub fn closed_body() -> Value {
    json!({ "error": "signup_closed", "message": CLOSED_MESSAGE })
}

/// The refusal, 403.
pub fn closed() -> Reply {
    reply::json(403, closed_body())
}

/// Whether a new account may be created for this address.
pub async fn may_create(s: &AppState, email: &str) -> Result<bool, ApiError> {
    if s.cfg.signup != "invite" {
        return Ok(true);
    }
    let email = crate::js::trim(email).to_lowercase();
    if listed(&s.cfg.signup_allow, &s.env("OC_STAFF_EMAILS").unwrap_or_default(), &email) {
        return Ok(true);
    }
    // A team invite is an `auth_tokens` row bound to the invited address (a cancelled one is marked used). Design shares
    // need no check: they only ever point at an existing account.
    let invited: Option<i32> = sqlx::query_scalar("SELECT 1 FROM auth_tokens WHERE kind = 'invite' AND email = $1 AND used_at IS NULL AND expires_at > now() LIMIT 1").bind(&email).fetch_optional(&s.pool).await?;
    Ok(invited.is_some())
}

/// `email` (already lowercase) against `OC_SIGNUP_ALLOW` (exact emails, `@domain`) and `OC_STAFF_EMAILS` (exact emails).
fn listed(allow: &str, staff: &str, email: &str) -> bool {
    let entries = |list: &str| list.split(',').map(|e| crate::js::trim(e).to_lowercase()).filter(|e| !e.is_empty()).collect::<Vec<_>>();
    if entries(staff).iter().any(|e| e == email) {
        return true;
    }
    // Everything after the first `@`, so `a@evil.test@example.test` isn't at example.test.
    let domain = email.split_once('@').map_or("", |(_, d)| d);
    entries(allow).iter().any(|e| match e.strip_prefix('@') {
        Some(d) => !d.is_empty() && d == domain,
        None => e == email,
    })
}

#[cfg(test)]
mod tests {
    use super::listed;

    #[test]
    fn matches_exact_emails_and_domains_only() {
        let allow = " Ana@Example.test , @Friends.test,@,";
        assert!(listed(allow, "", "ana@example.test"));
        assert!(listed(allow, "", "anyone@friends.test"));
        assert!(!listed(allow, "", "bob@example.test"));
        assert!(!listed(allow, "", "x@sub.friends.test"));
        assert!(!listed(allow, "", "x@notfriends.test"));
        assert!(!listed(allow, "", "x@friends.test.evil.test"));
        assert!(!listed(allow, "", "x@evil.test@friends.test"));
        assert!(!listed(allow, "", "x@"));
        assert!(!listed("", "", "x@friends.test"));
    }

    #[test]
    fn staff_entries_are_exact() {
        assert!(listed("", "Root@Example.test, b@example.test", "root@example.test"));
        assert!(!listed("", "@example.test", "root@example.test"));
    }
}
