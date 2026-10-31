// SPDX-License-Identifier: Apache-2.0
//! The HTTP service end to end: routing and the internal-token guard (no database), and — with
//! `OC_DATABASE_URL` set — every route and the `export` job on a throwaway `oc_rust_test_*` database with the real
//! oc-render painter and the bundled fonts.

mod common;

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use oc_render_svc::app::{AppState, router};
use oc_render_svc::oc_renderer::OcRenderer;
use oc_render_svc::service::{Service, Settings};
use oc_storage::{LocalStore, ObjectStore};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "render-svc-test-secret-0123456789abcdef";
const WEB: &str = "https://app.example";

fn manifest() -> oc_gfonts::manifest::FontManifest {
    let p = common::root().join("fixtures/fonts/manifest.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("manifest")).expect("manifest json")
}

fn service(pool: sqlx::PgPool, dir: &std::path::Path) -> Arc<Service> {
    let objects = ObjectStore::Local(LocalStore::new(dir.join("objects")));
    let fonts_dir = common::root().join("fixtures/fonts");
    let fonts = oc_gfonts::GoogleFonts::new(fonts_dir.clone(), pool.clone(), objects.clone());
    let m = manifest();
    let settings = Settings {
        web_origin: WEB.into(),
        usercontent_origin: String::new(),
        brand_name: "Pinselo".into(),
        fonts_dir,
        icc_dir: common::root().join("fixtures/icc"),
    };
    Arc::new(Service::new(
        pool,
        objects,
        Arc::new(OcRenderer::with_manifest(m.clone())),
        fonts,
        m,
        settings,
    ))
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "oc-render-svc-{tag}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Res {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&self.body)))
    }
    fn header(&self, k: &str) -> &str {
        self.headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Res {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header(oc_auth::INTERNAL_HEADER, t);
    }
    let req = req
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let headers = r.headers().clone();
    let body = r.into_body().collect().await.unwrap().to_bytes().to_vec();
    Res {
        status,
        headers,
        body,
    }
}

#[tokio::test]
async fn health_and_internal_token_guard() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://localhost:1/none")
        .unwrap();
    let dir = tmp("guard");
    let app = router(AppState::new(service(pool, &dir), SECRET.into()));
    let r = call(&app, "GET", "/healthz", None, Value::Null).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), json!({ "ok": true }));
    for path in [
        "/internal/thumbnail",
        "/internal/doc-to-design",
        "/internal/page-png",
        "/internal/site-build",
        "/internal/template-thumbnail",
        "/internal/export",
        "/internal/icc/inspect",
        "/internal/print/gamut",
    ] {
        let r = call(&app, "POST", path, None, json!({ "designId": "x" })).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{path} without token");
        assert_eq!(r.json(), json!({ "error": "forbidden" }));
        let r = call(&app, "POST", path, Some("nope"), json!({ "designId": "x" })).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{path} with a wrong token");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut v = Vec::new();
            f.read_to_end(&mut v).unwrap();
            (f.name().to_string(), v)
        })
        .collect()
}

/// A Docs design's stored state: design meta plus the doc body of a golden case.
fn doc_state(case: &str) -> Vec<u8> {
    let g = common::golden("docs.json");
    let c = g["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == case)
        .unwrap();
    let mut d = oc_schema::factory::empty_design(&oc_schema::factory::MetaInit {
        w: 816.0,
        h: 1056.0,
        title: Some("Quarterly".into()),
        pages: Some(1),
        ..Default::default()
    });
    d.meta.surface = serde_json::from_value(json!("doc")).unwrap();
    let ydoc = oc_crdt::codec::new_doc();
    oc_crdt::to_y(&d, &ydoc, None).unwrap();
    oc_crdt::apply_update(&ydoc, &common::unb64(c["state"].as_str().unwrap()), None).unwrap();
    oc_crdt::encode_state(&ydoc)
}

#[tokio::test]
async fn every_route_against_a_database() {
    let Some(db) = oc_db::testing::TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping the database-backed service test");
        return;
    };
    let dir = tmp("db");
    let res = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(scenario(
        db.pool.clone(),
        &dir,
    )))
    .await;
    // Always leave nothing behind.
    db.drop_db().await;
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(p) = res {
        std::panic::resume_unwind(p);
    }
}

