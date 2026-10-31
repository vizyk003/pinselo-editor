//! Port of services/api/src/print/fulfil.ts — the `print-fulfil` job: render the order's print file through the
//! render service (the `export` job with format `pdf-print`, the product's bleed, PDF/X-4 with the default CMYK
//! output intent; a PNG at 300 dpi for fulfillers that want images), store it, then hand the order to the
//! fulfiller (Gelato, Printful or the local provider).
//!
//! Worker: the TS API runs `work('print-fulfil', runFulfil, { concurrency: 2 })` in-process, and so does the binary
//! ([`start_worker`]: the same `oc_jobs` loop, woken by the enqueue's NOTIFY). An app state without that loop (the
//! parity harness) drains the queue after each enqueue instead ([`kick`]: `oc_jobs::drain` in a tracked background
//! task, so tests can wait for it). Both use the claim and bookkeeping of `oc_jobs`.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::http::error::ApiError;
use crate::print::catalog::print_catalog;
use crate::print::local::operator_email;
use crate::print::providers::{Provider, provider_for};
use crate::print::store::{Order, OrderExt, Patch, StatusChange, get_order, set_status, signed_file_url};
use crate::print::types::{FulfilmentOrder, print_file_options};
use crate::print::{gelato, local, printful};
use crate::state::AppState;

pub const QUEUE: &str = "print-fulfil";

/// The running job (`ctx.job`): its id and attempt counters.
pub struct JobCtx {
    pub id: Uuid,
    pub attempts: i32,
    pub max_attempts: i32,
}

