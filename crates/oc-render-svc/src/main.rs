// SPDX-License-Identifier: Apache-2.0
//! `oc-render-svc`: drop-in for services/render (same port, routes, queue and environment).

use std::sync::Arc;

use oc_jobs::{JobContext, WorkOpts, heartbeat, work};
use oc_render_svc::app::{AppState, ExportJob, router, run_export};
use oc_render_svc::oc_renderer::OcRenderer;
use oc_render_svc::renderer::Renderer;
use oc_render_svc::service::{Service, Settings};
use serde_json::Value;

#[tokio::main]
async fn main() {
    let cfg = oc_config::config();
    cfg.assert_production();
    let pool = oc_db::db().clone();
    let objects = oc_storage::objects().await.clone();
    let settings = Settings::from_config(cfg);
    let fonts =
        oc_gfonts::GoogleFonts::new(settings.fonts_dir.clone(), pool.clone(), objects.clone());
    let manifest = match fonts.full_manifest().await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[render] font manifest: {e}");
            std::process::exit(1);
        }
    };
    // The painter is built over the same manifest the host serves fonts from.
    let painter = OcRenderer::with_manifest(manifest.clone());
    let renderer: Arc<dyn Renderer> = Arc::new(painter);
    let svc = Arc::new(Service::new(
        pool.clone(),
        objects,
        renderer,
        fonts,
        manifest,
        settings,
    ));

    let s = svc.clone();
    let _export = work(
        pool.clone(),
        "export",
        move |p: Value, ctx: JobContext| {
            let s = s.clone();
            async move {
                let job: ExportJob = serde_json::from_value(p)?;
                run_export(&s, &job, &ctx)
                    .await
                    .map_err(|e| -> oc_jobs::HandlerError { e.to_string().into() })
            }
        },
        WorkOpts { concurrency: 1 },
    );
    let _hb = heartbeat(pool.clone(), "render", || serde_json::json!({}));

    // Internal routes require the service token; bind wider only inside a private network (compose).
    let host = std::env::var("OC_RENDER_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let listener =
        match tokio::net::TcpListener::bind((host.as_str(), cfg.render_port as u16)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[render] cannot listen on {host}:{}: {e}", cfg.render_port);
                std::process::exit(1);
            }
        };
    println!("[render] listening on :{}", cfg.render_port);
    let app = router(AppState::new(svc, cfg.secret.clone()));
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        eprintln!("[render] {e}");
        std::process::exit(1);
    }
}

/// SIGINT / SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
