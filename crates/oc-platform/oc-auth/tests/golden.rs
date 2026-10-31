//! Parity with tests/golden/platform/auth.json: tickets minted by TS verify in Rust, Rust mints the
//! byte-identical token for the same claims/clock, and verification outcomes match case by case.
use oc_auth::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

fn aud(v: &Value) -> Audience {
    serde_json::from_value(v.clone()).unwrap()
}

fn role(v: &Value) -> Option<Role> {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn tokens_are_byte_identical() {
    let g = golden("auth.json");
    let secret = g["secret"].as_str().unwrap();
    for s in g["signed"].as_array().unwrap() {
        let claims: TicketClaims = serde_json::from_value(s["claims"].clone()).unwrap();
        let tok = sign_ticket_at(secret, &claims, s["ttl"].as_i64().unwrap(), s["now"].as_i64().unwrap());
        assert_eq!(tok, s["token"].as_str().unwrap());
        // TS-minted token verifies in Rust.
        let t = verify_ticket_at(secret, s["token"].as_str().unwrap(), claims.aud, s["now"].as_i64().unwrap()).unwrap();
        assert_eq!(t.claims, claims);
    }
}

#[test]
fn verification_matches() {
    let g = golden("auth.json");
    let secret = g["secret"].as_str().unwrap();
    for c in g["verify"].as_array().unwrap() {
        let got = verify_ticket_at(secret, c["token"].as_str().unwrap(), aud(&c["aud"]), c["now"].as_i64().unwrap());
        let got = got.map(|t| serde_json::to_value(t).unwrap()).unwrap_or(Value::Null);
        assert_eq!(got, c["result"], "case {c}");
    }
}

#[test]
fn internal_token_and_roles() {
    let g = golden("auth.json");
    let secret = g["secret"].as_str().unwrap();
    assert_eq!(INTERNAL_HEADER, g["header"].as_str().unwrap());
    assert_eq!(internal_token_with(secret), g["internalToken"].as_str().unwrap());
    for c in g["isInternal"].as_array().unwrap() {
        let v = match &c["value"] {
            Value::Array(a) => a.first().and_then(Value::as_str),
            v => v.as_str(),
        };
        assert_eq!(is_internal_with(secret, v), c["result"].as_bool().unwrap(), "{c}");
    }
    for c in g["atLeast"].as_array().unwrap() {
        assert_eq!(at_least(role(&c["role"]), role(&c["min"]).unwrap()), c["result"].as_bool().unwrap(), "{c}");
    }
    for c in g["maxRole"].as_array().unwrap() {
        assert_eq!(max_role(role(&c["a"]), role(&c["b"])), role(&c["result"]), "{c}");
    }
}
