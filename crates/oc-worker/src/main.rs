//! `oc-worker`: drop-in for services/worker (same queues, schedules and environment).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use oc_jobs::{JobContext, WorkOpts, every, heartbeat, work};
use oc_social::publish::{JobInfo, RenderService};
use oc_social::{Http, Social, SocialConfig};
use oc_worker::gc::{GcOptions, collect_garbage};
use oc_worker::jobs::{self, JobError};
use oc_worker::DAY;
use serde_json::Value;

fn job_info(ctx: &JobContext) -> JobInfo {
    JobInfo { id: ctx.job.id, attempts: ctx.job.attempts, max_attempts: ctx.job.max_attempts }
}

type Res = Result<Value, oc_jobs::HandlerError>;

fn boxed(r: Result<Value, JobError>) -> Res {
    r.map_err(|e| e.into())
}

#[tokio::main]
async fn main() {
    let cfg = oc_config::config();
    cfg.assert_production();
    let pool = oc_db::db().clone();
    let objects = oc_storage::objects().await.clone();
    let social = Arc::new(Social::new(pool.clone(), Http::new(), objects.clone(), SocialConfig::from_config(cfg)));
    let render = Arc::new(RenderService::new(&social.cfg, std::env::var("OC_FFMPEG").ok()));

    let s = social.clone();
    let _webhook = work(pool.clone(), "webhook", move |p: Value, _ctx: JobContext| {
        let s = s.clone();
        async move { boxed(jobs::webhook(&s, &p).await) }
    }, WorkOpts { concurrency: 4 });
    let s = social.clone();
    let _email = work(pool.clone(), "email", move |p: Value, _ctx: JobContext| {
        let s = s.clone();
        async move { boxed(jobs::email(&s, &p).await) }
    }, WorkOpts { concurrency: 4 });
    let s = social.clone();
    let _export = work(pool.clone(), "account-export", move |p: Value, ctx: JobContext| {
        let s = s.clone();
        async move { boxed(jobs::account_export(&s, &p, job_info(&ctx)).await) }
    }, WorkOpts { concurrency: 1 });
    let s = social.clone();
    let _delete = work(pool.clone(), "account-delete", move |p: Value, _ctx: JobContext| {
        let s = s.clone();
        async move { boxed(jobs::account_delete(&s, &p).await) }
    }, WorkOpts { concurrency: 1 });

    let p = pool.clone();
    let _maintenance = every("maintenance", DAY / 24, move || {
        let p = p.clone();
        async move { jobs::maintenance(&p).await.map(|_| ()) }
    });
    // Content Planner: publish scheduled posts (delayed jobs) + a sweep for anything missed.
    let (s, r) = (social.clone(), render.clone());
    let _publish = work(pool.clone(), "publish", move |p: Value, ctx: JobContext| {
        let (s, r) = (s.clone(), r.clone());
        async move { boxed(jobs::publish(&s, &p, job_info(&ctx), r.as_ref()).await) }
    }, WorkOpts { concurrency: 2 });
    let s = social.clone();
    let _sweep = every("social-sweep", Duration::from_secs(60), move || {
        let s = s.clone();
        async move { jobs::social_sweep(&s).await.map(|_| ()) }
    });
    let (p, o) = (pool.clone(), objects.clone());
    let _gc = every("object-gc", DAY, move || {
        let (p, o) = (p.clone(), o.clone());
        async move {
            let r = collect_garbage(&p, &o, GcOptions::from_env(|k| std::env::var(k).ok()), SystemTime::now()).await?;
            println!("{}", r.log_line());
            Ok::<(), oc_worker::gc::GcError>(())
        }
    });
    let p = pool.clone();
    tokio::spawn(async move {
        if let Err(e) = jobs::maintenance(&p).await {
            eprintln!("[worker] maintenance {e}");
        }
    });
    let _hb = heartbeat(pool.clone(), "worker", || serde_json::json!({}));
    println!("[worker] running: email, webhook, account-export, account-delete, publish, maintenance, object-gc, social-sweep");
    shutdown_signal().await;
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
