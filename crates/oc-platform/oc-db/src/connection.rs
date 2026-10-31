//! One place that turns `OC_DATABASE_URL` into connect options (port of `packages/db/src/connection.ts`, plan 12
//! §A1), shared by the pool, the LISTEN connections and the migrator.
//!
//! The URL is read the way `pg-connection-string` 2.14 reads it (sslmode, ssl, sslrootcert, sslcert, sslkey,
//! uselibpqcompat, pgbouncer), giving the same TLS decision as the TS ([`Tls`]), which is then mapped onto sqlx
//! explicitly: verification against `OC_DATABASE_SSL_CA` is `VerifyFull` plus `ssl_root_cert_from_pem`, never a mode
//! sqlx would read from the URL. Refuses transaction-mode poolers (LISTEN/NOTIFY and multi-statement transactions
//! need a session) and makes TLS against Supabase explicit. The guards apply to `PGHOST`/`PGPORT` too, because the
//! driver falls back to them when the URL leaves host or port empty. Messages never contain the URL.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// A refused database setup. Retrying cannot fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PgConfigError(pub String);

fn refuse(msg: impl Into<String>) -> PgConfigError {
    PgConfigError(msg.into())
}

/// A fully qualified name's trailing dot (`db.x.supabase.co.`) resolves the same, so it is ignored (E2).
pub fn is_supabase_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h.ends_with(".supabase.co") || h.ends_with(".supabase.com")
}

fn is_supabase_pooler(host: &str) -> bool {
    host.trim_end_matches('.').to_ascii_lowercase().ends_with(".pooler.supabase.com")
}

/// The TLS settings `pg` would use (`ClientConfig.ssl`), before they are mapped onto sqlx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tls {
    /// No TLS settings in the URL: the driver default applies (TS: pg reads `PGSSLMODE`; here sqlx reads it).
    Default,
    /// `ssl: false` (`sslmode=disable`, `ssl=0`).
    Off,
    /// An `ssl` object (or `ssl: true`, which is an empty one).
    On(TlsOpts),
}

/// The fields of pg's `ssl` object that matter here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TlsOpts {
    /// PEM text of the CA (`OC_DATABASE_SSL_CA` or `sslrootcert`); `None` = the built-in roots.
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
    /// `rejectUnauthorized: false` makes this `false`.
    pub reject_unauthorized: bool,
    /// `false` when uselibpqcompat sets its no-op `checkServerIdentity` (the host name is not checked).
    pub check_host: bool,
}

impl TlsOpts {
    fn verify() -> Self {
        TlsOpts { reject_unauthorized: true, check_host: true, ..Default::default() }
    }
}

/// What [`connect_options_with`] reads besides the URL (the TS reads `config` and `process.env`).
#[derive(Debug, Clone, Default)]
pub struct DbSettings {
    /// `OC_DATABASE_SSL_CA`.
    pub ssl_ca: String,
    /// `NODE_ENV === 'production'`.
    pub production: bool,
    pub pghost: Option<String>,
    pub pgport: Option<String>,
}

impl DbSettings {
    /// From the process-wide config and environment.
    pub fn from_env() -> Self {
        let c = oc_config::config();
        DbSettings { ssl_ca: c.database_ssl_ca.clone(), production: c.env == "production", pghost: std::env::var("PGHOST").ok(), pgport: std::env::var("PGPORT").ok() }
    }
}

/// The decision for one URL: connect options plus what the guards saw (for tests and logs).
#[derive(Debug, Clone)]
pub struct DbConnect {
    pub options: PgConnectOptions,
    pub tls: Tls,
    /// Host and port the guards checked (URL first, then `PGHOST`/`PGPORT`).
    pub host: String,
    pub port: Option<f64>,
}

const INVALID_URL: &str = "OC_DATABASE_URL is not a valid PostgreSQL connection URL.";

