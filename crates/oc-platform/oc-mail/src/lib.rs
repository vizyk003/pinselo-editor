//! Transactional email (CO-070): render template → `email_outbox` row → `email` job → SMTP (`OC_SMTP_URL`).
//! Without SMTP configured, messages stay in the outbox and are viewable in the dev mailbox (`/dev/mail`).
//!
//! Templates, catalogs (embedded from `locales/`), HTML/text layout and the outbox/job
//! protocol are identical to `packages/mail`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use oc_config::BRAND;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

mod catalogs;

/// Print order statuses customers are emailed about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintMailStatus {
    InProduction,
    Shipped,
    Delivered,
    Failed,
    Cancelled,
    Returned,
    Submitted,
}

/// Email templates; JSON shape is the TS discriminated union (`{"kind": "magic", "url": …}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Template {
    Magic { url: String },
    Verify { url: String, name: String },
    Reset { url: String },
    Share { actor: String, title: String, url: String, role: String },
    Mention { actor: String, title: String, body: String, url: String },
    Comment { actor: String, title: String, body: String, url: String },
    Reply { actor: String, title: String, body: String, url: String },
    Invite { actor: String, team: String, url: String },
    Decision { subject: String, body: String },
    ExportReady { url: String },
    AccountDeleted,
    NewLogin { device: String, ip: String, when: String },
    FormResponse { form: String, title: String, summary: String, url: String },
    Assign { actor: String, title: String, body: String, #[serde(default, skip_serializing_if = "Option::is_none")] due: Option<String>, url: String },
    PostPublished { title: String, links: String, url: String },
    PostFailed { title: String, error: String, url: String },
    PostReminder { title: String, caption: String, files: String, url: String },
    ApprovalRequest { actor: String, title: String, message: String, url: String },
    ApprovalDecision { actor: String, title: String, approved: bool, note: String, url: String },
    PrintConfirmed { number: String, product: String, total: String, url: String },
    PrintStatus { number: String, status: PrintMailStatus, tracking: String, note: String, url: String },
    PrintOperator {
        number: String,
        product: String,
        quantity: String,
        shipping: String,
        address: String,
        #[serde(rename = "fileUrl")]
        file_url: String,
        url: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Security,
    Mentions,
    Comments,
    Shares,
    Service,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Security => "security",
            Category::Mentions => "mentions",
            Category::Comments => "comments",
            Category::Shares => "shares",
            Category::Service => "service",
        }
    }
}

impl Template {
    pub fn category(&self) -> Category {
        use Category::*;
        match self {
            Template::Magic { .. } | Template::Verify { .. } | Template::Reset { .. } | Template::NewLogin { .. } | Template::AccountDeleted => Security,
            Template::Share { .. } | Template::Invite { .. } => Shares,
            Template::Mention { .. } | Template::Assign { .. } => Mentions,
            Template::Comment { .. } | Template::Reply { .. } | Template::FormResponse { .. } | Template::ApprovalRequest { .. } | Template::ApprovalDecision { .. } => Comments,
            Template::Decision { .. }
            | Template::ExportReady { .. }
            | Template::PostPublished { .. }
            | Template::PostFailed { .. }
            | Template::PostReminder { .. }
            | Template::PrintConfirmed { .. }
            | Template::PrintStatus { .. }
            | Template::PrintOperator { .. } => Service,
        }
    }
}

/// HTML-escape `& < > "`.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ------------------------------------------------------------------ catalogs

type Catalog = HashMap<String, String>;

fn embedded(code: &str) -> Option<&'static str> {
    catalogs::CATALOG_FILES.iter().find(|(c, _)| *c == code).map(|(_, s)| *s)
}

fn catalog(locale: &str) -> std::sync::Arc<Catalog> {
    static CACHE: OnceLock<Mutex<HashMap<String, std::sync::Arc<Catalog>>>> = OnceLock::new();
    let code: String = locale.chars().filter(|c| c.is_ascii_alphabetic() || *c == '-').collect();
    if let Some(c) = CACHE.get_or_init(Default::default).lock().unwrap().get(&code) {
        return c.clone();
    }
    let c = match embedded(&code).and_then(|s| serde_json::from_str::<Catalog>(s).ok()) {
        Some(c) => std::sync::Arc::new(c),
        None => {
            let lang = code.split('-').next().unwrap_or("").to_owned();
            if lang != code { catalog(&lang) } else { Default::default() }
        }
    };
    CACHE.get_or_init(Default::default).lock().unwrap().insert(code, c.clone());
    c
}

