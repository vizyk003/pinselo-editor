//! The part of `jose` 6 that routes/account.ts uses: `jwtVerify(idToken, createRemoteJWKSet(jwksUri),
//! { issuer, audience })` for OIDC sign-in. RS256 and ES256 keys; the key is picked by `kid` (and `kty`/`alg`
//! compatible with the token's `alg`). Expiry and not-before use the app clock, with no tolerance. Every
//! failure is an error (the route then answers 500, as jose throws in TS).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

fn b64(s: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).map_err(|_| "Invalid Compact JWS".to_owned())
}

/// Verify `token` against the key set at `jwks_uri`; the payload on success.
pub async fn verify(http: &reqwest::Client, token: &str, jwks_uri: &str, issuer: &str, audience: &str, now_ms: i64) -> Result<Value, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid Compact JWS".into());
    }
    let header: Value = serde_json::from_slice(&b64(parts[0])?).map_err(|e| e.to_string())?;
    let payload: Value = serde_json::from_slice(&b64(parts[1])?).map_err(|e| e.to_string())?;
    let sig = b64(parts[2])?;
    let alg = header.get("alg").and_then(Value::as_str).unwrap_or("");
    let kid = header.get("kid").and_then(Value::as_str);
    let jwks: Value = serde_json::from_slice(&http.get(jwks_uri).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let kty = match alg {
        "RS256" => "RSA",
        "ES256" => "EC",
        other => return Err(format!("Unsupported \"alg\" value: {other}")),
    };
    let key = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter().find(|k| {
                k.get("kty").and_then(Value::as_str) == Some(kty)
                    && kid.is_none_or(|id| k.get("kid").and_then(Value::as_str) == Some(id))
                    && k.get("alg").and_then(Value::as_str).is_none_or(|a| a == alg)
                    && k.get("use").and_then(Value::as_str).is_none_or(|u| u == "sig")
            })
        })
        .ok_or("no applicable key found in the JSON Web Key Set")?;
    let field = |k: &str| key.get(k).and_then(Value::as_str).ok_or_else(|| format!("JWK is missing {k}")).and_then(b64);
    let signed = format!("{}.{}", parts[0], parts[1]);
    use ring::signature as rs;
    let ok = match alg {
        "RS256" => rs::RsaPublicKeyComponents { n: &field("n")?, e: &field("e")? }.verify(&rs::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &sig).is_ok(),
        _ => {
            let mut pt = vec![4u8];
            pt.extend(field("x")?);
            pt.extend(field("y")?);
            rs::UnparsedPublicKey::new(&rs::ECDSA_P256_SHA256_FIXED, pt).verify(signed.as_bytes(), &sig).is_ok()
        }
    };
    if !ok {
        return Err("signature verification failed".into());
    }
    let now = (now_ms as f64 / 1000.0).floor();
    if payload.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err("unexpected \"iss\" claim value".into());
    }
    let aud_ok = match payload.get("aud") {
        Some(Value::String(a)) => a == audience,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(audience)),
        _ => false,
    };
    if !aud_ok {
        return Err("unexpected \"aud\" claim value".into());
    }
    if let Some(exp) = payload.get("exp") {
        let e = exp.as_f64().ok_or("\"exp\" claim must be a number")?;
        if e <= now {
            return Err("\"exp\" claim timestamp check failed".into());
        }
    }
    if let Some(nbf) = payload.get("nbf") {
        let n = nbf.as_f64().ok_or("\"nbf\" claim must be a number")?;
        if n > now {
            return Err("\"nbf\" claim timestamp check failed".into());
        }
    }
    Ok(payload)
}
