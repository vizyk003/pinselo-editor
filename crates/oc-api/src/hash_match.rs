//! Port of services/api/src/hashMatch.ts — upload hash matching (TS-001):
//! * local: 64-bit dHash against `hash_blocklist` (always on);
//! * PhotoDNA Cloud Service when `OC_PHOTODNA_KEY` is set (images and a video's first frame);
//!   `OC_PHOTODNA_URL` overrides the Match endpoint. If the cloud check is unavailable the upload proceeds and
//!   the gap is audited, unless `OC_PHOTODNA_FAIL_CLOSED=1` (then it is refused with a retryable error).

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::media;
use crate::routes::safety;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashHit {
    pub hash: String,
    pub category: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashVerdict {
    Ok,
    Hit(HashHit),
    Unavailable,
}

const DEFAULT_PHOTODNA_URL: &str = "https://api.microsoftmoderator.com/photodna/v1.0/Match?enhance=false";

/// Re-encode to a JPEG the service accepts (≥160 px on the short side, first frame of video/GIF).
async fn photo_dna_jpeg(src: &Path, dir: &Path) -> Option<Vec<u8>> {
    let out = dir.join("pdna.jpg");
    let (s, o) = (src.to_string_lossy().into_owned(), out.to_string_lossy().into_owned());
    let r = media::run(
        "ffmpeg",
        &["-hide_banner", "-loglevel", "error", "-y", "-i", &s, "-frames:v", "1", "-vf", "scale='if(lt(iw,ih),max(160,min(iw,2048)),-2)':'if(lt(iw,ih),-2,max(160,min(ih,2048)))'", "-q:v", "3", &o],
        true,
    )
    .await;
    let bytes = match r {
        Ok(_) => tokio::fs::read(&out).await.ok(),
        Err(_) => None,
    };
    let _ = tokio::fs::remove_file(&out).await;
    bytes
}

/// `photoDnaMatch(jpeg, key)`.
pub async fn photo_dna_match(state: &AppState, jpeg: Vec<u8>, key: &str) -> Result<Option<HashHit>, String> {
    let url = state.env("OC_PHOTODNA_URL").unwrap_or_else(|| DEFAULT_PHOTODNA_URL.to_owned());
    let r = state
        .http
        .post(url)
        .header("content-type", "image/jpeg")
        .header("ocp-apim-subscription-key", key)
        .body(jpeg)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("PhotoDNA HTTP {}", r.status().as_u16()));
    }
    let j: Value = serde_json::from_slice(&r.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // 3000 = OK; 3208 = image too small/unsupported (nothing to match).
    if let Some(code) = j.get("Status").and_then(|s| s.get("Code")).and_then(Value::as_i64)
        && code != 3000
    {
        if code == 3208 {
            return Ok(None);
        }
        return Err(format!("PhotoDNA status {code}"));
    }
    if j.get("IsMatch") != Some(&Value::Bool(true)) {
        return Ok(None);
    }
    let src = j.pointer("/MatchDetails/MatchFlags/0/Source").and_then(Value::as_str).filter(|s| !s.is_empty());
    Ok(Some(HashHit {
        hash: j.get("TrackingId").and_then(Value::as_str).unwrap_or("photodna").to_owned(),
        category: "csam".into(),
        source: format!("photodna{}", src.map(|s| format!(":{s}")).unwrap_or_default()),
    }))
}

/// `matchUpload(src, dir, phash, ctx)`.
pub async fn match_upload(state: &AppState, src: &Path, dir: &Path, phash: Option<&str>, owner_id: Uuid, ip: Option<&str>, raster: bool) -> Result<HashVerdict, sqlx::Error> {
    if let Some(hit) = safety::blocklist_match(state, phash).await? {
        return Ok(HashVerdict::Hit(hit));
    }
    let Some(key) = state.env_nonempty("OC_PHOTODNA_KEY") else { return Ok(HashVerdict::Ok) };
    if !raster {
        return Ok(HashVerdict::Ok);
    }
    let Some(jpeg) = photo_dna_jpeg(src, dir).await else { return Ok(HashVerdict::Ok) };
    match photo_dna_match(state, jpeg, &key).await {
        Ok(Some(hit)) => Ok(HashVerdict::Hit(hit)),
        Ok(None) => Ok(HashVerdict::Ok),
        Err(e) => {
            oc_audit::audit_at(
                &state.pool,
                oc_audit::AuditEvent {
                    actor_id: Some(owner_id),
                    action: "ts.hash_check_unavailable".into(),
                    meta: Some(json!({ "provider": "photodna", "error": e })),
                    ip: ip.map(str::to_owned),
                    ..Default::default()
                },
                state.now(),
            )
            .await?;
            Ok(if state.env("OC_PHOTODNA_FAIL_CLOSED").as_deref() == Some("1") { HashVerdict::Unavailable } else { HashVerdict::Ok })
        }
    }
}
