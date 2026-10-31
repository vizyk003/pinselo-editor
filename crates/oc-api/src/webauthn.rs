//! The parts of `@simplewebauthn/server` 14 that routes/account.ts uses: registration/authentication options
//! and response verification.
//!
//! Supported like the TS library: attestation `none` and `packed` self-attestation; credential keys EdDSA
//! (Ed25519), ES256 (P-256), RS256 and ML-DSA-44 (FIPS 204, COSE alg -48, key type AKP). NOT supported (the TS library verifies them against certificate chains):
//! `packed` with an `x5c` chain, `fido-u2f`, `android-key`, `android-safetynet`, `tpm`, `apple` — those answer
//! with an error (500), as an attestation the TS side failed to verify would. Browsers send `none` when
//! `attestation: 'none'` is requested, which is what the API asks for.
//!
//! As in the library, malformed responses throw (the route then answers 500); only a signature that does not
//! verify yields `verified: false`. ML-DSA-44 is offered first, as `@simplewebauthn/server` does on a runtime whose
//! `SubtleCrypto.supports('verify', 'ML-DSA-44')` is true (Node 24 from `.nvmrc`; Node 22 and Node < 24.7 lack it and
//! offer `[-8, -7, -257]`). ML-DSA keys are COSE `AKP` keys (draft-ietf-cose-dilithium): `{1: 7, 3: alg, -1: pub}`,
//! signatures are pure ML-DSA with an empty context, verified with RustCrypto's `ml-dsa` (pinned).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub const SUPPORTED_ALGS: [i64; 4] = [-48, -8, -7, -257];

/// A decoded CBOR item.
#[derive(Debug, Clone, PartialEq)]
pub enum Cbor {
    Int(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(Cbor, Cbor)>),
    Bool(bool),
    Null,
    Float(f64),
}

