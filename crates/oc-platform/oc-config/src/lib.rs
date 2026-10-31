//! Runtime configuration (port of `packages/config`). Every variable is `OC_*`-prefixed (plus `NODE_ENV`)
//! and has a local-dev default. Parsing follows the TS exactly: strings are taken verbatim when set
//! (even empty), integers are `parseInt(v, 10)` when the variable is non-empty.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;

/// "Pinselo" is a codename until trademark clearance (ADR-0029). The product name lives ONLY here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Brand {
    pub name: &'static str,
    pub short: &'static str,
    pub tagline: &'static str,
    pub accent: &'static str,
}

pub const BRAND: Brand = Brand { name: "Pinselo", short: "OC", tagline: "Design anything, together.", accent: "#0c7a80" };

pub const DEV_SECRET: &str = "dev-only-secret-change-me";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub brand: Brand,
    pub env: String,
    pub database_url: String,
    /// CA certificate for the database connection (plan 12 §A1): PEM text, an absolute path, or a path relative to
    /// the repo root. When set, the server certificate and hostname are verified (Supabase's CA is not in system
    /// stores).
    pub database_ssl_ca: String,
    /// Connections per process pool (Supabase session pooler: the project's Pool Size is shared by every service).
    pub database_pool_max: i64,
    /// Optional Supabase Auth (plan 12): on only when `url` and `publishable_key` are both set.
    pub supabase: SupabaseConfig,
    /// Shared secret for session signing and collab/render tickets. MUST be set in production.
    pub secret: String,
    pub web_origin: String,
    /// Origin that serves user uploads (PL-008). Empty = same origin (local dev).
    pub usercontent_origin: String,
    pub api_port: i64,
    pub collab_port: i64,
    pub render_port: i64,
    pub render_url: String,
    pub api_url: String,
    pub data_dir: String,
    /// Upload quota per user in bytes (Free plan: 5 GB).
    pub upload_quota: i64,
    pub max_upload_bytes: i64,
    pub max_media_bytes: i64,
    /// `OC_SIGNUP`: who can create an account, `open` (default) or `invite`. Not serialized: the frozen config parity
    /// (tests/golden/platform/config.json) comes from packages/config, which has no sign-up settings.
    #[serde(skip)]
    pub signup: String,
    /// `OC_SIGNUP_ALLOW`: comma-separated emails and `@domain` entries allowed to sign up in `invite` mode.
    #[serde(skip)]
    pub signup_allow: String,
}

/// `config.supabase`. The publishable key is the only one that ever reaches the browser; `secret_key` (Auth Admin
/// API) and `jwt_secret` (legacy HS256) are server-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupabaseConfig {
    /// `OC_SUPABASE_URL` without trailing slashes.
    pub url: String,
    pub publishable_key: String,
    pub secret_key: String,
    pub jwt_secret: String,
    /// Override for the expected `iss` claim (custom domains); default `{url}/auth/v1`.
    pub jwt_issuer: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    /// The TS would silently produce `NaN` here; Rust refuses instead.
    #[error("{name}={value:?} is not an integer")]
    NotAnInteger { name: &'static str, value: String },
    /// A typo must not leave sign-up open by accident.
    #[error("OC_SIGNUP={0:?} must be open or invite")]
    InvalidSignup(String),
}