async fn scenario(pool: sqlx::PgPool, dir: &std::path::Path) {
    let user = uuid::Uuid::new_v4();
    let ws = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users(id, email, name, color) VALUES ($1, 'r@example.com', 'R', '#000')",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO workspaces(id, name) VALUES ($1, 'W')")
        .bind(ws)
        .execute(&pool)
        .await
        .unwrap();
    let design = |id: &'static str, surface: &'static str, title: &'static str, state: Vec<u8>| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO designs(id, workspace_id, owner_id, title, surface, width, height) VALUES ($1, $2, $3, $4, $5, 100, 100)").bind(id).bind(ws).bind(user).bind(title).bind(surface).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO doc_state(design_id, state) VALUES ($1, $2)")
                .bind(id)
                .bind(state)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    design(
        "rdoc000000000000",
        "doc",
        "Quarterly report",
        doc_state("everything"),
    )
    .await;
    let card = common::design("fixture:card-business-clean");
    design(
        "rcard00000000000",
        "print",
        "Clean card",
        oc_crdt::encode_doc(&card).unwrap(),
    )
    .await;
    let site = common::design("kitchen-sink");
    design(
        "rsite00000000000",
        "website",
        "Kitchen",
        oc_crdt::encode_doc(&site).unwrap(),
    )
    .await;
    let deck = common::design("fixture:vd-43-sales");
    design(
        "rdeck00000000000",
        "presentation",
        "Deck",
        oc_crdt::encode_doc(&deck).unwrap(),
    )
    .await;
    sqlx::query("INSERT INTO templates(id, title, category, preset, width, height, doc) VALUES ('tpl1', 'T', 'c', 'p', 336, 192, $1)").bind(sqlx::types::Json(common::design_json("fixture:card-business-clean"))).execute(&pool).await.unwrap();

    let svc = service(pool.clone(), dir);
    let mut st = AppState::new(svc.clone(), SECRET.into());
    st.thumb_delay = Duration::from_millis(10);
    let app = router(st);
    let tok = oc_auth::internal_token_with(SECRET);
    let t = Some(tok.as_str());
    let export = |id: &str, format: &str| json!({ "designId": id, "format": format, "scale": 1, "quality": 90, "transparent": false });

    // Docs: Markdown (byte-identical to the TS golden), Word, wrong surface, missing design.
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rdoc000000000000", "md"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.header("content-disposition"),
        "attachment; filename=\"Quarterly report.md\""
    );
    let want = common::golden("docs.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "everything")
        .unwrap()["markdown"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(String::from_utf8(r.body).unwrap(), want);
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rdoc000000000000", "docx"),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let parts = unzip(&r.body);
    let doc_xml = String::from_utf8(
        parts
            .iter()
            .find(|p| p.0 == "word/document.xml")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert!(doc_xml.contains("Quarterly report") && doc_xml.contains("Embedded design"));
    assert!(
        parts.iter().any(|p| p.0.starts_with("word/media/")),
        "the chart is rendered into the .docx"
    );
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "md"),
    )
    .await;
    assert_eq!(
        (r.status, r.json()["error"].clone()),
        (StatusCode::BAD_REQUEST, json!("not_a_doc"))
    );
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("missing000000000", "png"),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "POST", "/internal/export", t, json!({ "designId": "rcard00000000000", "format": "png", "scale": 1, "quality": 90, "transparent": false, "pages": [7] })).await;
    assert_eq!(
        (r.status, r.json()["error"].clone()),
        (StatusCode::BAD_REQUEST, json!("no_pages"))
    );

    // Rasters, vectors, packages.
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "png"),
    )
    .await;
    assert_eq!(
        (r.status, r.header("content-type")),
        (StatusCode::OK, "image/png")
    );
    assert_eq!(&r.body[..4], b"\x89PNG");
    assert_eq!(
        r.header("content-disposition"),
        "attachment; filename=\"Clean Business Card.png\""
    );
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "jpg"),
    )
    .await;
    assert_eq!(&r.body[..2], &[0xff, 0xd8]);
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rdeck00000000000", "zip"),
    )
    .await;
    let files = unzip(&r.body);
    assert_eq!(files.len(), deck.pages.len());
    assert_eq!(files[0].0, "Quarterly Sales Review 43-01.png");
    for f in ["pdf", "pdf-print", "pdf-flat"] {
        let r = call(
            &app,
            "POST",
            "/internal/export",
            t,
            export("rdeck00000000000", f),
        )
        .await;
        assert_eq!(
            r.status,
            StatusCode::OK,
            "{f}: {}",
            String::from_utf8_lossy(&r.body)
        );
        assert_eq!(&r.body[..5], b"%PDF-", "{f}");
    }
    // Print PDFs: PDF/X-4 and PDF/X-1a with the bundled FOGRA51 profile (when `pnpm icc:sync` ran).
    if common::root().join("fixtures/icc/PSOcoated_v3.icc").exists() {
        for (std, label) in [("pdfx4", oc_render::pdf::PdfStandard::PdfX4), ("pdfx1a", oc_render::pdf::PdfStandard::PdfX1a)] {
            let mut body = export("rdeck00000000000", "pdf-print");
            body["pages"] = json!([0]);
            body["print"] = json!({ "bleedMm": 3, "cropMarks": true, "registrationMarks": true, "colorBars": true, "standard": std, "profile": "pso-coated-v3", "intent": "relative" });
            let r = call(&app, "POST", "/internal/export", t, body).await;
            assert_eq!(r.status, StatusCode::OK, "{std}: {}", String::from_utf8_lossy(&r.body));
            let rep = oc_render::pdfx_check::check(&r.body, label);
            assert!(rep.issues.is_empty(), "{std}: {:#?}", rep.issues);
            assert_eq!(rep.output_condition.as_deref(), Some("FOGRA51"));
        }
    }
    let mut body = export("rdeck00000000000", "pdf-print");
    body["print"] = json!({ "bleedMm": 30, "cropMarks": true });
    let r = call(&app, "POST", "/internal/export", t, body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "bleed out of range");
    let mut body = export("rdeck00000000000", "pdf-print");
    body["print"] = json!({ "bleedMm": 3, "cropMarks": true, "standard": "pdfx4", "profile": "nope" });
    let r = call(&app, "POST", "/internal/export", t, body).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "unknown profile");
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "svg"),
    )
    .await;
    assert!(String::from_utf8(r.body).unwrap().starts_with("<svg"));
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rdeck00000000000", "pptx"),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    assert!(
        unzip(&r.body)
            .iter()
            .any(|p| p.0 == "ppt/slides/slide1.xml")
    );
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "html-email"),
    )
    .await;
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains("/api/email-images/"), "{html}");
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM email_images")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n.0 > 0);
    // Video, GIF and audio: a static card as GIF and MP4 (its settled frame reused), no audio in it; the reel (a
    // video element and a soundtrack over stored playback proxies) as GIF and M4A, and as MP4 in the job below.
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "gif"),
    )
    .await;
    assert_eq!(
        (r.status, &r.body[..6], r.header("content-type")),
        (StatusCode::OK, &b"GIF89a"[..], "image/gif"),
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rcard00000000000", "m4a"),
    )
    .await;
    assert_eq!(
        (r.status, r.json()),
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": "no_audio", "message": "This design has no audio." })
        )
    );
    let ffmpeg = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    let media = common::root().join("tests/golden/render-video/media");
    for (id, file, mime, kind, w, h, audio, ms) in [
        (
            "clip",
            "clip.mp4",
            "video/mp4",
            "video",
            160,
            90,
            true,
            4000,
        ),
        ("music", "music.m4a", "audio/mp4", "audio", 0, 0, true, 6000),
    ] {
        let sha = svc
            .put(&std::fs::read(media.join(file)).unwrap(), mime)
            .await
            .unwrap();
        sqlx::query("INSERT INTO assets(id, workspace_id, owner_id, sha256, mime, kind, bytes, width, height, name, proxy_sha256, proxy_mime, has_audio, duration_ms) VALUES ($1, $2, $3, $4, $5, $6, 1, $7, $8, $9, $4, $5, $10, $11)")
            .bind(id).bind(ws).bind(user).bind(&sha).bind(mime).bind(kind).bind(w).bind(h).bind(file).bind(audio).bind(ms)
            .execute(&pool).await.unwrap();
    }
    let reels: Value = serde_json::from_str(
        &std::fs::read_to_string(common::root().join("tests/golden/render-video/designs.json"))
            .unwrap(),
    )
    .unwrap();
    let reel: oc_schema::DesignDoc = serde_json::from_value(reels["real-reel"].clone()).unwrap();
    design(
        "rreel00000000000",
        "video",
        "Reel",
        oc_crdt::encode_doc(&reel).unwrap(),
    )
    .await;
    let media_of = svc.media_for(&reel).await.unwrap();
    assert_eq!(
        (
            media_of["clip"].w,
            media_of["clip"].h,
            media_of["music"].w,
            media_of["music"].duration_ms
        ),
        (160, 90, 2, 6000.0)
    );
    assert!(std::path::Path::new(&media_of["clip"].file).exists());
    let r = call(
        &app,
        "POST",
        "/internal/export",
        t,
        export("rreel00000000000", "gif"),
    )
    .await;
    assert_eq!((r.status, &r.body[..6]), (StatusCode::OK, &b"GIF89a"[..]));
    if ffmpeg {
        let r = call(
            &app,
            "POST",
            "/internal/export",
            t,
            export("rcard00000000000", "mp4"),
        )
        .await;
        assert_eq!(
            (r.status, &r.body[4..8], r.header("content-type")),
            (StatusCode::OK, &b"ftyp"[..], "video/mp4"),
            "{}",
            String::from_utf8_lossy(&r.body)
        );
        let r = call(
            &app,
            "POST",
            "/internal/export",
            t,
            export("rreel00000000000", "m4a"),
        )
        .await;
        assert_eq!(
            (r.status, &r.body[4..8], r.header("content-disposition")),
            (
                StatusCode::OK,
                &b"ftyp"[..],
                "attachment; filename=\"real.m4a\""
            ),
            "{}",
            String::from_utf8_lossy(&r.body)
        );
    } else {
        eprintln!("ffmpeg not installed: skipping the MP4/M4A routes");
    }

    // Docs → design pages; page PNG; website; template thumbnail; coalesced thumbnail.
    let r = call(
        &app,
        "POST",
        "/internal/doc-to-design",
        t,
        json!({ "designId": "rdoc000000000000", "embeds": [] }),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let d = r.json();
    assert_eq!(d["doc"]["meta"]["surface"], "print");
    assert!(
        d["doc"]["pages"].as_object().unwrap().len() >= 2,
        "the page break starts a second page"
    );
    let texts: Vec<String> = d["doc"]["elements"]
        .as_object()
        .unwrap()
        .values()
        .filter(|e| e["type"] == "text")
        .map(|e| {
            e["body"]["paragraphs"][0]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["text"].as_str().unwrap())
                .collect()
        })
        .collect();
    assert!(texts.iter().any(|t| t == "Quarterly report") && texts.iter().any(|t| t == "Contents"));
    let r = call(
        &app,
        "POST",
        "/internal/page-png",
        t,
        json!({ "designId": "rcard00000000000", "w": 200 }),
    )
    .await;
    assert_eq!((r.status, &r.body[..4]), (StatusCode::OK, &b"\x89PNG"[..]));
    let r = call(
        &app,
        "POST",
        "/internal/page-png",
        t,
        json!({ "designId": "missing000000000" }),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "POST", "/internal/site-build", t, json!({ "designId": "rsite00000000000", "title": "Kitchen", "assetOrigin": "https://cdn.example" })).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let html = r.json()["html"].as_str().unwrap().to_string();
    assert!(
        html.contains("<section id=\"p1\" aria-label=\"Home\" class=\"rf\"")
            && html.contains("<svg aria-hidden=\"true\" focusable=\"false\"")
    );
    assert!(
        html.contains(
            "og:image\" content=\"https://app.example/api/sites/og/rsite00000000000.png\""
        )
    );
    let r = call(
        &app,
        "POST",
        "/internal/template-thumbnail",
        t,
        json!({ "templateId": "tpl1" }),
    )
    .await;
    assert_eq!(r.json(), json!({ "ok": true }));
    let th: (Option<String>,) = sqlx::query_as("SELECT thumbnail FROM templates WHERE id = 'tpl1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(th.0.is_some());
    let r = call(
        &app,
        "POST",
        "/internal/template-thumbnail",
        t,
        json!({ "templateId": "nope" }),
    )
    .await;
    assert_eq!(r.json(), json!({ "ok": false }));
    for _ in 0..3 {
        let r = call(
            &app,
            "POST",
            "/internal/thumbnail",
            t,
            json!({ "designId": "rcard00000000000" }),
        )
        .await;
        assert_eq!(r.json(), json!({ "queued": true }));
    }
    let mut thumb = None;
    for _ in 0..200 {
        let r: (Option<String>,) =
            sqlx::query_as("SELECT thumb_sha256 FROM designs WHERE id = 'rcard00000000000'")
                .fetch_one(&pool)
                .await
                .unwrap();
        if r.0.is_some() {
            thumb = r.0;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let thumb = thumb.expect("thumbnail written");
    assert_eq!(&svc.objects.get(&thumb).await.unwrap()[..4], b"\x89PNG");

    // The durable export queue: a background PDF and a Word download.
    let worker = oc_jobs::work(
        pool.clone(),
        "export",
        {
            let s = svc.clone();
            move |p: Value, ctx: oc_jobs::JobContext| {
                let s = s.clone();
                async move {
                    let job: oc_render_svc::app::ExportJob = serde_json::from_value(p)?;
                    oc_render_svc::app::run_export(&s, &job, &ctx)
                        .await
                        .map_err(|e| -> oc_jobs::HandlerError { e.to_string().into() })
                }
            }
        },
        oc_jobs::WorkOpts { concurrency: 1 },
    );
    let pdf = oc_jobs::enqueue(&pool, "export", &json!({ "designId": "rdeck00000000000", "userId": user.to_string(), "format": "pdf-print", "print": { "bleedMm": 5, "cropMarks": false, "standard": if common::root().join("fixtures/icc/PSOcoated_v3.icc").exists() { "pdfx4" } else { "none" } } }), oc_jobs::EnqueueOpts::default()).await.unwrap();
    let word = oc_jobs::enqueue(
        &pool,
        "export",
        &json!({ "designId": "rdoc000000000000", "userId": "", "format": "docx" }),
        oc_jobs::EnqueueOpts::default(),
    )
    .await
    .unwrap();
    let video = oc_jobs::enqueue(&pool, "export", &json!({ "designId": "rreel00000000000", "userId": user.to_string(), "format": if ffmpeg { "mp4" } else { "gif" }, "quality": "720p" }), oc_jobs::EnqueueOpts::default()).await.unwrap();
    for (id, ty, name) in [
        (pdf, "application/pdf", "Quarterly Sales Review 43.pdf"),
        (
            word,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "Quarterly report.docx",
        ),
        if ffmpeg {
            (video, "video/mp4", "real.mp4")
        } else {
            (video, "image/gif", "real.gif")
        },
    ] {
        let mut job = None;
        for _ in 0..6000 {
            let j = oc_jobs::get_job(&pool, id).await.unwrap().unwrap();
            if j.finished_at.is_some() {
                job = Some(j);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let job = job.expect("job finished");
        let res = job.result.expect("job result").0;
        assert_eq!(
            (
                res["type"].as_str().unwrap(),
                res["filename"].as_str().unwrap()
            ),
            (ty, name),
            "{:?}",
            job.error
        );
        assert_eq!(
            svc.objects
                .get(res["sha"].as_str().unwrap())
                .await
                .unwrap()
                .len() as u64,
            res["size"].as_u64().unwrap()
        );
    }
    // The video job reported its frames while rendering.
    let progress: (f32,) = sqlx::query_as("SELECT progress FROM jobs WHERE id = $1")
        .bind(video)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(progress.0 > 0.0, "video job progress {}", progress.0);
    worker.stop().await;
    drop(app);
    drop(svc);
    pool.close().await;
}

/// The colour-profile routes need no database: ICC validation and gamut checks against a bundled profile.
#[tokio::test]
async fn icc_inspect_and_gamut() {
    let pool = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect_lazy("postgres://localhost:1/none").unwrap();
    let dir = tmp("icc");
    let app = router(AppState::new(service(pool, &dir), SECRET.into()));
    let tok = oc_auth::internal_token_with(SECRET);
    let raw = |bytes: Vec<u8>| {
        Request::builder().method("POST").uri("/internal/icc/inspect").header("content-type", "application/vnd.iccprofile").header(oc_auth::INTERNAL_HEADER, tok.as_str()).body(Body::from(bytes)).unwrap()
    };
    let r = app.clone().oneshot(raw(b"garbage".to_vec())).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let icc = common::root().join("fixtures/icc/PSOcoated_v3.icc");
    let Ok(bytes) = std::fs::read(&icc) else {
        eprintln!("skipping: run `pnpm icc:sync`");
        return;
    };
    let r = app.clone().oneshot(raw(bytes)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!((v["class"].as_str(), v["colorSpace"].as_str(), v["channels"].as_u64()), (Some("prtr"), Some("CMYK"), Some(4)));
    let t = Some(tok.as_str());
    let body = json!({ "profile": "pso-coated-v3", "intent": "relative", "colors": [
        { "space": "srgb", "r": 0, "g": 1, "b": 0, "a": 1 },
        { "space": "srgb", "r": 0.5, "g": 0.5, "b": 0.5, "a": 1 },
        { "space": "cmyk", "c": 1, "m": 0, "y": 0, "k": 0, "a": 1 }
    ] });
    let r = call(&app, "POST", "/internal/print/gamut", t, body).await;
    assert_eq!(r.status, StatusCode::OK);
    let res = r.json()["results"].clone();
    assert_eq!(res[0]["inGamut"], json!(false));
    assert_eq!(res[1]["inGamut"], json!(true));
    assert_eq!(res[2]["cmyk"], json!([100.0, 0.0, 0.0, 0.0]));
    let r = call(&app, "POST", "/internal/print/gamut", t, json!({ "profile": "nope", "colors": [] })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = call(&app, "POST", "/internal/print/gamut", t, json!({ "colors": 1 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let _ = std::fs::remove_dir_all(dir);
}