impl Cbor {
    pub fn get(&self, key: &Cbor) -> Option<&Cbor> {
        match self {
            Cbor::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn get_int(&self, k: i128) -> Option<&Cbor> {
        self.get(&Cbor::Int(k))
    }
    pub fn get_text(&self, k: &str) -> Option<&Cbor> {
        self.get(&Cbor::Text(k.to_owned()))
    }
}

/// Decode the first CBOR item of `b`: the item and the number of bytes it used.
pub fn cbor_first(b: &[u8]) -> Result<(Cbor, usize), String> {
    fn arg(b: &[u8], p: &mut usize, info: u8) -> Result<u64, String> {
        let take = |p: &mut usize, n: usize| -> Result<u64, String> {
            let s = b.get(*p..*p + n).ok_or("cbor: truncated")?;
            *p += n;
            Ok(s.iter().fold(0u64, |a, &x| (a << 8) | u64::from(x)))
        };
        match info {
            0..=23 => Ok(u64::from(info)),
            24 => take(p, 1),
            25 => take(p, 2),
            26 => take(p, 4),
            27 => take(p, 8),
            _ => Err("cbor: unsupported length".into()),
        }
    }
    fn item(b: &[u8], p: &mut usize, depth: usize) -> Result<Cbor, String> {
        if depth > 32 {
            return Err("cbor: too deep".into());
        }
        let ib = *b.get(*p).ok_or("cbor: truncated")?;
        *p += 1;
        let (major, info) = (ib >> 5, ib & 0x1f);
        Ok(match major {
            0 => Cbor::Int(i128::from(arg(b, p, info)?)),
            1 => Cbor::Int(-1 - i128::from(arg(b, p, info)?)),
            2 | 3 => {
                let n = arg(b, p, info)? as usize;
                let s = b.get(*p..*p + n).ok_or("cbor: truncated")?.to_vec();
                *p += n;
                if major == 2 { Cbor::Bytes(s) } else { Cbor::Text(String::from_utf8(s).map_err(|_| "cbor: bad text")?) }
            }
            4 => {
                let n = arg(b, p, info)?;
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(item(b, p, depth + 1)?);
                }
                Cbor::Array(v)
            }
            5 => {
                let n = arg(b, p, info)?;
                let mut v = Vec::new();
                for _ in 0..n {
                    let k = item(b, p, depth + 1)?;
                    let x = item(b, p, depth + 1)?;
                    v.push((k, x));
                }
                Cbor::Map(v)
            }
            7 => match info {
                20 => Cbor::Bool(false),
                21 => Cbor::Bool(true),
                22 | 23 => Cbor::Null,
                25 => {
                    let h = arg(b, p, 25)? as u16;
                    Cbor::Float(half(h))
                }
                26 => Cbor::Float(f64::from(f32::from_bits(arg(b, p, 26)? as u32))),
                27 => Cbor::Float(f64::from_bits(arg(b, p, 27)?)),
                _ => return Err("cbor: unsupported simple value".into()),
            },
            _ => return Err("cbor: unsupported major type".into()),
        })
    }
    let mut p = 0;
    let v = item(b, &mut p, 0)?;
    Ok((v, p))
}

fn half(h: u16) -> f64 {
    let exp = (h >> 10) & 0x1f;
    let mant = f64::from(h & 0x3ff);
    let v = if exp == 0 { mant * 2f64.powi(-24) } else if exp != 31 { (mant + 1024.0) * 2f64.powi(i32::from(exp) - 25) } else if mant == 0.0 { f64::INFINITY } else { f64::NAN };
    if h & 0x8000 != 0 { -v } else { v }
}

pub fn b64url(b: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

/// `isoBase64URL.toBuffer` (padding and the standard alphabet tolerated).
pub fn from_b64url(s: &str) -> Vec<u8> {
    let clean: String = s.chars().filter(|c| *c != '=').map(|c| match c {
        '+' => '-',
        '/' => '_',
        c => c,
    }).collect();
    URL_SAFE_NO_PAD.decode(clean.as_bytes()).unwrap_or_default()
}

fn is_b64url(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=')
}

/// `generateRegistrationOptions(...)` with the API's arguments.
pub fn registration_options(challenge: &[u8], rp_name: &str, rp_id: &str, user_name: &str, user_display_name: &str, user_id: &[u8], exclude: &[(String, Option<Vec<String>>)]) -> Value {
    let excl: Vec<Value> = exclude
        .iter()
        .map(|(id, transports)| {
            let mut m = Map::new();
            m.insert("id".into(), json!(id.trim_end_matches('=')));
            if let Some(t) = transports {
                m.insert("transports".into(), json!(t));
            }
            m.insert("type".into(), json!("public-key"));
            Value::Object(m)
        })
        .collect();
    json!({
        "challenge": b64url(challenge),
        "rp": { "name": rp_name, "id": rp_id },
        "user": { "id": b64url(user_id), "name": user_name, "displayName": user_display_name },
        "pubKeyCredParams": SUPPORTED_ALGS.iter().map(|a| json!({ "alg": a, "type": "public-key" })).collect::<Vec<_>>(),
        "timeout": 60000,
        "attestation": "none",
        "excludeCredentials": excl,
        "authenticatorSelection": { "residentKey": "preferred", "userVerification": "preferred", "requireResidentKey": false },
        "extensions": { "credProps": true },
        "hints": [],
    })
}

/// `generateAuthenticationOptions({ rpID, userVerification: 'preferred' })`.
pub fn authentication_options(challenge: &[u8], rp_id: &str) -> Value {
    json!({ "rpId": rp_id, "challenge": b64url(challenge), "timeout": 60000, "userVerification": "preferred" })
}

pub struct AuthData {
    pub rp_id_hash: Vec<u8>,
    pub up: bool,
    pub uv: bool,
    pub counter: u32,
    pub aaguid: Option<Vec<u8>>,
    pub credential_id: Option<Vec<u8>>,
    pub credential_public_key: Option<Vec<u8>>,
}

/// `parseAuthenticatorData(authData)`.
pub fn parse_auth_data(a: &[u8]) -> Result<AuthData, String> {
    if a.len() < 37 {
        return Err(format!("Authenticator data was {} bytes, expected at least 37 bytes", a.len()));
    }
    let flags = a[32];
    let counter = u32::from_be_bytes([a[33], a[34], a[35], a[36]]);
    let mut p = 37;
    let (mut aaguid, mut cid, mut cpk) = (None, None, None);
    if flags & 0x40 != 0 {
        aaguid = Some(a.get(p..p + 16).ok_or("truncated authenticator data")?.to_vec());
        p += 16;
        let n = usize::from(u16::from_be_bytes([*a.get(p).ok_or("truncated")?, *a.get(p + 1).ok_or("truncated")?]));
        p += 2;
        cid = Some(a.get(p..p + n).ok_or("truncated authenticator data")?.to_vec());
        p += n;
        let (_, used) = cbor_first(&a[p..])?;
        cpk = Some(a[p..p + used].to_vec());
        p += used;
    }
    if flags & 0x80 != 0 {
        let (_, used) = cbor_first(&a[p..])?;
        p += used;
    }
    if a.len() > p {
        return Err("Leftover bytes detected while parsing authenticator data".into());
    }
    Ok(AuthData { rp_id_hash: a[..32].to_vec(), up: flags & 1 != 0, uv: flags & 4 != 0, counter, aaguid, credential_id: cid, credential_public_key: cpk })
}

/// `verifyAKP`: pure ML-DSA (empty context) over `data`. A public key of the wrong length fails like WebCrypto's
/// `importKey` (an error); a malformed signature does not verify.
fn verify_ml_dsa<P: ml_dsa::MlDsaParams>(public_key: &[u8], data: &[u8], sig: &[u8]) -> Result<bool, String> {
    let enc = ml_dsa::EncodedVerifyingKey::<P>::try_from(public_key).map_err(|_| "Invalid keyData".to_owned())?;
    let vk = ml_dsa::VerifyingKey::<P>::decode(&enc);
    let Ok(es) = ml_dsa::EncodedSignature::<P>::try_from(sig) else { return Ok(false) };
    let Some(sig) = ml_dsa::Signature::<P>::decode(&es) else { return Ok(false) };
    Ok(vk.verify_with_context(data, &[], &sig))
}

/// `verifySignature({ signature, data, credentialPublicKey })` for EdDSA / ES256 / RS256 / ML-DSA COSE keys.
pub fn verify_signature(cose: &[u8], data: &[u8], sig: &[u8]) -> Result<bool, String> {
    use ring::signature as rs;
    let (key, _) = cbor_first(cose)?;
    let kty = key.get_int(1).cloned();
    let alg = key.get_int(3).cloned();
    let bytes = |k: i128| match key.get_int(k) {
        Some(Cbor::Bytes(b)) => Ok(b.clone()),
        _ => Err("Credential public key was missing a coordinate".to_owned()),
    };
    match (kty, alg) {
        (Some(Cbor::Int(1)), _) => {
            let x = bytes(-2)?;
            Ok(rs::UnparsedPublicKey::new(&rs::ED25519, x).verify(data, sig).is_ok())
        }
        (Some(Cbor::Int(2)), Some(Cbor::Int(-7))) => {
            let (x, y) = (bytes(-2)?, bytes(-3)?);
            let mut pt = vec![4u8];
            pt.extend_from_slice(&x);
            pt.extend_from_slice(&y);
            Ok(rs::UnparsedPublicKey::new(&rs::ECDSA_P256_SHA256_ASN1, pt).verify(data, sig).is_ok())
        }
        (Some(Cbor::Int(3)), Some(Cbor::Int(-257))) => {
            let (n, e) = (bytes(-1)?, bytes(-2)?);
            Ok(rs::RsaPublicKeyComponents { n: &n, e: &e }.verify(&rs::RSA_PKCS1_2048_8192_SHA256, data, sig).is_ok())
        }
        // AKP (ML-DSA): `verifyAKP` maps -48/-49/-50 to WebCrypto's ML-DSA-44/65/87 (Node 24 verifies all three).
        (Some(Cbor::Int(7)), alg) => {
            let Some(Cbor::Int(alg)) = alg else { return Err("Public key was missing alg (AKP)".into()) };
            let Some(Cbor::Bytes(public)) = key.get_int(-1) else { return Err("Public key was missing pub (AKP)".into()) };
            match alg {
                -48 => verify_ml_dsa::<ml_dsa::MlDsa44>(public, data, sig),
                -49 => verify_ml_dsa::<ml_dsa::MlDsa65>(public, data, sig),
                -50 => verify_ml_dsa::<ml_dsa::MlDsa87>(public, data, sig),
                other => Err(format!("Could not map COSE alg value of {other} to a WebCrypto key alg name")),
            }
        }
        _ => Err("Unsupported public key".into()),
    }
}

fn cose_alg(cose: &[u8]) -> Result<i128, String> {
    match cbor_first(cose)?.0.get_int(3) {
        Some(Cbor::Int(a)) => Ok(*a),
        _ => Err("Credential public key was missing numeric alg".into()),
    }
}

fn client_data(resp: &Value) -> Result<(Value, Vec<u8>), String> {
    let raw = resp.get("clientDataJSON").and_then(Value::as_str).ok_or("Credential response clientDataJSON was not a string")?;
    let bytes = from_b64url(raw);
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok((v, bytes))
}

fn common_checks(response: &Value, want_type: &str, challenge: &str, origin: &str, what: &str) -> Result<Value, String> {
    let id = response.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return Err("Missing credential ID".into());
    }
    if response.get("rawId").and_then(Value::as_str) != Some(id) {
        return Err("Credential ID was not base64url-encoded".into());
    }
    if response.get("type").and_then(Value::as_str) != Some("public-key") {
        return Err(format!("Unexpected credential type {}, expected \"public-key\"", response.get("type").map_or("undefined".into(), crate::http::jsjson::stringify)));
    }
    let (cd, _) = client_data(response.get("response").unwrap_or(&Value::Null))?;
    let s = |k: &str| cd.get(k).and_then(Value::as_str).unwrap_or("undefined").to_owned();
    if s("type") != want_type {
        return Err(format!("Unexpected {what} response type: {}", s("type")));
    }
    if s("challenge") != challenge {
        return Err(format!("Unexpected {what} response challenge \"{}\", expected \"{challenge}\"", s("challenge")));
    }
    if s("origin") != origin {
        return Err(format!("Unexpected {what} response origin \"{}\", expected \"{origin}\"", s("origin")));
    }
    Ok(cd)
}

pub struct Registration {
    pub id: String,
    pub public_key: Vec<u8>,
    pub counter: u32,
    pub transports: Option<Vec<String>>,
}

/// `verifyRegistrationResponse({ response, expectedChallenge, expectedOrigin, expectedRPID })`:
/// `Ok(None)` = `verified: false`.
pub fn verify_registration(response: &Value, challenge: &str, origin: &str, rp_id: &str) -> Result<Option<Registration>, String> {
    common_checks(response, "webauthn.create", challenge, origin, "registration")?;
    let resp = response.get("response").cloned().unwrap_or(Value::Null);
    let ao = from_b64url(resp.get("attestationObject").and_then(Value::as_str).unwrap_or(""));
    let (att, _) = cbor_first(&ao)?;
    let fmt = match att.get_text("fmt") {
        Some(Cbor::Text(t)) => t.clone(),
        _ => String::new(),
    };
    let Some(Cbor::Bytes(auth_data)) = att.get_text("authData") else { return Err("Attestation object had no authData".into()) };
    let att_stmt = att.get_text("attStmt").cloned().unwrap_or(Cbor::Map(vec![]));
    let a = parse_auth_data(auth_data)?;
    if a.rp_id_hash != Sha256::digest(rp_id.as_bytes()).to_vec() {
        return Err("Unexpected RP ID hash".into());
    }
    if !a.up {
        return Err("User presence was required, but user was not present".into());
    }
    if !a.uv {
        return Err("User verification was required, but user could not be verified".into());
    }
    let cid = a.credential_id.clone().ok_or("No credential ID was provided by authenticator")?;
    let cpk = a.credential_public_key.clone().ok_or("No public key was provided by authenticator")?;
    if a.aaguid.is_none() {
        return Err("No AAGUID was present during registration".into());
    }
    let alg = cose_alg(&cpk)?;
    if !SUPPORTED_ALGS.iter().any(|x| i128::from(*x) == alg) {
        let supported = SUPPORTED_ALGS.iter().map(i64::to_string).collect::<Vec<_>>().join(", ");
        return Err(format!("Unexpected public key alg \"{alg}\", expected one of \"{supported}\""));
    }
    let client_hash = Sha256::digest(from_b64url(resp.get("clientDataJSON").and_then(Value::as_str).unwrap_or(""))).to_vec();
    let verified = match fmt.as_str() {
        "none" => {
            if matches!(&att_stmt, Cbor::Map(m) if !m.is_empty()) {
                return Err("None attestation had unexpected attestation statement".into());
            }
            true
        }
        "packed" => {
            if att_stmt.get_text("x5c").is_some() {
                return Err("Packed attestation with a certificate chain is not supported by this server".into());
            }
            let Some(Cbor::Bytes(sig)) = att_stmt.get_text("sig") else { return Err("No attestation signature provided in attestation statement".into()) };
            match att_stmt.get_text("alg") {
                Some(Cbor::Int(x)) if *x == alg => {}
                _ => return Err("Attestation alg did not match credential public key alg".into()),
            }
            let mut data = auth_data.clone();
            data.extend_from_slice(&client_hash);
            verify_signature(&cpk, &data, sig)?
        }
        other => return Err(format!("Unsupported Attestation Format: {other}")),
    };
    if !verified {
        return Ok(None);
    }
    let transports = resp.get("transports").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
    Ok(Some(Registration { id: b64url(&cid), public_key: cpk, counter: a.counter, transports }))
}

/// `verifyAuthenticationResponse(...)`: `Ok(Some(newCounter))`, or `Ok(None)` when the signature fails.
pub fn verify_authentication(response: &Value, challenge: &str, origin: &str, rp_id: &str, public_key: &[u8], counter: i64) -> Result<Option<u32>, String> {
    let cd = common_checks(response, "webauthn.get", challenge, origin, "authentication")?;
    if cd.get("crossOrigin").is_none_or(|v| v == &Value::Bool(false) || v.is_null()) && cd.get("topOrigin").and_then(Value::as_str).is_some_and(|t| !t.is_empty()) {
        return Err("Unexpected top origin within a non-cross-origin authentication response".into());
    }
    let resp = response.get("response").cloned().unwrap_or(Value::Null);
    let ad = resp.get("authenticatorData").and_then(Value::as_str).unwrap_or("");
    let sig = resp.get("signature").and_then(Value::as_str).unwrap_or("");
    if !is_b64url(ad) {
        return Err("Credential response authenticatorData was not a base64url string".into());
    }
    if !is_b64url(sig) {
        return Err("Credential response signature was not a base64url string".into());
    }
    let auth_data = from_b64url(ad);
    let a = parse_auth_data(&auth_data)?;
    if a.rp_id_hash != Sha256::digest(rp_id.as_bytes()).to_vec() {
        return Err("Unexpected RP ID hash".into());
    }
    if !a.up {
        return Err("User not present during authentication".into());
    }
    if !a.uv {
        return Err("User verification required, but user could not be verified".into());
    }
    let client_hash = Sha256::digest(from_b64url(resp.get("clientDataJSON").and_then(Value::as_str).unwrap_or(""))).to_vec();
    if (a.counter > 0 || counter > 0) && i64::from(a.counter) <= counter {
        return Err(format!("Response counter value {} was lower than expected {counter}", a.counter));
    }
    cose_alg(public_key)?;
    let mut data = auth_data.clone();
    data.extend_from_slice(&client_hash);
    Ok(verify_signature(public_key, &data, &from_b64url(sig))?.then_some(a.counter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbor_basics() {
        // {1: 1, 3: -8, -1: 6, -2: h'0102'}
        let b = hex::decode("a4010103272006214201 02".replace(' ', "")).unwrap();
        let (v, n) = cbor_first(&b).unwrap();
        assert_eq!(n, b.len());
        assert_eq!(v.get_int(3), Some(&Cbor::Int(-8)));
        assert_eq!(v.get_int(-2), Some(&Cbor::Bytes(vec![1, 2])));
    }

    /// COSE AKP key `{1: 7, 3: -48, -1: pub}` (what an ML-DSA-44 authenticator and the harnesses send).
    fn akp_cose(public: &[u8]) -> Vec<u8> {
        let mut cose = hex::decode("a3010703382f20590520").unwrap();
        assert_eq!(public.len(), 0x520);
        cose.extend_from_slice(public);
        cose
    }

    /// fixtures/webauthn/ml-dsa-44.json: a key seed and message with a signature made by Node 24's WebCrypto/OpenSSL
    /// (`nodeSignature`, hedged) and one made by `ml-dsa` (`rustSignature`, deterministic). Rust must derive Node's
    /// public key from the seed and verify Node's signature; tools/golden/api.ts checks that Node verifies ours.
    #[test]
    fn ml_dsa_44_cross_checks_with_node() {
        use ml_dsa::{KeyExport as _, Keypair as _, Signer as _, SignatureEncoding as _};
        let v: Value = serde_json::from_str(include_str!("../../../fixtures/webauthn/ml-dsa-44.json")).unwrap();
        let h = |k: &str| hex::decode(v[k].as_str().unwrap()).unwrap();
        let (seed, msg) = (h("seed"), h("message"));
        let sk = ml_dsa::SigningKey::<ml_dsa::MlDsa44>::from_seed(&ml_dsa::Seed::try_from(seed.as_slice()).unwrap());
        let public = sk.verifying_key().to_bytes().to_vec();
        assert_eq!(hex::encode(&public), hex::encode(h("publicKey")), "public key derived from the seed");
        let cose = akp_cose(&public);
        assert_eq!(cose_alg(&cose).unwrap(), -48);
        assert!(verify_signature(&cose, &msg, &h("nodeSignature")).unwrap(), "Node's signature verifies");
        let ours = sk.sign(&msg).to_vec();
        assert_eq!(hex::encode(&ours), v["rustSignature"].as_str().unwrap_or(""), "deterministic ml-dsa signature");
        assert!(verify_signature(&cose, &msg, &ours).unwrap());
        // Tampered message / signature, short signature: not verified. A truncated key is an error (importKey).
        let mut bad = msg.clone();
        bad[0] ^= 1;
        assert!(!verify_signature(&cose, &bad, &ours).unwrap());
        let mut bad_sig = ours.clone();
        bad_sig[5] ^= 1;
        assert!(!verify_signature(&cose, &msg, &bad_sig).unwrap());
        assert!(!verify_signature(&cose, &msg, &ours[..100]).unwrap());
        let mut short = hex::decode("a3010703382f2058ff").unwrap();
        short.extend_from_slice(&public[..255]);
        assert!(verify_signature(&short, &msg, &ours).is_err());
    }
}
