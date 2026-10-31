//! Upload links: how a file an assistant can't carry through a tool call gets into the user's uploads.
//! `create_upload_link` makes one; `/api/upload-links/{token}` then takes the bytes (PUT or POST, raw or multipart:
//! `curl -T file url` from a client with network access) or, opened in a browser, shows a small page with a file field.
//! The token is the only credential, is scoped to this one upload, and works once within 15 minutes. Sessions are not
//! used, except that someone signed in as another user is turned away so a file can't land in the wrong account by
//! mistake. Files go through the same ingest as every other upload.

use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use futures::StreamExt as _;
use tokio::io::AsyncWriteExt as _;

use crate::auth::current_user;
use crate::bearer::sha256;
use crate::http::multipart;
use crate::http::ratelimit::RouteLimit;
use crate::prelude::*;
use crate::routes::assets::{Ingest, IngestOpts, TempDir, ingest_file};

const PATH: &str = "/api/upload-links/{token}";
const LIFETIME_MINUTES: i64 = 15;
const MAX_LINKS_PER_HOUR: i64 = 20;
const MAX_FILES_PER_LINK: usize = 20;

pub const LIMITS: &[RouteLimit] = &[
    RouteLimit { method: "GET", path: PATH, max: 30, window_ms: crate::http::ratelimit::MINUTE },
    RouteLimit { method: "PUT", path: PATH, max: 10, window_ms: crate::http::ratelimit::MINUTE },
    RouteLimit { method: "POST", path: PATH, max: 10, window_ms: crate::http::ratelimit::MINUTE },
];

pub fn routes() -> Router<AppState> {
    Router::new().route(PATH, get(page).put(receive).post(receive))
}