/// A translator for a recipient locale: gettext-style English keys, `{var}` placeholders.
#[derive(Clone)]
pub struct Translator {
    cat: std::sync::Arc<Catalog>,
}

impl Translator {
    pub fn new(locale: &str) -> Self {
        Translator { cat: if locale == "en" { Default::default() } else { catalog(locale) } }
    }

    /// `tr(s, vars)`: catalog lookup (empty translations fall back to the key), then `{\w+}` substitution;
    /// unknown placeholders are left as-is.
    pub fn tr(&self, s: &str, vars: &[(&str, &str)]) -> String {
        let src = self.cat.get(s).filter(|t| !t.is_empty()).map(String::as_str).unwrap_or(s);
        let mut out = String::with_capacity(src.len());
        let mut rest = src;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let after = &rest[i + 1..];
            let n = after.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
            if n > 0 && after.as_bytes().get(n) == Some(&b'}') {
                let k = &after[..n];
                match vars.iter().find(|(kk, _)| *kk == k) {
                    Some((_, v)) => out.push_str(v),
                    None => out.push_str(&rest[i..i + n + 2]),
                }
                rest = &after[n + 1..];
            } else {
                out.push('{');
                rest = after;
            }
        }
        out.push_str(rest);
        out
    }

    fn t(&self, s: &str) -> String {
        self.tr(s, &[])
    }
}

pub fn translator(locale: &str) -> Translator {
    Translator::new(locale)
}

// ------------------------------------------------------------------ rendering

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cta {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub subject: String,
    pub heading: String,
    pub lines: Vec<String>,
    pub cta: Option<Cta>,
}

fn cta(label: String, url: &str) -> Option<Cta> {
    Some(Cta { label, url: url.to_owned() })
}

fn quoted(s: &str) -> String {
    format!("\u{201c}{s}\u{201d}")
}

fn non_empty_lines(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split('\n').filter(|l| !l.is_empty()).map(str::to_owned)
}

