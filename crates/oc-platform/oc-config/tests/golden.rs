//! Parity with tests/golden/platform/config.json (generated from packages/config by tools/golden/platform.ts).
use std::collections::HashMap;

use oc_config::Config;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

#[test]
fn config_matches_ts() {
    let cases = golden("config.json");
    let cases = cases.as_array().unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let env: HashMap<String, String> = serde_json::from_value(case["env"].clone()).unwrap();
        let want = &case["config"];
        let has_nan = want.as_object().unwrap().values().any(Value::is_null);
        match Config::from_map(&env) {
            Err(e) => assert!(has_nan, "unexpected error {e} for {env:?}"),
            Ok(c) => {
                assert!(!has_nan, "TS produced NaN for {env:?} but Rust accepted it");
                assert_eq!(&serde_json::to_value(&c).unwrap(), want, "config for {env:?}");
                let refusal = case.get("refusal").and_then(Value::as_str).map(str::to_owned);
                assert_eq!(c.startup_refusal(), refusal, "refusal for {env:?}");
                let warnings: Vec<String> = case.get("warnings").map(|w| serde_json::from_value(w.clone()).unwrap()).unwrap_or_default();
                let problems: Vec<String> = serde_json::from_value(case["problems"].clone()).unwrap();
                if refusal.is_none() {
                    // The TS exits on the refusal before it warns or lists production problems.
                    assert_eq!(c.startup_warnings(), warnings, "warnings for {env:?}");
                    assert_eq!(c.production_problems(), problems, "production problems for {env:?}");
                }
                assert_eq!(c.supabase_auth_enabled(), case["supabase"]["enabled"].as_bool().unwrap(), "supabase enabled for {env:?}");
                assert_eq!(c.supabase_issuer(), case["supabase"]["issuer"].as_str().unwrap(), "supabase issuer for {env:?}");
            }
        }
    }
}