/// `ctx.progress(fraction, label)`.
async fn progress(s: &AppState, ctx: &JobCtx, fraction: f64, label: &str) -> Result<(), String> {
    sqlx::query("UPDATE jobs SET progress = $2, label = COALESCE($3, label) WHERE id = $1")
        .bind(ctx.id)
        .bind(fraction.clamp(0.0, 1.0) as f32)
        .bind(label)
        .execute(&s.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `enqueue('print-fulfil', { orderId }, { ownerId, maxAttempts: 3, label })`, then drain the queue.
pub async fn enqueue_fulfil(s: &AppState, o: &Order) -> Result<(), ApiError> {
    let owner = o.str_of("user_id").and_then(|u| Uuid::parse_str(u).ok());
    oc_jobs::enqueue(
        &s.pool,
        QUEUE,
        &json!({ "orderId": o.id() }),
        oc_jobs::EnqueueOpts { owner_id: owner, max_attempts: Some(3), label: Some(format!("Print order {}", o.text("number"))), ..Default::default() },
    )
    .await?;
    kick(s);
    Ok(())
}

/// The job handler (`runFulfil`), shared by the worker loop and the in-process drain.
fn handler(s: &AppState) -> impl oc_jobs::Handler {
    let s = s.clone();
    move |payload: Value, ctx: oc_jobs::JobContext| {
        let s = s.clone();
        async move {
            let c = JobCtx { id: ctx.job.id, attempts: ctx.job.attempts, max_attempts: ctx.job.max_attempts };
            let order_id = payload.get("orderId").and_then(Value::as_str).unwrap_or("").to_owned();
            run_fulfil(&s, &order_id, &c).await.map_err(oc_jobs::HandlerError::from)
        }
    }
}

#[derive(Default)]
struct WorkerSlot(OnceLock<oc_jobs::Worker>);

/// After an enqueue: with the worker loop running (the binary), `enqueue`'s NOTIFY already wakes it, as in TS (one
/// claim loop per process, at most 2 jobs at a time; several replicas share the queue through `SKIP LOCKED`).
/// Without one (the parity harness), drain the due jobs in a tracked background task so tests can wait for them.
pub fn kick(s: &AppState) {
    if s.local::<WorkerSlot>().0.get().is_some() {
        return;
    }
    let s2 = s.clone();
    s.spawn(async move {
        oc_jobs::drain(&s2.pool, QUEUE, handler(&s2)).await;
    });
}

/// `work('print-fulfil', runFulfil, { concurrency: 2 })`, once per app state; started by the binary.
pub fn start_worker(s: &AppState) {
    s.local::<WorkerSlot>().0.get_or_init(|| oc_jobs::work(s.pool.clone(), QUEUE, handler(s), oc_jobs::WorkOpts { concurrency: 2 }));
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn api_err(e: ApiError) -> String {
    match e {
        ApiError::Internal(m) => m,
        other => format!("{other:?}"),
    }
}

/// Render through the durable export job and wait for it (the render service may be busy with other exports).
async fn render_pdf(s: &AppState, o: &Order, p: &oc_schema::print::PrintProduct, provider: Provider, ctx: &JobCtx) -> Result<(String, String), String> {
    let owner = o.str_of("user_id").and_then(|u| Uuid::parse_str(u).ok());
    let job = oc_jobs::enqueue(
        &s.pool,
        "export",
        &json!({ "designId": o.get("design_id"), "userId": o.str_of("user_id").unwrap_or(""), "format": "pdf-print", "print": print_file_options(provider, p) }),
        oc_jobs::EnqueueOpts { owner_id: owner, max_attempts: Some(2), label: Some(format!("Print file {}", o.text("number"))), ..Default::default() },
    )
    .await
    .map_err(err)?;
    let deadline = s.now_ms() + 20 * 60_000;
    loop {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let j = oc_jobs::get_job(&s.pool, job).await.map_err(err)?;
        match &j {
            Some(j) if j.status == oc_jobs::JobStatus::Done => {
                let r = j.result.as_ref().map(|r| r.0.clone()).unwrap_or(Value::Null);
                return Ok((r["sha"].as_str().unwrap_or("").to_owned(), r["type"].as_str().unwrap_or("").to_owned()));
            }
            Some(j) if j.status != oc_jobs::JobStatus::Failed && j.status != oc_jobs::JobStatus::Cancelled => {
                progress(s, ctx, (f64::from(j.progress) * 0.6).min(0.6), "Rendering print file").await?;
            }
            _ => return Err(format!("print file render failed: {}", j.and_then(|j| j.error).unwrap_or_else(|| "job missing".into()))),
        }
        if s.now_ms() > deadline {
            return Err("print file render timed out".into());
        }
    }
}

/// PNG of the first page at 300 dpi of the print size (apparel, mugs, stickers at Printful).
async fn render_png(s: &AppState, o: &Order, p: &oc_schema::print::PrintProduct) -> Result<(String, String), String> {
    let d: Option<(f64, f64)> = sqlx::query_as("SELECT width, height FROM designs WHERE id = $1").bind(o.str_of("design_id")).fetch_optional(&s.pool).await.map_err(err)?;
    let size = p.sizes.iter().find(|x| Some(x.id.as_str()) == o.str_of("size_id")).unwrap_or(&p.sizes[0]);
    let Some((w, h)) = d else { return Err("design not found".into()) };
    let scale = (((size.w.max(size.h) / 96.0) * 300.0) / w.max(h)).clamp(0.1, 4.0);
    let body = json!({ "designId": o.get("design_id"), "format": "png", "pages": [0], "scale": scale, "quality": 100, "transparent": true });
    let res = s
        .http
        .post(format!("{}/internal/export", s.cfg.render_url))
        .header("content-type", "application/json")
        .header(oc_auth::INTERNAL_HEADER, oc_auth::internal_token_with(&s.cfg.secret))
        .body(crate::http::jsjson::stringify(&body))
        .send()
        .await
        .map_err(|_| "fetch failed".to_owned())?;
    if !res.status().is_success() {
        let status = res.status().as_u16();
        return Err(format!("render failed: {status} {}", res.text().await.unwrap_or_default()));
    }
    let bytes = res.bytes().await.map_err(err)?;
    let sha = s.store.put(&bytes, Some("image/png")).await.map_err(err)?;
    Ok((sha, "image/png".into()))
}

/// `toFulfilmentOrder(o, orientation)`.
pub fn to_fulfilment_order(o: &Order, orientation: &'static str) -> Result<FulfilmentOrder, String> {
    let options = o.get("options").and_then(Value::as_object).map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_owned())).collect()).unwrap_or_default();
    Ok(FulfilmentOrder {
        id: o.id(),
        number: o.text("number"),
        email: o.text("email"),
        user_id: o.str_of("user_id").map(str::to_owned),
        product_id: o.text("product_id"),
        size_id: o.text("size_id"),
        options,
        quantity: o.num_of("quantity"),
        address: serde_json::from_value(o.get("address").cloned().unwrap_or(Value::Null)).map_err(err)?,
        shipping_method: o.text("shipping_method"),
        currency: o.text("currency"),
        shipping_cents: o.num_of("shipping_cents"),
        tax_cents: o.num_of("tax_cents"),
        orientation,
    })
}