fn pooler_message(why: &str) -> String {
    format!(
        "OC_DATABASE_URL points at a transaction-mode pooler ({why}). Pinselo needs a session-mode connection: the job queue and collaboration use LISTEN/NOTIFY, and transactions span several statements. Use Supabase's session pooler (port 5432) or a direct connection."
    )
}

/// `console.warn` once per process per message.
fn warn_once(msg: String) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    if WARNED.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner()).insert(msg.clone()) {
        eprintln!("{msg}");
    }
}

/// `OC_DATABASE_SSL_CA` as PEM text: inline PEM, an absolute path, or a path relative to the repo root (never cwd).
fn read_ca(setting: &str) -> Result<String, PgConfigError> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(setting) {
        return Ok(hit.clone());
    }
    let s = setting.trim();
    let pem = if s.starts_with("-----BEGIN") {
        // Inline PEM; env files often carry it on one line with literal \n.
        setting.replace("\\n", "\n")
    } else {
        let file = if Path::new(s).is_absolute() { Path::new(s).to_path_buf() } else { oc_config::repo_root().join(s) };
        if !file.exists() {
            return Err(refuse("OC_DATABASE_SSL_CA names a file that does not exist (an absolute path, or a path relative to the repo root)."));
        }
        std::fs::read_to_string(&file).map_err(|_| refuse("OC_DATABASE_SSL_CA names a file that cannot be read."))?
    };
    if !pem.contains("-----BEGIN CERTIFICATE-----") {
        return Err(refuse("OC_DATABASE_SSL_CA does not contain a PEM certificate."));
    }
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(setting.to_owned(), pem.clone());
    Ok(pem)
}

/// Why `tls` does not amount to verify-full, or `None` when it does (or when TLS is left to the driver default).
fn tls_weakness(tls: &Tls) -> Option<&'static str> {
    match tls {
        Tls::Off => Some("TLS is off (sslmode=disable or ssl=0)"),
        Tls::Default => None,
        Tls::On(o) if !o.reject_unauthorized => Some("TLS certificate verification is off (sslmode=no-verify, or uselibpqcompat with sslmode=prefer or require)"),
        Tls::On(o) if !o.check_host => Some("the server host name is not verified (uselibpqcompat with sslmode=require or verify-ca)"),
        Tls::On(_) => None,
    }
}

/// Parameters that only this module interprets; sqlx never sees them (it would read sslmode itself, and fails on
/// `sslmode=no-verify`).
const OWN_PARAMS: [&str; 7] = ["sslmode", "ssl", "sslrootcert", "sslcert", "sslkey", "uselibpqcompat", "pgbouncer"];

/// Explicit connect options for `OC_DATABASE_URL`-style `url` (process config and environment). Refuses the setups
/// the TS refuses ([`PgConfigError`]).
pub fn connect_options(url: &str) -> Result<PgConnectOptions, PgConfigError> {
    connect_options_with(url, &DbSettings::from_env()).map(|c| c.options)
}

