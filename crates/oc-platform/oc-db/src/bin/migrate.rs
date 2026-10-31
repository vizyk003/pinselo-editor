//! `oc-migrate`: the Rust equivalent of `pnpm db:migrate`. Applies pending migrations to OC_DATABASE_URL (see
//! oc_db::migrate for the by-name rule and the Supabase lockdown), then prints the exposure report. Exits 1 when a
//! migration failed, and in production when the Supabase Data API roles can still reach anything.
use oc_db::checks::{CheckContext, check_database_exposure, report_exposure};

#[tokio::main]
async fn main() {
    oc_db::migrate::print_server_warnings();
    let cfg = oc_config::config();
    let pool = oc_db::lazy_pool(&cfg.database_url).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let mut failed = false;
    if let Err(e) = oc_db::migrate::migrate_with(&pool, &oc_db::migrations_dir(), |f| println!("applied {f}")).await {
        failed = true;
        eprintln!("{e}");
    }
    // After every run, a failed one too: whatever it committed is reported.
    match check_database_exposure(&pool).await {
        Ok(r) => {
            if report_exposure("migrate", &r, CheckContext::Migrate, cfg.env == "production") {
                failed = true;
            }
        }
        Err(e) => {
            failed = true;
            eprintln!("[migrate] exposure check failed: {e}");
        }
    }
    pool.close().await;
    if failed {
        std::process::exit(1);
    }
}