/// `runFulfil({ orderId }, ctx)`: the job handler; the `Err` is the job's error message.
pub async fn run_fulfil(s: &AppState, order_id: &str, ctx: &JobCtx) -> Result<Value, String> {
    let Some(o) = get_order(s, order_id).await.map_err(api_err)? else { return Ok(json!({ "skipped": "missing" })) };
    let st = o.status();
    if st != "paid" && st != "failed" && st != "rendering" {
        return Ok(json!({ "skipped": st }));
    }
    let catalog = print_catalog(s).map_err(api_err)?;
    let p = oc_schema::print::find_product(&catalog, &o.text("product_id"));
    match attempt(s, o, p, ctx).await {
        Ok(v) => Ok(v),
        Err(msg) => {
            let last = ctx.attempts >= ctx.max_attempts;
            if let Ok(Some(cur)) = get_order(s, order_id).await
                && cur.status() == "rendering"
            {
                let _ = set_status(s, &cur.id(), "failed", StatusChange { source: "system", note: Some(msg.clone()), patch: Patch { error: Some(msg.clone()), ..Default::default() }, quiet: !last, ..Default::default() })
                    .await
                    .map_err(err)?;
                if last && let Some(op) = operator_email(s) {
                    let tpl = oc_mail::Template::Decision { subject: format!("Print order {} failed", cur.text("number")), body: format!("{msg}\n\n{}/admin/print", s.cfg.web_origin) };
                    let _ = oc_mail::send_email(&s.pool, &op, &tpl, None).await;
                }
            }
            Err(msg)
        }
    }
}

async fn attempt(s: &AppState, mut o: Order, p: Option<&oc_schema::print::PrintProduct>, ctx: &JobCtx) -> Result<Value, String> {
    let Some(p) = p else { return Err(format!("product {} is no longer in the catalogue", o.text("product_id"))) };
    if o.str_of("design_id").is_none_or(str::is_empty) {
        return Err("the design was deleted".into());
    }
    let provider = o.str_of("provider").and_then(Provider::parse).filter(|x| x.enabled(s)).unwrap_or_else(|| provider_for(s, p));
    if o.status() != "rendering" {
        o = set_status(s, &o.id(), "rendering", StatusChange { source: "system", patch: Patch { provider: Some(provider.id().into()), ..Default::default() }, quiet: true, ..Default::default() })
            .await
            .map_err(err)?;
    }
    progress(s, ctx, 0.05, "Rendering print file").await?;
    let format = provider.file_format();
    let want = if format == "pdf" { "application/pdf" } else { "image/png" };
    let (file_sha, file_type) = match (o.str_of("file_sha").filter(|x| !x.is_empty()), o.str_of("file_type")) {
        (Some(sha), Some(t)) if t == want => (sha.to_owned(), t.to_owned()),
        _ if format == "pdf" => render_pdf(s, &o, p, provider, ctx).await?,
        _ => render_png(s, &o, p).await?,
    };
    let dims: Option<(f64, f64)> = sqlx::query_as("SELECT width, height FROM designs WHERE id = $1").bind(o.str_of("design_id")).fetch_optional(&s.pool).await.map_err(err)?;
    let orientation = if dims.is_some_and(|(w, h)| w > h) { "hor" } else { "ver" };
    let url = signed_file_url(s, &o.id(), format);
    progress(s, ctx, 0.7, "Sending to the printer").await?;
    let fo = to_fulfilment_order(&o, orientation)?;
    let r = match provider {
        Provider::Gelato => gelato::submit(s, &fo, p, &url).await?,
        Provider::Printful => printful::submit(s, &fo, p, &url).await?,
        Provider::Local => local::submit(s, &fo, p, &url).await.map_err(api_err)?,
    };
    let fin = if r.status == "submitted" || r.status == "in_production" { r.status.as_str() } else { "submitted" };
    // rendering → submitted (→ in_production when the fulfiller already started).
    o = set_status(
        s,
        &o.id(),
        "submitted",
        StatusChange {
            source: "system",
            note: Some(r.note.clone().unwrap_or_else(|| format!("Sent to {}.", provider.id()))),
            patch: Patch { provider: Some(provider.id().into()), provider_order_id: r.provider_order_id.clone(), file_sha: Some(file_sha), file_type: Some(file_type), ..Default::default() },
            quiet: true,
            ..Default::default()
        },
    )
    .await
    .map_err(err)?;
    if fin == "in_production" {
        set_status(s, &o.id(), "in_production", StatusChange { source: "provider", ..Default::default() }).await.map_err(err)?;
    }
    progress(s, ctx, 1.0, "Submitted").await?;
    Ok(json!({ "provider": provider.id(), "providerOrderId": r.provider_order_id }))
}