pub struct NewLink {
    pub id: Uuid,
    pub url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// A new link for this user; `None` when they have made too many lately.
pub async fn create(s: &AppState, user_id: Uuid, filename: Option<&str>, purpose: Option<&str>) -> Result<Option<NewLink>, sqlx::Error> {
    let now = s.now();
    sqlx::query("DELETE FROM upload_links WHERE user_id = $1 AND expires_at < $2").bind(user_id).bind(now - chrono::Duration::days(1)).execute(&s.pool).await?;
    let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM upload_links WHERE user_id = $1 AND created_at > $2").bind(user_id).bind(now - chrono::Duration::hours(1)).fetch_one(&s.pool).await?;
    if recent >= MAX_LINKS_PER_HOUR {
        return Ok(None);
    }
    let id = oc_db::uuid();
    // Two v4 UUIDs: 244 random bits from the system's generator.
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let expires_at = now + chrono::Duration::minutes(LIFETIME_MINUTES);
    sqlx::query("INSERT INTO upload_links(id, user_id, token_sha256, filename, purpose, created_at, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(id)
        .bind(user_id)
        .bind(sha256(&token))
        .bind(filename)
        .bind(purpose)
        .bind(now)
        .bind(expires_at)
        .execute(&s.pool)
        .await?;
    Ok(Some(NewLink { id, url: format!("{}/api/upload-links/{token}", s.cfg.web_origin), expires_at }))
}

/// `pending`, `done` (with the asset ids) or `expired`, for the user who made the link; `None` for anyone else.
pub async fn status(s: &AppState, user_id: Uuid, id: Uuid) -> Result<Option<(&'static str, Vec<String>)>, sqlx::Error> {
    type Row = (chrono::DateTime<chrono::Utc>, Vec<String>);
    let row: Option<Row> = sqlx::query_as("SELECT expires_at, asset_ids FROM upload_links WHERE id = $1 AND user_id = $2").bind(id).bind(user_id).fetch_optional(&s.pool).await?;
    Ok(row.map(|(expires, assets)| {
        if !assets.is_empty() {
            ("done", assets)
        } else if expires < s.now() {
            ("expired", assets)
        } else {
            ("pending", assets)
        }
    }))
}

enum Look {
    Unknown,
    Gone,
    WrongUser,
    Open { user_id: Uuid, filename: Option<String>, purpose: Option<String> },
}

async fn look_up(s: &AppState, req: &Req, token: &str) -> Result<Look, ApiError> {
    type Row = (Uuid, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>);
    let row: Option<Row> = sqlx::query_as("SELECT user_id, filename, purpose, expires_at, used_at FROM upload_links WHERE token_sha256 = $1").bind(sha256(token)).fetch_optional(&s.pool).await?;
    let Some((user_id, filename, purpose, expires_at, used_at)) = row else { return Ok(Look::Unknown) };
    if used_at.is_some() || expires_at < s.now() {
        return Ok(Look::Gone);
    }
    if current_user(s, req).await?.is_some_and(|u| u.id != user_id) {
        return Ok(Look::WrongUser);
    }
    Ok(Look::Open { user_id, filename, purpose })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The page's few strings. There is no server-side catalogue for web pages (the mail one serves the mailer), and this
/// page has no account language to go by, so it follows the browser's. JSON answers stay in English.
struct Words {
    add: &'static str,
    intro: &'static str,
    for_: &'static str,
    upload: &'static str,
    uploaded: &'static str,
    uploaded_body: &'static str,
    refused: &'static str,
    unknown: &'static str,
    gone: &'static str,
    wrong_user: &'static str,
    no_file: &'static str,
    file_label: &'static str,
    received: &'static str,
    refused_file: &'static str,
}

const EN: Words = Words {
    add: "Add a file",
    intro: "Choose files or drop them on the box. They go to your uploads, and this link then stops working.",
    for_: "For:",
    upload: "Upload",
    uploaded: "Uploaded",
    uploaded_body: "The file is in your uploads. You can close this page and go back to your assistant.",
    refused: "This link can’t be used",
    unknown: "This upload link doesn’t exist.",
    gone: "This upload link has expired or was already used. Ask your assistant for a new one.",
    wrong_user: "This upload link belongs to another account. Sign out, or use a private window.",
    no_file: "Choose a file first.",
    file_label: "Files to upload",
    received: "received",
    refused_file: "not accepted",
};

fn words(req: &Req) -> &'static Words {
    const DE: Words = Words { add: "Datei hinzufügen", intro: "Wähle eine Datei oder ziehe sie in das Feld. Sie landet in deinen Uploads, danach funktioniert dieser Link nicht mehr.", for_: "Für:", upload: "Hochladen", uploaded: "Hochgeladen", uploaded_body: "Die Datei ist in deinen Uploads. Du kannst diese Seite schließen und zu deinem Assistenten zurückkehren.", refused: "Dieser Link kann nicht verwendet werden", unknown: "Diesen Upload-Link gibt es nicht.", gone: "Dieser Upload-Link ist abgelaufen oder wurde bereits verwendet. Bitte deinen Assistenten um einen neuen.", wrong_user: "Dieser Upload-Link gehört zu einem anderen Konto. Melde dich ab oder nutze ein privates Fenster.", no_file: "Wähle zuerst eine Datei.", file_label: "Hochzuladende Dateien", received: "erhalten", refused_file: "nicht angenommen" };
    const ES: Words = Words { add: "Añadir un archivo", intro: "Elige un archivo o suéltalo en el recuadro. Irá a tus subidas y este enlace dejará de funcionar.", for_: "Para:", upload: "Subir", uploaded: "Subido", uploaded_body: "El archivo está en tus subidas. Puedes cerrar esta página y volver a tu asistente.", refused: "Este enlace no se puede usar", unknown: "Este enlace de subida no existe.", gone: "Este enlace de subida ha caducado o ya se usó. Pide uno nuevo a tu asistente.", wrong_user: "Este enlace de subida pertenece a otra cuenta. Cierra sesión o usa una ventana privada.", no_file: "Elige primero un archivo.", file_label: "Archivos para subir", received: "recibido", refused_file: "no aceptado" };
    const FR: Words = Words { add: "Ajouter un fichier", intro: "Choisissez un fichier ou déposez-le dans le cadre. Il rejoint vos imports, puis ce lien cesse de fonctionner.", for_: "Pour :", upload: "Envoyer", uploaded: "Envoyé", uploaded_body: "Le fichier est dans vos imports. Vous pouvez fermer cette page et revenir à votre assistant.", refused: "Ce lien n’est pas utilisable", unknown: "Ce lien d’envoi n’existe pas.", gone: "Ce lien d’envoi a expiré ou a déjà servi. Demandez-en un nouveau à votre assistant.", wrong_user: "Ce lien d’envoi appartient à un autre compte. Déconnectez-vous ou utilisez une fenêtre privée.", no_file: "Choisissez d’abord un fichier.", file_label: "Fichiers à envoyer", received: "reçu", refused_file: "refusé" };
    const IT: Words = Words { add: "Aggiungi un file", intro: "Scegli un file o trascinalo nel riquadro. Finirà nei tuoi caricamenti, poi questo link smetterà di funzionare.", for_: "Per:", upload: "Carica", uploaded: "Caricato", uploaded_body: "Il file è nei tuoi caricamenti. Puoi chiudere questa pagina e tornare al tuo assistente.", refused: "Questo link non può essere usato", unknown: "Questo link di caricamento non esiste.", gone: "Questo link di caricamento è scaduto o è già stato usato. Chiedine uno nuovo al tuo assistente.", wrong_user: "Questo link di caricamento appartiene a un altro account. Esci o usa una finestra privata.", no_file: "Scegli prima un file.", file_label: "File da caricare", received: "ricevuto", refused_file: "non accettato" };
    const JA: Words = Words { add: "ファイルを追加", intro: "ファイルを選ぶか、枠にドロップしてください。アップロード済みファイルに追加され、このリンクは使えなくなります。", for_: "用途:", upload: "アップロード", uploaded: "アップロードしました", uploaded_body: "ファイルはアップロード済みファイルにあります。このページを閉じてアシスタントに戻ってください。", refused: "このリンクは使用できません", unknown: "このアップロードリンクは存在しません。", gone: "このアップロードリンクは期限切れか、すでに使用されています。アシスタントに新しいリンクを依頼してください。", wrong_user: "このアップロードリンクは別のアカウントのものです。ログアウトするか、プライベートウィンドウを使ってください。", no_file: "先にファイルを選んでください。", file_label: "アップロードするファイル", received: "受信しました", refused_file: "受け付けられません" };
    const PT: Words = Words { add: "Adicionar um arquivo", intro: "Escolha um arquivo ou solte-o na caixa. Ele vai para seus uploads e este link deixa de funcionar.", for_: "Para:", upload: "Enviar", uploaded: "Enviado", uploaded_body: "O arquivo está nos seus uploads. Você pode fechar esta página e voltar ao seu assistente.", refused: "Este link não pode ser usado", unknown: "Este link de upload não existe.", gone: "Este link de upload expirou ou já foi usado. Peça um novo ao seu assistente.", wrong_user: "Este link de upload pertence a outra conta. Saia da conta ou use uma janela privada.", no_file: "Escolha um arquivo primeiro.", file_label: "Arquivos para enviar", received: "recebido", refused_file: "não aceito" };
    const ZH: Words = Words { add: "添加文件", intro: "选择文件或将其拖到框内。文件会进入你的上传内容，之后此链接将失效。", for_: "用途：", upload: "上传", uploaded: "已上传", uploaded_body: "文件已在你的上传内容中。你可以关闭此页面并返回助手。", refused: "无法使用此链接", unknown: "此上传链接不存在。", gone: "此上传链接已过期或已被使用。请向助手索取新的链接。", wrong_user: "此上传链接属于另一个账户。请退出登录或使用隐私窗口。", no_file: "请先选择文件。", file_label: "要上传的文件", received: "已接收", refused_file: "未接受" };
    const AR: Words = Words { add: "إضافة ملف", intro: "اختر ملفًا أو أفلته في المربع. سيُضاف إلى ملفاتك المرفوعة، ثم يتوقف هذا الرابط عن العمل.", for_: "الغرض:", upload: "رفع", uploaded: "تم الرفع", uploaded_body: "الملف الآن ضمن ملفاتك المرفوعة. يمكنك إغلاق هذه الصفحة والعودة إلى مساعدك.", refused: "لا يمكن استخدام هذا الرابط", unknown: "رابط الرفع هذا غير موجود.", gone: "انتهت صلاحية رابط الرفع هذا أو سبق استخدامه. اطلب رابطًا جديدًا من مساعدك.", wrong_user: "رابط الرفع هذا يخص حسابًا آخر. سجّل الخروج أو استخدم نافذة خاصة.", no_file: "اختر ملفًا أولًا.", file_label: "الملفات المراد رفعها", received: "تم الاستلام", refused_file: "غير مقبول" };
    let accept = req.header("accept-language").unwrap_or("").to_ascii_lowercase();
    for tag in accept.split(',').map(|t| t.split(';').next().unwrap_or("").trim()) {
        match tag.split('-').next().unwrap_or("") {
            "de" => return &DE,
            "es" => return &ES,
            "fr" => return &FR,
            "it" => return &IT,
            "ja" => return &JA,
            "pt" => return &PT,
            "zh" => return &ZH,
            "ar" => return &AR,
            "en" => return &EN,
            _ => {}
        }
    }
    &EN
}

/// No script, no external resources; a plain form works with a click or a dropped file.
fn html(status: u16, brand: &str, title: &str, body: &str) -> Reply {
    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{title} · {brand}</title>\
<style>body{{font:16px system-ui,sans-serif;margin:0;padding:48px 16px;background:#f6f6f4;color:#1f2328}}main{{max-width:480px;margin:auto;background:#fff;border-radius:12px;padding:24px}}h1{{font-size:20px;margin:0 0 12px}}label{{display:block;font-weight:600;margin-top:12px}}input[type=file]{{display:block;width:100%;box-sizing:border-box;padding:40px 12px;border:2px dashed #b5b7ba;border-radius:10px;margin:8px 0 16px}}button{{font:inherit;padding:8px 16px}}</style>\
</head><body><main><h1>{title}</h1>{body}</main></body></html>"
    );
    Reply::bytes("text/html; charset=utf-8", page.into_bytes())
        .status(status)
        .header("content-security-policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'")
        .header("cache-control", "no-store")
}

fn wants_html(req: &Req) -> bool {
    req.header("accept").is_some_and(|a| a.contains("text/html"))
}

/// `shown` is what a person reads in a browser; `message` goes to clients.
fn refusal(s: &AppState, req: &Req, status: u16, error: &str, message: &str, shown: &str) -> Reply {
    if wants_html(req) {
        return html(status, s.cfg.brand.name, words(req).refused, &format!("<p>{}</p>", escape(shown)));
    }
    reply::json(status, json!({ "error": error, "message": message })).header("cache-control", "no-store")
}

fn closed(s: &AppState, req: &Req, look: &Look) -> Option<Reply> {
    let w = words(req);
    match look {
        Look::Unknown => Some(refusal(s, req, 404, "not_found", EN.unknown, w.unknown)),
        Look::Gone => Some(refusal(s, req, 410, "link_used_or_expired", EN.gone, w.gone)),
        Look::WrongUser => Some(refusal(s, req, 403, "wrong_user", EN.wrong_user, w.wrong_user)),
        Look::Open { .. } => None,
    }
}

async fn page(State(s): State<AppState>, Path(token): Path<String>, req: Req) -> ApiResult {
    let look = look_up(&s, &req, &token).await?;
    if let Some(r) = closed(&s, &req, &look) {
        return Ok(r);
    }
    let Look::Open { filename, purpose, .. } = look else { return Ok(not_open()) };
    let w = words(&req);
    let what = purpose.or(filename).map_or_else(String::new, |p| format!("<p>{} {}</p>", w.for_, escape(&p)));
    let body = format!("{what}<p>{}</p><form method=\"post\" enctype=\"multipart/form-data\"><label for=\"file\">{}</label><input id=\"file\" type=\"file\" name=\"file\" multiple required><button type=\"submit\">{}</button></form>", w.intro, w.file_label, w.upload);
    Ok(html(200, s.cfg.brand.name, w.add, &body))
}

fn not_open() -> Reply {
    reply::json(404, json!({ "error": "not_found" }))
}

/// Save a raw request body, stopping at `limit`. `false` when it was over.
async fn save_raw(body: axum::body::Body, limit: u64, dest: &std::path::Path) -> Result<bool, ApiError> {
    let mut out = tokio::fs::File::create(dest).await?;
    let mut stream = body.into_data_stream();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ApiError::status(400, "the upload was interrupted"))?;
        total += chunk.len() as u64;
        if total > limit {
            return Ok(false);
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(true)
}

async fn receive(State(s): State<AppState>, Path(token): Path<String>, raw: Request) -> Result<Response, ApiError> {
    let (mut parts, body) = raw.into_parts();
    let is_multipart = parts.headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.to_ascii_lowercase().starts_with("multipart/form-data"));
    // The raw body bypasses the JSON parser and its 10 MiB limit: the request is read without it, and the bytes after.
    let (mut req, rest) = if is_multipart {
        (Req::from_request(Request::from_parts(parts, body), &s).await?, None)
    } else {
        parts.headers.remove("content-type");
        parts.headers.remove("content-length");
        parts.headers.remove("transfer-encoding");
        (Req::from_request(Request::from_parts(parts, axum::body::Body::empty()), &s).await?, Some(body))
    };
    let look = look_up(&s, &req, &token).await?;
    if let Some(r) = closed(&s, &req, &look) {
        return Ok(r.into_response());
    }
    let Look::Open { user_id, filename, .. } = look else { return Ok(not_open().into_response()) };

    let dir = TempDir::new("oc-link-").await?;
    let limit = s.cfg.max_media_bytes as u64;
    let too_large = format!("Max {} MB", crate::js::num(s.cfg.max_media_bytes as f64 / 1024f64.powi(2)));
    // (name, path on disk). One raw body, or the files of a form.
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    match rest {
        Some(body) => {
            let src = dir.0.join("raw");
            if !save_raw(body, limit, &src).await? {
                return Ok(refusal(&s, &req, 413, "too_large", &too_large, &too_large).into_response());
            }
            files.push((filename.clone().unwrap_or_else(|| "upload".into()), src));
        }
        None => {
            for (path, f) in multipart::save_files(&mut req, limit, &dir.0, MAX_FILES_PER_LINK).await? {
                if f.truncated {
                    return Ok(refusal(&s, &req, 413, "too_large", &too_large, &too_large).into_response());
                }
                files.push((f.filename, path));
            }
        }
    }
    let mut sized = Vec::new();
    for f in files {
        if tokio::fs::metadata(&f.1).await?.len() > 0 {
            sized.push(f);
        }
    }
    let files = sized;
    if files.is_empty() {
        return Ok(refusal(&s, &req, 400, "no_file", EN.no_file, words(&req).no_file).into_response());
    }

    // Single use: only one request can take the link. If nothing got in for a fixable reason, it is given back.
    let now = s.now();
    let claimed = sqlx::query("UPDATE upload_links SET used_at = $2 WHERE token_sha256 = $1 AND used_at IS NULL AND expires_at >= $2").bind(sha256(&token)).bind(now).execute(&s.pool).await?.rows_affected();
    if claimed == 0 {
        return Ok(closed(&s, &req, &Look::Gone).unwrap_or_else(not_open).into_response());
    }

    let release = || sqlx::query("UPDATE upload_links SET used_at = NULL WHERE token_sha256 = $1").bind(sha256(&token)).execute(&s.pool);
    let mut results: Vec<Value> = Vec::new();
    let mut assets: Vec<String> = Vec::new();
    let mut first_refusal: Option<(u16, &'static str, String)> = None;
    let mut blocked = false;
    for (name, src) in &files {
        let shown = if files.len() == 1 { filename.as_deref().unwrap_or(name) } else { name };
        let done = ingest_file(&s, src, &dir.0, IngestOpts { name: shown, owner_id: user_id, ip: Some(&req.ip), source: None, attribution: None, skip_quota: false }).await;
        match done {
            Err(e) => {
                if assets.is_empty() {
                    release().await?;
                }
                return Err(e);
            }
            Ok(Ingest::Error { status, error, message }) => {
                blocked |= error == "blocked";
                let m = message.unwrap_or_else(|| "The file couldn’t be stored.".to_owned());
                results.push(json!({ "file": name, "status": "refused", "error": error, "message": m }));
                first_refusal.get_or_insert((status, error, m));
            }
            Ok(Ingest::Asset(a)) => {
                let asset = a["id"].as_str().unwrap_or("").to_owned();
                results.push(json!({ "file": name, "status": "received", "asset": asset, "w": a["w"], "h": a["h"] }));
                assets.push(asset);
            }
        }
    }
    if assets.is_empty() {
        // A blocked file keeps the link closed.
        if !blocked {
            release().await?;
        }
        if files.len() == 1
            && let Some((status, error, m)) = &first_refusal
        {
            return Ok(refusal(&s, &req, *status, error, m, m).into_response());
        }
    } else {
        sqlx::query("UPDATE upload_links SET asset_ids = $2 WHERE token_sha256 = $1").bind(sha256(&token)).bind(&assets).execute(&s.pool).await?;
    }
    if wants_html(&req) {
        // Plain lines, so whoever is at the page, person or assistant, can read what arrived.
        let w = words(&req);
        let list: String = results
            .iter()
            .map(|r| {
                let name = escape(r["file"].as_str().unwrap_or(""));
                match r["asset"].as_str() {
                    Some(id) => format!("<li data-status=\"received\" data-asset=\"{id}\">{name}: {} ({id})</li>", w.received),
                    None => format!("<li data-status=\"refused\">{name}: {} ({})</li>", w.refused_file, escape(r["message"].as_str().unwrap_or(""))),
                }
            })
            .collect();
        let body = format!("<ul id=\"results\">{list}</ul><p>{}</p>", if assets.is_empty() { w.refused } else { w.uploaded_body });
        return Ok(html(200, s.cfg.brand.name, if assets.is_empty() { w.refused } else { w.uploaded }, &body).into_response());
    }
    // What an assistant reads back from `curl -T`: the asset to place with add_image { asset }.
    let mut out = json!({ "ok": !assets.is_empty(), "files": results });
    if let Some(a) = assets.first() {
        out["asset"] = json!(a);
        out["next"] = json!("Place it with add_image { asset }.");
    }
    Ok(Reply::json(out).header("cache-control", "no-store").into_response())
}