impl Config {
    /// Read from the process environment.
    pub fn from_env() -> Result<Config, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from an explicit map (tests, embedding).
    pub fn from_map(env: &HashMap<String, String>) -> Result<Config, ConfigError> {
        Self::from_lookup(|k| env.get(k).cloned())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let s = |name: &str, dflt: &str| get(name).unwrap_or_else(|| dflt.to_owned());
        let i = |name: &'static str, dflt: i64| -> Result<i64, ConfigError> {
            match get(name) {
                Some(v) if !v.is_empty() => match oc_js::parse_int(&v) {
                    Some(n) if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 => Ok(n as i64),
                    _ => Err(ConfigError::NotAnInteger { name, value: v }),
                },
                _ => Ok(dflt),
            }
        };
        let signup = match s("OC_SIGNUP", "").trim().to_lowercase().as_str() {
            "" | "open" => "open",
            "invite" => "invite",
            other => return Err(ConfigError::InvalidSignup(other.to_owned())),
        };
        Ok(Config {
            brand: BRAND,
            env: s("NODE_ENV", "development"),
            database_url: s("OC_DATABASE_URL", "postgres://localhost:5432/opencanvas"),
            database_ssl_ca: s("OC_DATABASE_SSL_CA", ""),
            database_pool_max: i("OC_DATABASE_POOL_MAX", 10)?,
            supabase: SupabaseConfig {
                url: s("OC_SUPABASE_URL", "").trim_end_matches('/').to_owned(),
                publishable_key: s("OC_SUPABASE_PUBLISHABLE_KEY", ""),
                secret_key: s("OC_SUPABASE_SECRET_KEY", ""),
                jwt_secret: s("OC_SUPABASE_JWT_SECRET", ""),
                jwt_issuer: s("OC_SUPABASE_JWT_ISSUER", ""),
            },
            secret: s("OC_SECRET", DEV_SECRET),
            web_origin: s("OC_WEB_ORIGIN", "http://localhost:3000"),
            usercontent_origin: s("OC_USERCONTENT_ORIGIN", ""),
            api_port: i("OC_API_PORT", 4000)?,
            collab_port: i("OC_COLLAB_PORT", 4100)?,
            render_port: i("OC_RENDER_PORT", 4200)?,
            render_url: s("OC_RENDER_URL", "http://localhost:4200"),
            api_url: s("OC_API_URL", "http://localhost:4000"),
            data_dir: s("OC_DATA_DIR", ""),
            upload_quota: i("OC_UPLOAD_QUOTA", 5 * 1024_i64.pow(3))?,
            max_upload_bytes: i("OC_MAX_UPLOAD_BYTES", 50 * 1024_i64.pow(2))?,
            max_media_bytes: i("OC_MAX_MEDIA_BYTES", 1024_i64.pow(3))?,
            signup: signup.to_owned(),
            signup_allow: s("OC_SIGNUP_ALLOW", ""),
        })
    }

    /// Supabase Auth is optional: enabled when both the project URL and the publishable key are set.
    pub fn supabase_auth_enabled(&self) -> bool {
        !self.supabase.url.is_empty() && !self.supabase.publishable_key.is_empty()
    }

    /// The `iss` claim Supabase access tokens must carry.
    pub fn supabase_issuer(&self) -> String {
        if self.supabase.jwt_issuer.is_empty() { format!("{}/auth/v1", self.supabase.url.trim_end_matches('/')) } else { self.supabase.jwt_issuer.clone() }
    }

    /// Only one of `OC_SUPABASE_URL` / `OC_SUPABASE_PUBLISHABLE_KEY` is set.
    fn supabase_half_set(&self) -> bool {
        self.supabase.url.is_empty() != self.supabase.publishable_key.is_empty()
    }

    /// The refusal that stops every environment (not only production): the publishable key is served to browsers,
    /// so a secret key there must never start. Messages name variables, never values.
    pub fn startup_refusal(&self) -> Option<String> {
        let pk = &self.supabase.publishable_key;
        (!pk.is_empty() && (pk.starts_with("sb_secret_") || jwt_role(pk).as_deref() == Some("service_role"))).then(|| {
            "Refusing to start: OC_SUPABASE_PUBLISHABLE_KEY holds a secret (service_role) key. Use the publishable (anon) key; it is sent to browsers.".to_owned()
        })
    }

    /// Warnings printed at startup outside production.
    pub fn startup_warnings(&self) -> Vec<String> {
        if self.env != "production" && self.supabase_half_set() {
            return vec!["Supabase Auth is off: set both OC_SUPABASE_URL and OC_SUPABASE_PUBLISHABLE_KEY (only one is set).".to_owned()];
        }
        Vec::new()
    }

    /// Problems that make this config unfit for production (empty outside `NODE_ENV=production`).
    pub fn production_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.env != "production" {
            return problems;
        }
        if self.secret == DEV_SECRET || self.secret.encode_utf16().count() < 32 {
            problems.push("OC_SECRET must be a random value of 32+ characters".to_owned());
        }
        if !self.web_origin.starts_with("https://") {
            problems.push("OC_WEB_ORIGIN must be https".to_owned());
        }
        if !self.usercontent_origin.is_empty() {
            // TS: `new URL(...)` throws on an invalid origin, which also aborts startup.
            match (hostname(&self.usercontent_origin), hostname(&self.web_origin)) {
                (Some(uc), Some(web)) => {
                    if uc.ends_with(&web) {
                        problems.push("OC_USERCONTENT_ORIGIN must not be a subdomain of the app origin".to_owned());
                    }
                }
                (None, _) => problems.push(format!("Invalid URL: {}", self.usercontent_origin)),
                (_, None) => problems.push(format!("Invalid URL: {}", self.web_origin)),
            }
        }
        if self.supabase_half_set() {
            problems.push("OC_SUPABASE_URL and OC_SUPABASE_PUBLISHABLE_KEY must be set together (or both left empty for the built-in sign-in)".to_owned());
        }
        if self.supabase_auth_enabled() {
            if !self.supabase.url.starts_with("https://") {
                problems.push("OC_SUPABASE_URL must be https".to_owned());
            }
            if self.supabase.secret_key.is_empty() {
                problems.push("OC_SUPABASE_SECRET_KEY must be set when Supabase Auth is on (account deletion removes the Supabase user)".to_owned());
            }
        }
        problems
    }

    /// Refuse to boot with a secret publishable key (any environment), or a production service with development
    /// secrets or origins (exits with code 1). Messages name variables, never values.
    pub fn assert_production(&self) {
        if let Some(msg) = self.startup_refusal() {
            eprintln!("{msg}");
            std::process::exit(1);
        }
        for w in self.startup_warnings() {
            eprintln!("{w}");
        }
        let problems = self.production_problems();
        if !problems.is_empty() {
            eprintln!("Refusing to start in production:\n - {}", problems.join("\n - "));
            std::process::exit(1);
        }
    }
}

