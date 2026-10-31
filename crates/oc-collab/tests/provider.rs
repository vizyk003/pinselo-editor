//! End-to-end protocol conformance: the real browser client stack (`@hocuspocus/provider` 3.4.4 + yjs)
//! against the `oc-collab` binary, driven by tests/collab/provider-client.ts.
//!
//! Needs `OC_DATABASE_URL` (any database on the target server; only its `postgres` maintenance database is
//! used to create and drop a throwaway `oc_rust_test_<uuid>` database), `pnpm` and the workspace's node
//! modules. Skipped (passes) without `OC_DATABASE_URL`. The driver starts, SIGKILLs and SIGTERMs the server
//! itself, stands in for the render service (it sets `OC_RENDER_URL` to a recorder that answers 503) and exits
//! non-zero when any scenario fails; see its header for the scenario list (16 scenarios, ~25 s).
//!
//! `OC_COLLAB_SERVER_CMD` runs the scenarios against another server command. Without cargo, against any migrated
//! throwaway database and a free port:
//!
//! ```text
//! SERVER_CMD=target/debug/oc-collab \
//!   OC_DATABASE_URL=postgres://localhost:5432/oc_rust_test_<id> OC_COLLAB_PORT=<port> \
//!   OC_SECRET=collab-conformance-secret-0123456789abcdef NODE_ENV=development pnpm tsx tests/collab/provider-client.ts
//! ```

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Stdio;

use oc_db::testing::TestDb;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test(flavor = "multi_thread")]
async fn hocuspocus_provider_conformance() {
    let Some(db) = TestDb::create().await else {
        eprintln!("OC_DATABASE_URL not set: skipping");
        return;
    };
    let root = repo_root();
    let server_cmd = std::env::var("OC_COLLAB_SERVER_CMD").unwrap_or_else(|_| env!("CARGO_BIN_EXE_oc-collab").to_owned());
    let port = free_port();
    let status = tokio::process::Command::new("pnpm")
        .args(["tsx", "tests/collab/provider-client.ts"])
        .current_dir(&root)
        .env("SERVER_CMD", &server_cmd)
        .env("OC_DATABASE_URL", &db.url)
        .env("OC_COLLAB_PORT", port.to_string())
        .env("OC_SECRET", "collab-conformance-secret-0123456789abcdef")
        .env("NODE_ENV", "development")
        .stdin(Stdio::null())
        .status()
        .await
        .expect("run pnpm tsx tests/collab/provider-client.ts");
    db.drop_db().await;
    assert!(status.success(), "provider-client.ts reported failures ({status})");
}