/// [`connect_options`] with explicit settings.
pub fn connect_options_with(url: &str, s: &DbSettings) -> Result<DbConnect, PgConfigError> {
    // pg-connection-string retries a URL with an empty host (`postgres://u:p@/db`) with a dummy one.
    let (mut parsed, hostless) = match url::Url::parse(url) {
        Ok(u) => (u, false),
        Err(_) => (url::Url::parse(&url.replacen("@/", "@___DUMMY___/", 1)).map_err(|_| refuse(INVALID_URL))?, true),
    };
    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return Err(refuse(INVALID_URL));
    }
    let params: HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let p = |k: &str| params.get(k).map(String::as_str);

    // Host and port as pg sees them: the URL (a `host`/`port` parameter wins), else PGHOST/PGPORT.
    let url_host = if hostless { String::new() } else { pct_decode(parsed.host_str().unwrap_or("")) };
    let host = p("host").map(str::to_owned).filter(|h| !h.is_empty()).or_else(|| (!url_host.is_empty()).then_some(url_host.clone()));
    let host = host.or_else(|| s.pghost.clone()).unwrap_or_default();
    let port_str = p("port").map(str::to_owned).or_else(|| parsed.port().map(|n| n.to_string())).filter(|v| !v.is_empty());
    let port: Option<f64> = match port_str.or_else(|| s.pgport.clone().filter(|v| !v.is_empty())) {
        Some(v) => js_number(&v),
        None => Some(5432.0),
    };
    let supabase = is_supabase_host(&host);

    if port == Some(6543.0) {
        return Err(refuse(pooler_message("port 6543")));
    }
    if p("pgbouncer") == Some("true") {
        return Err(refuse(pooler_message("pgbouncer=true")));
    }
    if is_supabase_pooler(&host) && port != Some(5432.0) {
        return Err(refuse(pooler_message(&format!("port {}", port.map_or("NaN".to_owned(), |n| n.to_string())))));
    }

    let sslmode = p("sslmode");
    let mut tls = parse_tls(&params)?;
    let ca_setting = if s.ssl_ca.trim().is_empty() { "" } else { s.ssl_ca.as_str() };
    if tls == Tls::Off {
        if !ca_setting.is_empty() {
            return Err(refuse("OC_DATABASE_URL turns TLS off (sslmode=disable or ssl=0) but OC_DATABASE_SSL_CA is set; remove one of them."));
        }
    } else if !ca_setting.is_empty() && sslmode != Some("no-verify") {
        // verify-full: the CA must sign the chain and the host name is checked. Built field by field, so
        // uselibpqcompat's skipped host name check never survives.
        let (cert, key) = match &tls {
            Tls::On(o) => (o.cert.clone(), o.key.clone()),
            _ => (None, None),
        };
        tls = Tls::On(TlsOpts { ca: Some(read_ca(ca_setting)?), cert, key, ..TlsOpts::verify() });
    } else if sslmode.is_none() && tls == Tls::Default && supabase {
        return Err(refuse(
            "OC_DATABASE_URL points at Supabase without TLS settings. Set OC_DATABASE_SSL_CA to the Supabase CA certificate (Dashboard → Database Settings → SSL Configuration).",
        ));
    }

    let weakness = tls_weakness(&tls);
    if let Some(w) = weakness.filter(|_| supabase) {
        // Every Supabase project's certificate chains to the same CA, so anything short of verify-full can be intercepted.
        if s.production {
            return Err(refuse(format!("OC_DATABASE_URL weakens TLS for Supabase: {w}. Set OC_DATABASE_SSL_CA to the Supabase CA certificate and remove that setting.")));
        }
        warn_once(format!("[db] {w}: the connection to Supabase can be intercepted. Set OC_DATABASE_SSL_CA instead."));
    } else if let Some(w) = weakness.filter(|_| sslmode == Some("no-verify")) {
        warn_once(format!("[db] {w}: the connection can be intercepted. Set OC_DATABASE_SSL_CA instead."));
    }

    // What sqlx parses: the URL without our parameters, with the host pg would use when the URL has none.
    let kept: Vec<(String, String)> = parsed.query_pairs().filter(|(k, _)| !OWN_PARAMS.contains(&k.as_ref())).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(kept);
    }
    if hostless {
        let h = s.pghost.clone().filter(|h| !h.is_empty()).unwrap_or_else(|| "localhost".to_owned());
        parsed.set_host(Some(&h)).map_err(|_| refuse(INVALID_URL))?;
    }
    let mut options = PgConnectOptions::from_str(parsed.as_str()).map_err(|_| refuse(INVALID_URL))?;
    options = match &tls {
        Tls::Default => options,
        Tls::Off => options.ssl_mode(PgSslMode::Disable),
        Tls::On(o) => {
            let mode = if !o.reject_unauthorized {
                PgSslMode::Require
            } else if o.check_host {
                PgSslMode::VerifyFull
            } else {
                PgSslMode::VerifyCa
            };
            let mut opts = options.ssl_mode(mode);
            if let Some(ca) = &o.ca {
                opts = opts.ssl_root_cert_from_pem(ca.clone().into_bytes());
            }
            if let Some(cert) = &o.cert {
                opts = opts.ssl_client_cert_from_pem(cert.as_bytes());
            }
            if let Some(key) = &o.key {
                opts = opts.ssl_client_key_from_pem(key.as_bytes());
            }
            opts
        }
    };
    Ok(DbConnect { options, tls, host, port })
}