/// The `role` claim of a JWT-shaped key (legacy anon/service_role keys), or `None`. Never verifies anything. Mirrors
/// the TS: three dot-separated parts, the payload decoded like `atob` (forgiving base64 over base64url) as Latin-1 text.
fn jwt_role(key: &str) -> Option<String> {
    use base64::Engine as _;
    let parts: Vec<&str> = key.split('.').collect();
    let part = *parts.get(1)?;
    if parts.len() != 3 || part.is_empty() {
        return None;
    }
    let mut b64: String = part.chars().map(|c| match c {
        '-' => '+',
        '_' => '/',
        c => c,
    }).collect();
    while !b64.len().is_multiple_of(4) {
        b64.push('=');
    }
    const ATOB: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true).with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let bytes = ATOB.decode(b64.as_bytes()).ok()?;
    let latin1: String = bytes.iter().map(|&b| b as char).collect();
    let v: serde_json::Value = serde_json::from_str(&latin1).ok()?;
    v.get("role")?.as_str().map(str::to_owned)
}

/// WHATWG `new URL(s).hostname`.
pub fn hostname(s: &str) -> Option<String> {
    url::Url::parse(s).ok().map(|u| u.host_str().unwrap_or("").to_owned())
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Process-wide config, read once from the environment (like the TS module-level `config`).
/// Panics if an integer variable is malformed.
pub fn config() -> &'static Config {
    CONFIG.get_or_init(|| Config::from_env().unwrap_or_else(|e| panic!("invalid configuration: {e}")))
}

/// Node-only filesystem layout (port of `@oc/config/node`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub root: PathBuf,
    pub data: PathBuf,
    pub objects: PathBuf,
    pub thumbs: PathBuf,
    pub fonts: PathBuf,
    pub emoji: PathBuf,
    pub graphics: PathBuf,
    pub templates_seed: PathBuf,
}

/// The repository root. TS resolves it from the package's source location; Rust does the same from this
/// crate's manifest directory (`crates/oc-platform/oc-config` → three levels up).
pub fn repo_root() -> PathBuf {
    normalize(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

impl Paths {
    pub fn new(cfg: &Config) -> Paths {
        Self::with_root(repo_root(), cfg)
    }

    pub fn with_root(root: PathBuf, cfg: &Config) -> Paths {
        let data = if cfg.data_dir.is_empty() { root.join(".data") } else { resolve(&cfg.data_dir) };
        Paths {
            objects: data.join("objects"),
            thumbs: data.join("thumbs"),
            fonts: root.join("fixtures/fonts"),
            emoji: root.join("fixtures/emoji"),
            graphics: root.join("fixtures/graphics"),
            templates_seed: root.join("fixtures/templates-seed"),
            data,
            root,
        }
    }

    /// The TS module creates these on import.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.objects)?;
        std::fs::create_dir_all(&self.thumbs)
    }

    /// Content-addressed key: `a/{sha256[0..2]}/{sha256}` (ADR-0014).
    pub fn object_path(&self, sha256: &str) -> PathBuf {
        let pre: String = sha256.chars().take(2).collect();
        self.objects.join("a").join(pre).join(sha256)
    }
}

static PATHS: OnceLock<Paths> = OnceLock::new();

/// Process-wide paths; directories are created on first use like the TS module import.
pub fn paths() -> &'static Paths {
    PATHS.get_or_init(|| {
        let p = Paths::new(config());
        let _ = p.ensure_dirs();
        p
    })
}

/// Node `path.resolve(p)`: absolute against the cwd, lexically normalized.
pub fn resolve(p: &str) -> PathBuf {
    let p = Path::new(p);
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    normalize(&abs)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::from_map(&HashMap::new()).unwrap();
        assert_eq!(c.api_port, 4000);
        assert_eq!(c.upload_quota, 5_368_709_120);
        assert!(c.production_problems().is_empty());
    }

    #[test]
    fn nan_is_an_error() {
        let env = HashMap::from([("OC_API_PORT".to_owned(), "abc".to_owned())]);
        assert!(Config::from_map(&env).is_err());
    }

    #[test]
    fn object_path_layout() {
        let p = Paths::with_root("/r".into(), &Config::from_map(&HashMap::new()).unwrap());
        assert_eq!(p.object_path("abcdef"), PathBuf::from("/r/.data/objects/a/ab/abcdef"));
        assert!(repo_root().join("crates/oc-platform/oc-db/migrations").is_dir());
    }
}
