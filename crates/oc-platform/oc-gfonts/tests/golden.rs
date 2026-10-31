//! Parity with tests/golden/platform/gfonts.json (slugs, catalogue rows, merged manifest, CSS2 URLs).
use oc_gfonts::*;
use serde_json::Value;
use sha2::Digest;

fn golden(name: &str) -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/");
    serde_json::from_str(&std::fs::read_to_string(format!("{p}{name}")).unwrap()).unwrap()
}

fn fonts_dir() -> std::path::PathBuf {
    oc_config::repo_root().join("fixtures/fonts")
}

#[test]
fn slugs_and_catalogue() {
    let g = golden("gfonts.json");
    for s in g["slugs"].as_array().unwrap() {
        assert_eq!(slug_of(s["family"].as_str().unwrap()), s["slug"].as_str().unwrap());
    }
    let meta: GoogleMetadata = serde_json::from_value(g["metadata"].clone()).unwrap();
    assert_eq!(serde_json::to_value(catalog_from_metadata(&meta)).unwrap(), g["catalog"]);
}

#[test]
fn full_manifest_is_byte_identical() {
    let g = golden("gfonts.json");
    let dir = fonts_dir();
    let pinned: manifest::FontManifest = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(pinned.families.len() as u64, g["manifest"]["pinnedCount"].as_u64().unwrap());
    let m = merge_manifest(pinned, &load_catalog(&dir));
    assert_eq!(m.families.len() as u64, g["manifest"]["totalCount"].as_u64().unwrap());
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(hex::encode(sha2::Sha256::digest(json.as_bytes())), g["manifest"]["sha256"].as_str().unwrap());
    let n = g["manifest"]["pinnedCount"].as_u64().unwrap() as usize;
    assert_eq!(serde_json::to_value(&m.families[n..n + 3]).unwrap(), g["manifest"]["firstExtra"]);
}

#[test]
fn preview_urls_match() {
    let g = golden("gfonts.json");
    let catalog = load_catalog(&fonts_dir());
    for p in g["previews"].as_array().unwrap() {
        let fam = catalog.iter().find(|c| c.slug == p["slug"].as_str().unwrap()).unwrap();
        let face = if fam.faces.iter().any(|f| f == "400") { "400" } else { fam.faces[0].as_str() };
        let reqs = p["requests"].as_array().unwrap();
        assert_eq!(css2_preview_url(&fam.family, face), reqs[0]["url"].as_str().unwrap());
        assert_eq!(reqs[0]["ua"], USER_AGENT);
        assert_eq!(reqs[1]["url"], "https://fonts.gstatic.com/l/font?kit=abc&skey=1");
        assert_eq!(extract_ttf_url("@font-face {\n  src: url(https://fonts.gstatic.com/l/font?kit=abc&skey=1) format('truetype');\n}\n").unwrap(), reqs[1]["url"].as_str().unwrap());
    }
    let pinned = golden("gfonts_pinned.json");
    let (slug, w, i) = parse_file(pinned["file"].as_str().unwrap()).unwrap();
    let fam = catalog.iter().find(|c| c.slug == slug).unwrap();
    assert_eq!(css2_face_url(&fam.family, w, i), pinned["requests"][0]["url"].as_str().unwrap());
}

#[tokio::test]
async fn unknown_file_error() {
    let g = golden("gfonts.json");
    let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://localhost:1/none").unwrap();
    let gf = GoogleFonts::new(fonts_dir(), pool, oc_storage::ObjectStore::Local(oc_storage::LocalStore::new(std::env::temp_dir())));
    assert_eq!(gf.font_file("gf/roboto/450.ttf").await.unwrap_err().to_string(), g["unknownFileError"].as_str().unwrap());
    assert_eq!(gf.catalog().len(), load_catalog(&fonts_dir()).len());
}