/// The structured content of a template (subject, heading, paragraphs, call to action).
pub fn render(t: &Template, tr: &Translator) -> Rendered {
    let brand = BRAND.name;
    let r = |subject: String, heading: String, lines: Vec<String>, cta: Option<Cta>| Rendered { subject, heading, lines, cta };
    match t {
        Template::Magic { url } => r(
            tr.tr("Your {brand} sign-in link", &[("brand", brand)]),
            tr.t("Sign in"),
            vec![tr.t("Click the button to sign in. The link works once and expires in 15 minutes."), tr.t("If you didn\u{2019}t ask for this, you can ignore this email.")],
            cta(tr.t("Sign in"), url),
        ),
        Template::Verify { url, name } => r(
            tr.tr("Confirm your email for {brand}", &[("brand", brand)]),
            tr.tr("Welcome, {name}", &[("name", name)]),
            vec![tr.t("Confirm your email address to finish setting up your account.")],
            cta(tr.t("Confirm email"), url),
        ),
        Template::Reset { url } => r(
            tr.tr("Reset your {brand} password", &[("brand", brand)]),
            tr.t("Reset your password"),
            vec![tr.t("Use this link within 30 minutes to choose a new password. If you didn\u{2019}t ask for it, ignore this email \u{2014} your password stays the same.")],
            cta(tr.t("Choose a new password"), url),
        ),
        Template::Share { actor, title, url, role } => {
            let can = match role.as_str() {
                "edit" => tr.tr("You can edit \u{201c}{title}\u{201d}.", &[("title", title)]),
                "comment" => tr.tr("You can comment on \u{201c}{title}\u{201d}.", &[("title", title)]),
                _ => tr.tr("You can view \u{201c}{title}\u{201d}.", &[("title", title)]),
            };
            r(
                tr.tr("{actor} shared \u{201c}{title}\u{201d} with you", &[("actor", actor), ("title", title)]),
                tr.tr("{actor} shared a design", &[("actor", actor)]),
                vec![can],
                cta(tr.t("Open design"), url),
            )
        }
        Template::Mention { actor, title, body, url } => r(
            tr.tr("{actor} mentioned you in \u{201c}{title}\u{201d}", &[("actor", actor), ("title", title)]),
            tr.tr("{actor} mentioned you", &[("actor", actor)]),
            vec![quoted(body)],
            cta(tr.t("Reply"), url),
        ),
        Template::Comment { actor, title, body, url } => r(
            tr.tr("New comment on \u{201c}{title}\u{201d}", &[("title", title)]),
            tr.tr("{actor} commented", &[("actor", actor)]),
            vec![quoted(body)],
            cta(tr.t("View comment"), url),
        ),
        Template::Reply { actor, title, body, url } => r(
            tr.tr("{actor} replied in \u{201c}{title}\u{201d}", &[("actor", actor), ("title", title)]),
            tr.tr("{actor} replied", &[("actor", actor)]),
            vec![quoted(body)],
            cta(tr.t("View thread"), url),
        ),
        Template::Invite { actor, team, url } => r(
            tr.tr("{actor} invited you to {team} on {brand}", &[("actor", actor), ("team", team), ("brand", brand)]),
            tr.tr("Join {team}", &[("team", team)]),
            vec![tr.tr("{actor} invited you to design together in {team}. The invite expires in 7 days.", &[("actor", actor), ("team", team)])],
            cta(tr.t("Accept invite"), url),
        ),
        Template::Decision { subject, body } => r(subject.clone(), subject.clone(), body.split('\n').map(str::to_owned).collect(), None),
        Template::ExportReady { url } => r(
            tr.tr("Your {brand} data export is ready", &[("brand", brand)]),
            tr.t("Your export is ready"),
            vec![tr.t("The download link works for 7 days.")],
            cta(tr.t("Download"), url),
        ),
        Template::AccountDeleted => r(
            tr.tr("Your {brand} account was deleted", &[("brand", brand)]),
            tr.t("Account deleted"),
            vec![tr.t("Your account and designs have been deleted. Backups containing your data are purged within 35 days.")],
            None,
        ),
        Template::Assign { actor, title, body, due, url } => {
            let mut lines = vec![quoted(body)];
            if let Some(d) = due.as_deref().filter(|d| !d.is_empty()) {
                lines.push(tr.tr("Due {date}", &[("date", d)]));
            }
            r(
                tr.tr("{actor} assigned you a task in \u{201c}{title}\u{201d}", &[("actor", actor), ("title", title)]),
                tr.tr("{actor} assigned you a task", &[("actor", actor)]),
                lines,
                cta(tr.t("Open task"), url),
            )
        }
        Template::FormResponse { form, title, summary, url } => {
            let mut lines = vec![tr.tr("On \u{201c}{title}\u{201d}:", &[("title", title)])];
            lines.extend(summary.split('\n').map(str::to_owned));
            r(
                tr.tr("New response: {form}", &[("form", form)]),
                tr.tr("New response to \u{201c}{form}\u{201d}", &[("form", form)]),
                lines,
                cta(tr.t("See all responses"), url),
            )
        }
        Template::PostPublished { title, links, url } => {
            let mut lines = vec![tr.tr("\u{201c}{title}\u{201d} was published as scheduled.", &[("title", title)])];
            lines.extend(non_empty_lines(links));
            r(tr.tr("Posted: \u{201c}{title}\u{201d}", &[("title", title)]), tr.t("Your post is live"), lines, cta(tr.t("Open Content Planner"), url))
        }
        Template::PostFailed { title, error, url } => r(
            tr.tr("Couldn\u{2019}t publish \u{201c}{title}\u{201d}", &[("title", title)]),
            tr.t("Your scheduled post failed"),
            vec![tr.tr("We couldn\u{2019}t publish \u{201c}{title}\u{201d}.", &[("title", title)]), error.clone(), tr.t("Open the Content Planner to fix the problem and try again.")],
            cta(tr.t("Open Content Planner"), url),
        ),
        Template::PostReminder { title, caption, files, url } => {
            let mut lines = vec![tr.tr("Your design \u{201c}{title}\u{201d} is ready to share. Download the files and paste the caption below.", &[("title", title)])];
            lines.extend(non_empty_lines(files));
            lines.push(tr.t("Caption:"));
            lines.push(if caption.is_empty() { tr.t("(no caption)") } else { caption.clone() });
            r(tr.tr("Time to post \u{201c}{title}\u{201d}", &[("title", title)]), tr.t("It\u{2019}s time to post"), lines, cta(tr.t("Open Content Planner"), url))
        }
        Template::ApprovalRequest { actor, title, message, url } => r(
            tr.tr("{actor} asked you to approve \u{201c}{title}\u{201d}", &[("actor", actor), ("title", title)]),
            tr.t("Approval requested"),
            if message.is_empty() { vec![] } else { vec![quoted(message)] },
            cta(tr.t("Review design"), url),
        ),
        Template::PrintConfirmed { number, product, total, url } => r(
            tr.tr("Order {number} confirmed", &[("number", number)]),
            tr.t("Thanks for your order"),
            vec![tr.tr("We received your payment for {product}.", &[("product", product)]), tr.tr("Total: {total}", &[("total", total)]), tr.t("We\u{2019}ll email you when it ships.")],
            cta(tr.t("View order"), url),
        ),
        Template::PrintStatus { number, status, tracking, note, url } => {
            let label = match status {
                PrintMailStatus::Submitted => tr.t("Your order is being prepared"),
                PrintMailStatus::InProduction => tr.t("Your order is being printed"),
                PrintMailStatus::Shipped => tr.t("Your order is on its way"),
                PrintMailStatus::Delivered => tr.t("Your order was delivered"),
                PrintMailStatus::Failed => tr.t("There\u{2019}s a problem with your order"),
                PrintMailStatus::Cancelled => tr.t("Your order was cancelled"),
                PrintMailStatus::Returned => tr.t("Your order was returned"),
            };
            let mut lines = vec![];
            if !tracking.is_empty() {
                lines.push(tr.tr("Tracking: {tracking}", &[("tracking", tracking)]));
            }
            if !note.is_empty() {
                lines.push(note.clone());
            }
            r(tr.tr("Order {number}: {status}", &[("number", number), ("status", &label)]), label, lines, cta(tr.t("View order"), url))
        }
        Template::PrintOperator { number, product, quantity, shipping, address, file_url, url } => {
            let mut lines = vec![product.clone(), tr.tr("Quantity: {n}", &[("n", quantity)]), tr.tr("Shipping: {method}", &[("method", shipping)]), tr.t("Ship to:")];
            lines.extend(address.split('\n').map(str::to_owned));
            lines.push(tr.tr("Print file (PDF with bleed and crop marks): {url}", &[("url", file_url)]));
            lines.push(tr.t("Update the status in the admin console once it ships."));
            let h = tr.tr("New print order {number}", &[("number", number)]);
            r(h.clone(), h, lines, cta(tr.t("Open print orders"), url))
        }
        Template::ApprovalDecision { actor, title, approved, note, url } => {
            let head = if *approved {
                tr.tr("\u{201c}{title}\u{201d} was approved", &[("title", title)])
            } else {
                tr.tr("Changes requested on \u{201c}{title}\u{201d}", &[("title", title)])
            };
            let mut lines = vec![tr.tr("By {actor}", &[("actor", actor)])];
            if !note.is_empty() {
                lines.push(quoted(note));
            }
            r(head.clone(), head, lines, cta(tr.t("Open design"), url))
        }
        Template::NewLogin { device, ip, when } => r(
            tr.tr("New sign-in to {brand}", &[("brand", brand)]),
            tr.t("New sign-in"),
            vec![
                tr.tr("A new sign-in from {device} ({ip}) at {when}.", &[("device", device), ("ip", ip), ("when", when)]),
                tr.t("If this wasn\u{2019}t you, reset your password and sign out other sessions in Settings \u{2192} Security."),
            ],
            None,
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedEmail {
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// `renderEmail(t, locale)`: subject, HTML and plain-text bodies, byte-identical to the TS.
pub fn render_email(t: &Template, locale: &str) -> RenderedEmail {
    let tr = translator(locale);
    let r = render(t, &tr);
    let mut html = String::new();
    html.push_str(&format!(
        "<!doctype html><html lang=\"{}\"><body style=\"margin:0;background:#f4f4f7;font-family:Inter,Segoe UI,Helvetica,Arial,sans-serif\">\n",
        esc(locale)
    ));
    html.push_str("<table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\"><tr><td align=\"center\" style=\"padding:32px 12px\">\n");
    html.push_str("<table width=\"560\" cellpadding=\"0\" cellspacing=\"0\" style=\"max-width:560px;background:#fff;border-radius:14px;overflow:hidden\">\n");
    html.push_str(&format!(
        "<tr><td style=\"background:linear-gradient(90deg,#080e2a,#c93316);padding:18px 28px;color:#fff;font-weight:800;font-size:18px\">{}</td></tr>\n",
        esc(BRAND.name)
    ));
    html.push_str("<tr><td style=\"padding:28px\">\n");
    html.push_str(&format!("<h1 style=\"margin:0 0 14px;font-size:22px;color:#17171c\">{}</h1>\n", esc(&r.heading)));
    for l in &r.lines {
        html.push_str(&format!("<p style=\"margin:0 0 12px;font-size:15px;line-height:1.55;color:#3d3d4c\">{}</p>", esc(l)));
    }
    html.push('\n');
    if let Some(c) = &r.cta {
        html.push_str(&format!(
            "<p style=\"margin:22px 0 6px\"><a href=\"{}\" style=\"background:#c93316;color:#fff;text-decoration:none;padding:12px 20px;border-radius:9px;font-weight:700;display:inline-block\">{}</a></p><p style=\"font-size:12px;color:#8a8aa0;word-break:break-all\">{}</p>",
            esc(&c.url),
            esc(&c.label),
            esc(&c.url)
        ));
    }
    html.push_str("\n</td></tr>\n");
    html.push_str(&format!(
        "<tr><td style=\"padding:16px 28px;font-size:12px;color:#8a8aa0;border-top:1px solid #eee\">{}</td></tr>\n",
        esc(&tr.tr("You\u{2019}re receiving this because of activity on your {brand} account. Manage email in Settings \u{2192} Notifications.", &[("brand", BRAND.name)]))
    ));
    html.push_str("</table></td></tr></table></body></html>");
    let mut text = format!("{}\n\n{}", r.heading, r.lines.join("\n\n"));
    if let Some(c) = &r.cta {
        text.push_str(&format!("\n\n{}: {}", c.label, c.url));
    }
    text.push('\n');
    RenderedEmail { subject: r.subject, html, text }
}

// ------------------------------------------------------------------ outbox + delivery

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("smtp: {0}")]
    Smtp(String),
}

/// Queue an email (outbox row + `email` job with 5 attempts). Respects per-user preferences for
/// non-security categories; returns `None` when the recipient opted out.
pub async fn send_email(pool: &PgPool, to: &str, t: &Template, user_id: Option<Uuid>) -> Result<Option<Uuid>, MailError> {
    let category = t.category();
    if let Some(uid) = user_id
        && category != Category::Security
        && category != Category::Service
    {
        let pref: Option<(bool, bool, bool)> = sqlx::query_as("SELECT mentions, comments, shares FROM email_preferences WHERE user_id = $1").bind(uid).fetch_optional(pool).await?;
        if let Some((mentions, comments, shares)) = pref {
            let on = match category {
                Category::Mentions => mentions,
                Category::Comments => comments,
                Category::Shares => shares,
                _ => true,
            };
            if !on {
                return Ok(None);
            }
        }
    }
    // Each recipient reads it in their account language (signed-out recipients get English).
    let locale: Option<Option<String>> =
        sqlx::query_scalar("SELECT locale FROM users WHERE lower(email) = lower($1) AND deleted_at IS NULL").bind(to).fetch_optional(pool).await.unwrap_or(None);
    let locale = locale.flatten().unwrap_or_else(|| "en".into());
    let RenderedEmail { subject, html, text } = render_email(t, &locale);
    let id = oc_db::uuid();
    sqlx::query("INSERT INTO email_outbox(id, to_email, subject, html, text, category) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(id)
        .bind(to.to_lowercase())
        .bind(subject)
        .bind(html)
        .bind(text)
        .bind(category.as_str())
        .execute(pool)
        .await?;
    oc_jobs::enqueue(pool, "email", &serde_json::json!({ "id": id }), oc_jobs::EnqueueOpts { max_attempts: Some(5), ..Default::default() }).await?;
    Ok(Some(id))
}

/// Default sender: `OC_MAIL_FROM`, else `"<Brand> <no-reply@<web origin host>>"`.
pub fn mail_from() -> String {
    std::env::var("OC_MAIL_FROM").unwrap_or_else(|_| format!("{} <no-reply@{}>", BRAND.name, oc_config::hostname(&oc_config::config().web_origin).unwrap_or_default()))
}

/// Build an SMTP transport from a nodemailer-style URL: `smtp://user:pass@host:port` (port 587 default,
/// STARTTLS when offered) or `smtps://…` (port 465 default, implicit TLS).
pub fn smtp_transport(smtp_url: &str) -> Result<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>, MailError> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::transport::smtp::client::{Tls, TlsParameters};
    let u = url::Url::parse(smtp_url).map_err(|e| MailError::Smtp(e.to_string()))?;
    let host = u.host_str().ok_or_else(|| MailError::Smtp("SMTP URL has no host".into()))?.to_owned();
    let secure = u.scheme() == "smtps";
    let port = u.port().unwrap_or(if secure { 465 } else { 587 });
    let params = TlsParameters::new(host.clone()).map_err(|e| MailError::Smtp(e.to_string()))?;
    let mut b = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(&host)
        .port(port)
        .tls(if secure { Tls::Wrapper(params) } else { Tls::Opportunistic(params) });
    if !u.username().is_empty() {
        let dec = |s: &str| url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default();
        b = b.credentials(Credentials::new(dec(u.username()), dec(u.password().unwrap_or(""))));
    }
    Ok(b.build())
}

static TRANSPORT: OnceLock<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>> = OnceLock::new();

/// Email job handler (runs in the worker). Without `OC_SMTP_URL` the message stays in the outbox
/// (dev mailbox) and is marked sent.
pub async fn deliver(pool: &PgPool, id: Uuid) -> Result<(), MailError> {
    use lettre::AsyncTransport;
    let m: Option<(String, String, String, String, String)> =
        sqlx::query_as("SELECT to_email, subject, html, text, status FROM email_outbox WHERE id = $1").bind(id).fetch_optional(pool).await?;
    let Some((to, subject, html, text, status)) = m else { return Ok(()) };
    if status != "queued" {
        return Ok(());
    }
    let Ok(smtp_url) = std::env::var("OC_SMTP_URL") else {
        // Dev: keep it in the outbox (visible at /dev/mail), mark as delivered locally.
        sqlx::query("UPDATE email_outbox SET status = 'sent', sent_at = now() WHERE id = $1").bind(id).execute(pool).await?;
        return Ok(());
    };
    let send = async {
        let transport = match TRANSPORT.get() {
            Some(t) => t,
            None => {
                let t = smtp_transport(&smtp_url)?;
                TRANSPORT.get_or_init(|| t)
            }
        };
        let msg = lettre::Message::builder()
            .from(mail_from().parse().map_err(|e: lettre::address::AddressError| MailError::Smtp(e.to_string()))?)
            .to(to.parse().map_err(|e: lettre::address::AddressError| MailError::Smtp(e.to_string()))?)
            .subject(subject)
            .multipart(lettre::message::MultiPart::alternative_plain_html(text, html))
            .map_err(|e| MailError::Smtp(e.to_string()))?;
        transport.send(msg).await.map_err(|e| MailError::Smtp(e.to_string()))?;
        Ok::<(), MailError>(())
    };
    match send.await {
        Ok(()) => {
            sqlx::query("UPDATE email_outbox SET status = 'sent', sent_at = now() WHERE id = $1").bind(id).execute(pool).await?;
            Ok(())
        }
        Err(e) => {
            sqlx::query("UPDATE email_outbox SET error = $2 WHERE id = $1").bind(id).bind(e.to_string()).execute(pool).await?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translator_placeholders() {
        let tr = translator("en");
        assert_eq!(tr.tr("{a} and {b} {c", &[("a", "1")]), "1 and {b} {c");
        assert_eq!(tr.tr("{}{x_1}", &[("x_1", "$&")]), "{}$&");
    }

    #[test]
    fn locale_fallback() {
        assert_eq!(translator("de-AT").t("Sign in"), translator("de").t("Sign in"));
        assert_ne!(translator("de").t("Sign in"), "Sign in");
        assert_eq!(translator("xx").t("Sign in"), "Sign in");
    }

    #[test]
    fn smtp_url_parsing() {
        assert!(smtp_transport("smtp://u%40x:p@mail.example.com").is_ok());
        assert!(smtp_transport("smtps://mail.example.com:2465").is_ok());
    }
}
