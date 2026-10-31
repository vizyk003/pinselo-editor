//! Parity with tests/golden/platform/mail.json: every template × locale renders byte-identical subject/HTML/text.
use oc_mail::*;
use serde_json::Value;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

#[test]
fn render_matches_ts() {
    let g = golden("mail.json");
    let cases = g["cases"].as_array().unwrap();
    assert!(cases.len() > 300);
    for c in cases {
        let t: Template = serde_json::from_value(c["template"].clone()).unwrap_or_else(|e| panic!("{e}: {}", c["template"]));
        // Round-trips to the TS JSON shape.
        assert_eq!(serde_json::to_value(&t).unwrap(), c["template"]);
        let locale = c["locale"].as_str().unwrap();
        let want: RenderedEmail = serde_json::from_value(c["out"].clone()).unwrap();
        let got = render_email(&t, locale);
        assert_eq!(got.subject, want.subject, "subject {} {locale}", c["template"]["kind"]);
        assert_eq!(got.text, want.text, "text {} {locale}", c["template"]["kind"]);
        assert_eq!(got.html, want.html, "html {} {locale}", c["template"]["kind"]);
    }
}

#[test]
fn translator_matches_ts() {
    let g = golden("mail.json");
    for c in g["translate"].as_array().unwrap() {
        let vars: Vec<(String, String)> = c["vars"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned())).collect();
        let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(translator(c["locale"].as_str().unwrap()).tr(c["s"].as_str().unwrap(), &vars), c["out"].as_str().unwrap());
    }
}