/// pg-connection-string's `ssl` result for the URL parameters.
fn parse_tls(params: &HashMap<String, String>) -> Result<Tls, PgConfigError> {
    let p = |k: &str| params.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let mut tls = match params.get("ssl").map(String::as_str) {
        None => Tls::Default,
        Some("0") => Tls::Off,
        // `true`, `1`, and any other non-empty string (truthy for pg).
        Some("") => Tls::Default,
        Some(_) => Tls::On(TlsOpts::verify()),
    };
    let read = |k: &str| -> Result<Option<String>, PgConfigError> { p(k).map(|f| std::fs::read_to_string(f).map_err(|_| refuse(INVALID_URL))).transpose() };
    if p("sslcert").is_some() || p("sslkey").is_some() || p("sslrootcert").is_some() || p("sslmode").is_some() {
        tls = Tls::On(TlsOpts { cert: read("sslcert")?, key: read("sslkey")?, ca: read("sslrootcert")?, ..TlsOpts::verify() });
    }
    let libpq = p("uselibpqcompat") == Some("true");
    match (p("sslmode"), &mut tls) {
        (Some("disable"), _) => tls = Tls::Off,
        (Some("prefer"), Tls::On(o)) if libpq => o.reject_unauthorized = false,
        (Some("require"), Tls::On(o)) if libpq => {
            if p("sslrootcert").is_some() {
                o.check_host = false;
            } else {
                o.reject_unauthorized = false;
            }
        }
        (Some("verify-ca"), Tls::On(o)) if libpq => {
            if o.ca.is_none() {
                // pg-connection-string throws ("SECURITY WARNING: Using sslmode=verify-ca requires ... sslrootcert").
                return Err(refuse(INVALID_URL));
            }
            o.check_host = false;
        }
        (Some("no-verify"), Tls::On(o)) if !libpq => o.reject_unauthorized = false,
        _ => {}
    }
    Ok(tls)
}

