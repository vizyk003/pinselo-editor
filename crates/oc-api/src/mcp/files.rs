//! Port of services/api/src/mcp/files.ts — download links for files an AI assistant exported: signed (HMAC with the
//! server secret) and valid for 24 hours, so the person can open them from the assistant's chat without signing in.
//!
//! A link is bound to the user and the grant that exported it (OAuth token family or personal access token): the
//! signature covers both, and a download only works while that grant is live and the account active, so
//! disconnecting the app kills its outstanding links. Not single-use: assistants and chat apps fetch the links they
//! show (previews, the model checking the file), which would spend a single-use link before the person clicks.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::prelude::*;

const TTL_S: i64 = 24 * 3600;

#[allow(clippy::too_many_arguments)]
fn sign(secret: &str, sha: &str, name: &str, ty: &str, exp: i64, user: &str, grant: &str) -> String {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("hmac key");
    m.update(format!("mcp-file.{sha}.{name}.{ty}.{exp}.{user}.{grant}").as_bytes());
    URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
}

/// Who a link belongs to: the user and their grant (`BearerUser::grant_id`).
pub struct FileOwner {
    pub user_id: Uuid,
    pub grant_id: Uuid,
}

/// `signedFileUrl(sha, name, type, owner)`.
pub fn signed_file_url(s: &AppState, sha: &str, name: &str, ty: &str, owner: &FileOwner) -> String {
    let exp = s.now_ms().div_euclid(1000) + TTL_S;
    let (u, g) = (owner.user_id.to_string(), owner.grant_id.to_string());
    let q = form_urlencoded::Serializer::new(String::new())
        .append_pair("n", name)
        .append_pair("t", ty)
        .append_pair("e", &exp.to_string())
        .append_pair("u", &u)
        .append_pair("g", &g)
        .append_pair("s", &sign(&s.cfg.secret, sha, name, ty, exp, &u, &g))
        .finish();
    format!("{}/api/mcp/files/{sha}?{q}", s.cfg.web_origin)
}

/// The lower-case canonical UUID form (`/^[0-9a-f]{8}-…$/`).
fn is_uuid(v: &str) -> bool {
    v.len() == 36 && Uuid::parse_str(v).is_ok_and(|u| u.to_string() == v)
}

struct Link<'a> {
    n: Option<&'a str>,
    t: Option<&'a str>,
    e: Option<&'a str>,
    u: Option<&'a str>,
    g: Option<&'a str>,
    s: Option<&'a str>,
}

/// `verifyFile(sha, query)`: the signature and expiry only.
fn verify_file(s: &AppState, sha: &str, l: &Link) -> bool {
    let exp = z::js_number(l.e.map(|e| Value::String(e.to_owned())).as_ref());
    let (Some(n), Some(t), Some(sig)) = (l.n.filter(|x| !x.is_empty()), l.t.filter(|x| !x.is_empty()), l.s.filter(|x| !x.is_empty())) else { return false };
    if exp == 0.0 || exp.is_nan() || exp < s.now_ms() as f64 / 1000.0 {
        return false;
    }
    let (Some(u), Some(g)) = (l.u.filter(|x| is_uuid(x)), l.g.filter(|x| is_uuid(x))) else { return false };
    // The signature covers `String(Number(e))`, as JS template literals print it.
    let want = sign(&s.cfg.secret, sha, n, t, exp as i64, u, g);
    if exp.fract() != 0.0 {
        return false;
    }
    use subtle::ConstantTimeEq as _;
    want.len() == sig.len() && bool::from(want.as_bytes().ct_eq(sig.as_bytes()))
}

/// `grantLive(userId, grantId)`: the grant behind a link is not revoked or expired, and its account is active.
pub async fn grant_live(s: &AppState, user_id: &str, grant_id: &str) -> Result<bool, ApiError> {
    let (Ok(u), Ok(g)) = (Uuid::parse_str(user_id), Uuid::parse_str(grant_id)) else { return Ok(false) };
    let row: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM api_tokens t JOIN users u ON u.id = t.user_id
     WHERE t.user_id = $1 AND (t.family = $2 OR (t.family IS NULL AND t.id = $2)) AND t.revoked_at IS NULL
       AND CASE WHEN t.kind = 'oauth' THEN t.refresh_expires_at > now() ELSE t.expires_at IS NULL OR t.expires_at > now() END
       AND u.deleted_at IS NULL AND u.disabled_at IS NULL
     LIMIT 1",
    )
    .bind(u)
    .bind(g)
    .fetch_optional(&s.pool)
    .await?;
    Ok(row.is_some())
}

pub async fn download(State(s): State<AppState>, Path(sha): Path<String>, req: Req) -> ApiResult {
    let q = |k: &str| req.query_str(k);
    let link = Link { n: q("n"), t: q("t"), e: q("e"), u: q("u"), g: q("g"), s: q("s") };
    let ok = sha.len() == 64
        && sha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && verify_file(&s, &sha, &link)
        && grant_live(&s, link.u.unwrap_or(""), link.g.unwrap_or("")).await?;
    if !ok {
        return Ok(reply::json(404, json!({ "error": "not_found", "message": "This download link is invalid or has expired. Ask the assistant to export again." })));
    }
    let name: String = q("n").unwrap_or("").chars().filter(|c| !matches!(c, '"' | '\r' | '\n')).collect();
    let bytes = s.store.get(&sha).await?;
    Ok(Reply::bytes(q("t").unwrap_or(""), bytes).header("content-disposition", format!("attachment; filename=\"{name}\"")).header("cache-control", "private, max-age=3600"))
}
