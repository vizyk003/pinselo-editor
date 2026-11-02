//! The part of `jose` 6 that routes/account.ts uses: `jwtVerify(idToken, createRemoteJWKSet(jwksUri),
//! { issuer, audience })` for OIDC sign-in, on `jsonwebtoken`. RS256 and ES256 keys; the key is picked by `kid` (and
//! `kty`/`alg` compatible with the token's `alg`). Expiry and not-before use the app clock, with no tolerance, so they
//! are checked here: the library only knows the system clock. Every failure is an error (the route then answers
//! 500, as jose throws in TS).

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value;

/// Verify `token` against the key set at `jwks_uri`; the payload on success.
pub async fn verify(http: &reqwest::Client, token: &str, jwks_uri: &str, issuer: &str, audience: &str, now_ms: i64) -> Result<Value, String> {
    let jwks: Value = serde_json::from_slice(&http.get(jwks_uri).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    verify_with(token, &jwks, issuer, audience, now_ms)
}

fn verify_with(token: &str, jwks: &Value, issuer: &str, audience: &str, now_ms: i64) -> Result<Value, String> {
    let header = jsonwebtoken::decode_header(token).map_err(|e| e.to_string())?;
    let (alg, kty) = match header.alg {
        Algorithm::RS256 => ("RS256", "RSA"),
        Algorithm::ES256 => ("ES256", "EC"),
        other => return Err(format!("Unsupported \"alg\" value: {other:?}")),
    };
    let key = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter().find(|k| {
                k.get("kty").and_then(Value::as_str) == Some(kty)
                    && header.kid.as_deref().is_none_or(|id| k.get("kid").and_then(Value::as_str) == Some(id))
                    && k.get("alg").and_then(Value::as_str).is_none_or(|a| a == alg)
                    && k.get("use").and_then(Value::as_str).is_none_or(|u| u == "sig")
            })
        })
        .ok_or("no applicable key found in the JSON Web Key Set")?;
    let field = |k: &str| key.get(k).and_then(Value::as_str).ok_or_else(|| format!("JWK is missing {k}"));
    let decoding = match header.alg {
        Algorithm::RS256 => DecodingKey::from_rsa_components(field("n")?, field("e")?),
        _ => DecodingKey::from_ec_components(field("x")?, field("y")?),
    }
    .map_err(|e| e.to_string())?;

    let mut rules = Validation::new(header.alg);
    rules.validate_exp = false;
    rules.validate_nbf = false;
    // Unless they are required, the library lets a token without these claims through.
    rules.set_required_spec_claims(&["iss", "aud"]);
    rules.set_issuer(&[issuer]);
    rules.set_audience(&[audience]);
    let payload = jsonwebtoken::decode::<Value>(token, &decoding, &rules).map_err(|e| e.to_string())?.claims;

    let now = (now_ms as f64 / 1000.0).floor();
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

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use serde_json::{Value, json};

    fn key() -> EcdsaKeyPair {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap()
    }

    fn jwks(k: &EcdsaKeyPair) -> Value {
        let p = k.public_key().as_ref();
        json!({ "keys": [{ "kty": "EC", "crv": "P-256", "kid": "k", "x": B64.encode(&p[1..33]), "y": B64.encode(&p[33..]) }] })
    }

    fn sign(k: &EcdsaKeyPair, header: &Value, claims: &Value) -> String {
        let body = format!("{}.{}", B64.encode(header.to_string()), B64.encode(claims.to_string()));
        let sig = k.sign(&SystemRandom::new(), body.as_bytes()).unwrap();
        format!("{body}.{}", B64.encode(sig.as_ref()))
    }

    const NOW: i64 = 1_000_000_000;

    fn check(k: &EcdsaKeyPair, claims: Value) -> Result<Value, String> {
        super::verify_with(&sign(k, &json!({ "alg": "ES256", "kid": "k" }), &claims), &jwks(k), "iss", "aud", NOW * 1000)
    }

    #[test]
    fn accepts_a_valid_token_on_the_app_clock() {
        let k = key();
        assert!(check(&k, json!({ "iss": "iss", "aud": "aud", "exp": NOW + 1, "sub": "u" })).is_ok());
        assert!(check(&k, json!({ "iss": "iss", "aud": ["x", "aud"] })).is_ok());
    }

    #[test]
    fn refuses_bad_claims() {
        let k = key();
        for claims in [
            json!({ "aud": "aud" }),
            json!({ "iss": "iss" }),
            json!({ "iss": "other", "aud": "aud" }),
            json!({ "iss": "iss", "aud": "other" }),
            json!({ "iss": "iss", "aud": "aud", "exp": NOW }),
            json!({ "iss": "iss", "aud": "aud", "nbf": NOW + 1 }),
            json!({ "iss": "iss", "aud": "aud", "exp": "soon" }),
        ] {
            assert!(check(&k, claims.clone()).is_err(), "{claims}");
        }
    }

    #[test]
    fn refuses_bad_signatures_and_algorithms() {
        let (k, other) = (key(), key());
        let claims = json!({ "iss": "iss", "aud": "aud" });
        let forged = sign(&other, &json!({ "alg": "ES256", "kid": "k" }), &claims);
        assert!(super::verify_with(&forged, &jwks(&k), "iss", "aud", NOW * 1000).is_err());
        let unsigned = format!("{}.{}.", B64.encode(r#"{"alg":"none"}"#), B64.encode(claims.to_string()));
        assert!(super::verify_with(&unsigned, &jwks(&k), "iss", "aud", NOW * 1000).is_err());
        let hs = sign(&k, &json!({ "alg": "HS256", "kid": "k" }), &claims);
        assert!(super::verify_with(&hs, &jwks(&k), "iss", "aud", NOW * 1000).is_err());
    }
}