/// `decodeURIComponent` for a host name (lossy on invalid UTF-8).
fn pct_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("h={}", s.replace('+', "%2B")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

/// JS `Number(v)` for a port string (`NaN` → `None`).
fn js_number(v: &str) -> Option<f64> {
    let t = v.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBfakefakefakefake\n-----END CERTIFICATE-----\n";
    const PASSWORD: &str = "S3cr3t-p4ss_w0rd";

    fn direct(q: &str) -> String {
        format!("postgres://postgres:{PASSWORD}@db.abcdefghijklmnop.supabase.co:5432/postgres{q}")
    }
    fn session(q: &str) -> String {
        format!("postgres://postgres.abcdefghijklmnop:{PASSWORD}@aws-0-eu-central-1.pooler.supabase.com:5432/postgres{q}")
    }
    fn local(q: &str) -> String {
        format!("postgres://oc:{PASSWORD}@db.internal:5432/opencanvas{q}")
    }
    fn dev() -> DbSettings {
        DbSettings::default()
    }
    fn prod() -> DbSettings {
        DbSettings { production: true, ..Default::default() }
    }
    fn with_ca(mut s: DbSettings, ca: &str) -> DbSettings {
        s.ssl_ca = ca.to_owned();
        s
    }
    fn tls(url: &str, s: &DbSettings) -> Tls {
        connect_options_with(url, s).unwrap_or_else(|e| panic!("{e}")).tls
    }
    fn ca_tls() -> Tls {
        Tls::On(TlsOpts { ca: Some(PEM.to_owned()), ..TlsOpts::verify() })
    }
    /// The refusal for `url`, checked to never contain the URL or the password.
    fn refusal(url: &str, s: &DbSettings) -> String {
        let e = connect_options_with(url, s).expect_err("expected a PgConfigError").0;
        assert!(!e.contains(PASSWORD) && !e.contains(url), "{e}");
        e
    }
    fn tmp_file(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("oc-db-conn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(name);
        std::fs::write(&f, body).unwrap();
        f.to_string_lossy().into_owned()
    }

    #[test]
    fn explicit_fields() {
        let c = connect_options_with("postgres://user%40x:p%40ss@db.internal:6000/my_db?application_name=oc", &dev()).unwrap();
        assert_eq!(c.options.get_host(), "db.internal");
        assert_eq!(c.options.get_port(), 6000);
        assert_eq!(c.options.get_username(), "user@x");
        assert_eq!(c.options.get_database(), Some("my_db"));
        assert_eq!(c.options.get_application_name(), Some("oc"));
        assert_eq!(c.tls, Tls::Default);
        assert!(refusal(&format!("postgres://u:{PASSWORD}@host:notaport/x"), &dev()).contains("not a valid"));
    }

    #[test]
    fn supabase_hosts() {
        assert!(is_supabase_host("db.abc.supabase.co"));
        assert!(is_supabase_host("aws-0-eu-central-1.pooler.supabase.com"));
        assert!(!is_supabase_host("supabase.co.evil.example"));
        assert!(is_supabase_host("db.abc.supabase.co."));
        assert!(is_supabase_host("aws-0-eu-central-1.pooler.supabase.com.."));
        assert!(!is_supabase_host("supabase.co.evil.example."));
        assert!(!is_supabase_host("localhost"));
    }

    #[test]
    fn tls_table() {
        assert_eq!(tls(&local("?sslmode=disable"), &dev()), Tls::Off);
        assert!(refusal(&local("?sslmode=disable"), &with_ca(dev(), PEM)).contains("OC_DATABASE_SSL_CA"));
        for mode in ["", "?sslmode=prefer", "?sslmode=require", "?sslmode=verify-ca", "?sslmode=verify-full"] {
            let c = connect_options_with(&direct(mode), &with_ca(dev(), PEM)).unwrap();
            assert_eq!(c.tls, ca_tls(), "{mode}");
            assert!(matches!(c.options.get_ssl_mode(), PgSslMode::VerifyFull), "{mode}");
        }
        for mode in ["prefer", "require", "verify-ca", "verify-full"] {
            let c = connect_options_with(&local(&format!("?sslmode={mode}")), &dev()).unwrap();
            assert_eq!(c.tls, Tls::On(TlsOpts::verify()), "{mode}");
            assert!(matches!(c.options.get_ssl_mode(), PgSslMode::VerifyFull), "{mode}");
        }
        // No TLS settings: unchanged for plain hosts, refused for Supabase.
        assert_eq!(tls(&local(""), &dev()), Tls::Default);
        assert_eq!(tls("postgres://localhost:5432/opencanvas", &dev()), Tls::Default);
        assert!(refusal(&direct(""), &dev()).contains("Set OC_DATABASE_SSL_CA"));
        assert!(refusal(&session(""), &dev()).contains("Database Settings → SSL Configuration"));
    }

    #[test]
    fn ca_sources() {
        let abs = tmp_file("ca.crt", PEM);
        let rel_dir = oc_config::repo_root().join(".data");
        std::fs::create_dir_all(&rel_dir).unwrap();
        let rel = format!(".data/test-ca-rust-{}.crt", std::process::id());
        std::fs::write(oc_config::repo_root().join(&rel), PEM).unwrap();
        for setting in [PEM.to_owned(), PEM.replace('\n', "\\n"), abs, rel.clone()] {
            assert_eq!(tls(&session(""), &with_ca(dev(), &setting)), ca_tls());
        }
        std::fs::remove_file(oc_config::repo_root().join(&rel)).ok();
        let missing = std::env::temp_dir().join("oc-missing-ca.crt").to_string_lossy().into_owned();
        let e = refusal(&session(""), &with_ca(dev(), &missing));
        assert!(e.contains("OC_DATABASE_SSL_CA") && !e.contains("missing"));
        assert!(refusal(&session(""), &with_ca(dev(), &tmp_file("not-pem.txt", "hello"))).contains("PEM certificate"));
    }

    #[test]
    fn sslrootcert_and_no_verify() {
        let abs = tmp_file("root.crt", PEM);
        let enc: String = url::form_urlencoded::byte_serialize(abs.as_bytes()).collect();
        assert_eq!(tls(&direct(&format!("?sslmode=verify-full&sslrootcert={enc}")), &dev()), ca_tls());
        let nv = Tls::On(TlsOpts { reject_unauthorized: false, ..TlsOpts::verify() });
        let c = connect_options_with(&direct("?sslmode=no-verify"), &dev()).unwrap();
        assert_eq!(c.tls, nv);
        assert!(matches!(c.options.get_ssl_mode(), PgSslMode::Require));
        assert_eq!(tls(&local("?sslmode=no-verify"), &with_ca(dev(), PEM)), nv);
        // Production: refused for Supabase, the operator's choice elsewhere.
        assert!(refusal(&direct("?sslmode=no-verify"), &prod()).contains("OC_DATABASE_SSL_CA"));
        assert_eq!(tls(&local("?sslmode=no-verify"), &prod()), nv);
    }

    #[test]
    fn pooler_guard() {
        let ca = with_ca(dev(), PEM);
        assert!(refusal(&format!("postgres://postgres.abc:{PASSWORD}@aws-0-eu-central-1.pooler.supabase.com:6543/postgres"), &ca).contains("transaction-mode pooler (port 6543)"));
        assert!(refusal(&format!("postgres://postgres:{PASSWORD}@db.abc.supabase.co:6543/postgres"), &ca).contains("session pooler (port 5432)"));
        assert!(refusal(&format!("postgres://oc:{PASSWORD}@pgbouncer.internal:6543/opencanvas"), &ca).contains("LISTEN/NOTIFY"));
        assert!(refusal(&local("?pgbouncer=true"), &dev()).contains("pgbouncer=true"));
        assert!(refusal(&format!("postgres://postgres.abc:{PASSWORD}@aws-0-eu-central-1.pooler.supabase.com:6544/postgres"), &ca).contains("port 6544"));
        assert!(refusal(&format!("postgres://postgres.abc:{PASSWORD}@aws-0-eu-central-1.pooler.supabase.com.:6544/postgres"), &ca).contains("port 6544"));
        let c = connect_options_with(&session(""), &ca).unwrap();
        assert_eq!((c.options.get_host(), c.options.get_port(), c.options.get_username()), ("aws-0-eu-central-1.pooler.supabase.com", 5432, "postgres.abcdefghijklmnop"));
        assert!(connect_options_with(&direct(""), &ca).is_ok());
        // A trailing-dot Supabase host gets the Supabase guards (E2).
        let td = format!("postgres://postgres:{PASSWORD}@db.abcdefghijklmnop.supabase.co.:5432/postgres");
        assert!(refusal(&td, &prod()).contains("Set OC_DATABASE_SSL_CA"));
        assert!(refusal(&format!("{td}?sslmode=disable"), &prod()).contains("TLS is off"));
    }

    #[test]
    fn tls_off_or_weakened_for_supabase() {
        for q in ["?sslmode=disable", "?ssl=0"] {
            assert!(refusal(&direct(q), &prod()).contains("TLS is off"));
            assert!(refusal(&session(q), &prod()).contains("OC_DATABASE_SSL_CA"));
            assert_eq!(tls(&direct(q), &dev()), Tls::Off);
            assert_eq!(tls(&local(q), &prod()), Tls::Off);
        }
        assert!(refusal(&local("?ssl=0"), &with_ca(dev(), PEM)).contains("OC_DATABASE_SSL_CA"));
        let abs = tmp_file("compat.crt", PEM);
        let enc: String = url::form_urlencoded::byte_serialize(abs.as_bytes()).collect();
        // uselibpqcompat cannot switch the host name check off when a CA is set.
        for mode in ["require", "verify-ca", "prefer"] {
            let c = connect_options_with(&direct(&format!("?uselibpqcompat=true&sslmode={mode}&sslrootcert={enc}")), &with_ca(dev(), PEM)).unwrap();
            assert_eq!(c.tls, ca_tls(), "{mode}");
            assert!(matches!(c.options.get_ssl_mode(), PgSslMode::VerifyFull));
        }
        // Without a CA: a Supabase host whose name is not checked is refused in production.
        let url = direct(&format!("?uselibpqcompat=true&sslmode=verify-ca&sslrootcert={enc}"));
        assert!(refusal(&url, &prod()).contains("host name is not verified"));
        assert!(refusal(&direct("?uselibpqcompat=true&sslmode=require"), &prod()).contains("verification is off"));
        let c = connect_options_with(&url, &dev()).unwrap();
        assert!(matches!(&c.tls, Tls::On(o) if !o.check_host && o.reject_unauthorized));
        assert!(matches!(c.options.get_ssl_mode(), PgSslMode::VerifyCa));
    }

    #[test]
    fn client_certificates_are_kept_with_a_ca() {
        let cert = tmp_file("client.crt", "CLIENT-CERT");
        let key = tmp_file("client.key", "CLIENT-KEY");
        let e = |s: &str| -> String { url::form_urlencoded::byte_serialize(s.as_bytes()).collect() };
        let t = tls(&local(&format!("?sslmode=verify-full&sslcert={}&sslkey={}", e(&cert), e(&key))), &with_ca(dev(), PEM));
        assert_eq!(t, Tls::On(TlsOpts { ca: Some(PEM.to_owned()), cert: Some("CLIENT-CERT".into()), key: Some("CLIENT-KEY".into()), ..TlsOpts::verify() }));
    }

    #[test]
    fn pghost_and_pgport() {
        let hostless = |q: &str| format!("postgres://postgres:{PASSWORD}@/postgres{q}");
        let env = |h: &str, p: Option<&str>| DbSettings { pghost: Some(h.into()), pgport: p.map(Into::into), ..Default::default() };
        let sb = env("db.abcdefghijklmnop.supabase.co", None);
        assert!(refusal(&hostless(""), &sb).contains("Set OC_DATABASE_SSL_CA"));
        let sbp = DbSettings { production: true, ..sb.clone() };
        assert!(refusal(&hostless("?sslmode=disable"), &sbp).contains("TLS is off"));
        assert!(refusal(&hostless("?sslmode=no-verify"), &sbp).contains("verification is off"));
        let c = connect_options_with(&hostless(""), &with_ca(sb, PEM)).unwrap();
        assert_eq!(c.tls, ca_tls());
        assert_eq!(c.options.get_host(), "db.abcdefghijklmnop.supabase.co");
        let pooler = |p: &str| with_ca(env("aws-0-eu-central-1.pooler.supabase.com", Some(p)), PEM);
        assert!(refusal(&hostless(""), &pooler("6543")).contains("port 6543"));
        let c = connect_options_with(&hostless(""), &pooler("5432")).unwrap();
        assert_eq!((c.options.get_username(), c.options.get_database()), ("postgres", Some("postgres")));
        // A URL host wins over PGHOST.
        assert_eq!(tls(&local(""), &env("db.abcdefghijklmnop.supabase.co", None)), Tls::Default);
    }
}
