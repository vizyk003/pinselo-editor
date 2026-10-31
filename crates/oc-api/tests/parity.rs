//! HTTP parity with services/api (ADR-0001): every transcript in tests/golden/api/ (written by
//! tools/golden/api.ts from the TS Fastify app) replayed against the Rust router. Adding a route group takes one
//! scenario file (tools/golden/api/groups/<group>.ts) and one line here.
//!
//! Needs `OC_DATABASE_URL` (a Postgres server; each group creates and drops its own `oc_rust_test_*`
//! database) and, for the media steps, ffmpeg on PATH; skipped otherwise.

mod harness;

macro_rules! groups {
    ($($name:ident => $file:literal),* $(,)?) => {$(
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            harness::run($file).await;
        }
    )*};
}

groups! {
    framework => "framework",
    account => "account",
    designs => "designs",
    assets => "assets",
    oidc => "oidc",
    teams => "teams",
    growth => "growth",
    graphics => "graphics",
    content => "content",
    print => "print",
    print_profiles => "print-profiles",
    forms => "forms",
    dev => "dev",
    dev_smtp => "dev-smtp",
    sheets => "sheets",
    embeds => "embeds",
    search => "search",
    billing => "billing",
    misc => "misc",
    v1 => "v1",
    safety => "safety",
    brandhub => "brandhub",
    sites => "sites",
    social => "social",
    imports => "imports",
    docs => "docs",
    projects => "projects",
    sharing => "sharing",
    uploads => "uploads",
    supabase => "supabase",
    supabase_legacy => "supabase-legacy",
    oauth => "oauth",
    mcp => "mcp",
}
