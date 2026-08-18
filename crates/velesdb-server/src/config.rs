//! Server configuration module.
//!
//! Loads configuration from multiple sources with priority:
//! CLI flags > environment variables > velesdb.toml > defaults.

use serde::Deserialize;
use std::path::{Path, PathBuf};

// ============================================================================
// Core engine configuration (issue #1549)
// ============================================================================

/// Loads the core [`velesdb_core::config::VelesConfig`] (search/HNSW/storage/
/// limits/quantization/WAL batching) from the same TOML file consumed by
/// [`ServerConfig::load`] for the server-transport sections (`[server]`,
/// `[auth]`, `[tls]`, `[cors]`).
///
/// The two structs deserialize independently from the same file, but
/// **`VelesConfig` also has its own `[server]` and `[logging]` fields**
/// (for standalone/embedded consumers) that collide in key name — not
/// meaning — with this crate's own `[server]` transport section. A
/// `[server] port = 443` meant as this crate's HTTP bind port would
/// otherwise *also* land in `VelesConfig.server.port` and be rejected by
/// its `>= 1024` validation rule, a spurious failure unrelated to the
/// value actually being configured. This function therefore uses
/// [`velesdb_core::config::VelesConfig::load_from_path_engine_only`],
/// which parses **only** the engine sections (`[search]`/`[hnsw]`/
/// `[storage]`/`[limits]`/`[quantization]`/`[wal_batch]`) and silently
/// drops every other top-level table before validating — so
/// `[server]`/`[auth]`/`[tls]`/`[cors]` stay exclusively owned by
/// [`ServerConfig::load`].
///
/// `VELESDB_*` environment variables still layer on top of the (filtered)
/// file, same as [`velesdb_core::config::VelesConfig::load_from_path`] —
/// e.g. `VELESDB_LIMITS_MAX_COLLECTIONS=5` overrides a `[limits]` value
/// from the file even though the file is filtered first.
///
/// - `config_path: Some(path)` — the caller passed `--config`/`VELESDB_CONFIG`
///   explicitly. The file **must** exist and pass
///   `load_from_path_engine_only` validation — never a silent fallback to
///   defaults on a broken path.
/// - `config_path: None` — mirrors [`load_toml_file`]'s default-file
///   behaviour: falls back to `velesdb.toml` in the current directory if
///   present (still validated, so a malformed default file still fails
///   fast), otherwise core defaults apply.
///
/// # Errors
///
/// Returns an error if an explicit path is missing, or if the file fails to
/// parse or validate. The underlying typed
/// [`velesdb_core::config::ConfigError`] is preserved as the error source so
/// callers can match on it if needed.
pub fn load_core_config(
    config_path: &Option<PathBuf>,
) -> anyhow::Result<velesdb_core::config::VelesConfig> {
    let path = match config_path {
        Some(p) => {
            if !p.exists() {
                anyhow::bail!("config file not found: {}", p.display());
            }
            p.clone()
        }
        None => {
            let default_path = PathBuf::from("velesdb.toml");
            if !default_path.exists() {
                return Ok(velesdb_core::config::VelesConfig::default());
            }
            default_path
        }
    };

    velesdb_core::config::VelesConfig::load_from_path_engine_only(&path).map_err(|e| {
        anyhow::anyhow!(
            "failed to load VelesDB core config from {}: {e}",
            path.display()
        )
    })
}

// ============================================================================
// TOML file configuration (all fields optional)
// ============================================================================

/// Root structure for `velesdb.toml`.
#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    server: Option<ServerSection>,
    auth: Option<AuthSection>,
    tls: Option<TlsSection>,
    cors: Option<CorsSection>,
}

#[derive(Debug, Deserialize, Default)]
struct ServerSection {
    host: Option<String>,
    port: Option<u16>,
    data_dir: Option<String>,
    shutdown_timeout_secs: Option<u64>,
    rate_limit: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
struct AuthSection {
    api_keys: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct TlsSection {
    cert: Option<String>,
    key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct CorsSection {
    allowed_origins: Option<Vec<String>>,
    allowed_methods: Option<Vec<String>>,
    allowed_headers: Option<Vec<String>>,
    allow_credentials: Option<bool>,
    max_age_secs: Option<u64>,
}

// ============================================================================
// Resolved configuration
// ============================================================================

/// TLS certificate and key paths.
///
/// Both fields must be `Some` together or both `None`; a partial pair is
/// rejected by [`ServerConfig::validate`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TlsConfig {
    /// Path to the PEM-encoded TLS certificate file.
    pub cert: Option<String>,
    /// Path to the PEM-encoded TLS private key file.
    pub key: Option<String>,
}

impl TlsConfig {
    /// Returns `true` when both cert and key are configured.
    pub fn is_enabled(&self) -> bool {
        self.cert.is_some() && self.key.is_some()
    }
}

/// Final resolved server configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: String,
    pub api_keys: Vec<String>,
    /// TLS certificate and key configuration (both or neither).
    pub tls: TlsConfig,
    pub shutdown_timeout_secs: u64,
    /// Maximum requests per second per IP address (0 = disabled).
    pub rate_limit: u32,
    /// CORS configuration for cross-origin requests.
    pub cors: CorsConfig,
}

/// CORS configuration for the server.
///
/// When `allowed_origins` contains `"*"`, the server uses a fully permissive
/// CORS policy (equivalent to `CorsLayer::permissive()`). Otherwise, only the
/// listed origins are allowed.
///
/// Defaults to permissive (`["*"]`) for backward compatibility.
#[derive(Debug, Clone, PartialEq)]
pub struct CorsConfig {
    /// Allowed origins. Use `["*"]` for permissive mode.
    pub allowed_origins: Vec<String>,
    /// Allowed HTTP methods (e.g. `["GET", "POST"]`).
    pub allowed_methods: Vec<String>,
    /// Allowed request headers (e.g. `["Content-Type", "Authorization"]`).
    /// Use `["*"]` to allow any header.
    pub allowed_headers: Vec<String>,
    /// Whether to allow credentials (cookies, authorization headers).
    pub allow_credentials: bool,
    /// How long (in seconds) browsers may cache preflight responses.
    pub max_age_secs: u64,
}

/// Default burst budget for rate limiting (requests per second per IP).
const DEFAULT_RATE_LIMIT: u32 = 100;

/// Default preflight cache duration in seconds (1 hour).
const DEFAULT_CORS_MAX_AGE_SECS: u64 = 3600;

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec!["*".to_string()],
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            allowed_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age_secs: DEFAULT_CORS_MAX_AGE_SECS,
        }
    }
}

impl CorsConfig {
    /// Returns `true` when CORS is in fully permissive mode (any origin).
    pub fn is_permissive(&self) -> bool {
        self.allowed_origins.iter().any(|o| o == "*")
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            data_dir: "./velesdb_data".to_string(),
            api_keys: Vec::new(),
            tls: TlsConfig::default(),
            shutdown_timeout_secs: 30,
            rate_limit: DEFAULT_RATE_LIMIT,
            cors: CorsConfig::default(),
        }
    }
}

// ============================================================================
// Loading logic
// ============================================================================

impl ServerConfig {
    /// Load configuration with priority: CLI > env > TOML file > defaults.
    ///
    /// `cli` contains values from clap (which merges CLI flags + env vars).
    /// `cli_sources` indicates which fields were explicitly set via CLI/env
    /// (as opposed to falling back to clap defaults).
    pub fn load(cli: CliOverrides) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let file_cfg = load_toml_file(&cli.config_path)?;
        Ok(Self::merge(defaults, file_cfg, cli))
    }

    fn merge(defaults: Self, file: FileConfig, cli: CliOverrides) -> Self {
        let server = file.server.unwrap_or_default();
        let auth = file.auth.unwrap_or_default();
        let tls = file.tls.unwrap_or_default();
        let cors_section = file.cors.unwrap_or_default();

        // Layer: TOML over defaults
        let host = server.host.unwrap_or(defaults.host);
        let port = server.port.unwrap_or(defaults.port);
        let data_dir = server.data_dir.unwrap_or(defaults.data_dir);
        let shutdown_timeout_secs = server
            .shutdown_timeout_secs
            .unwrap_or(defaults.shutdown_timeout_secs);
        let rate_limit = server.rate_limit.unwrap_or(defaults.rate_limit);
        let api_keys = auth.api_keys.unwrap_or(defaults.api_keys);
        let tls = TlsConfig {
            cert: tls.cert.or(defaults.tls.cert),
            key: tls.key.or(defaults.tls.key),
        };
        let cors = resolve_cors(defaults.cors, cors_section);

        // Layer: CLI/env over TOML (only override when explicitly set)
        let host = cli.host.unwrap_or(host);
        let port = cli.port.unwrap_or(port);
        let data_dir = cli.data_dir.unwrap_or(data_dir);
        let api_keys = cli.api_keys.unwrap_or(api_keys);
        let tls = TlsConfig {
            cert: cli.tls_cert.or(tls.cert),
            key: cli.tls_key.or(tls.key),
        };
        let rate_limit = cli.rate_limit.unwrap_or(rate_limit);

        Self {
            host,
            port,
            data_dir,
            api_keys,
            tls,
            shutdown_timeout_secs,
            rate_limit,
            cors,
        }
    }

    /// Validate the configuration at startup.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.port == 0 {
            anyhow::bail!("invalid port: 0 is not allowed");
        }
        if self.data_dir.is_empty() {
            anyhow::bail!("data_dir must not be empty");
        }

        // TLS: both cert and key must be provided together
        match (&self.tls.cert, &self.tls.key) {
            (Some(_), None) => {
                anyhow::bail!("tls_cert is set but tls_key is missing");
            }
            (None, Some(_)) => {
                anyhow::bail!("tls_key is set but tls_cert is missing");
            }
            (Some(cert), Some(key)) => {
                if !Path::new(cert).exists() {
                    anyhow::bail!("TLS cert file not found: {cert}");
                }
                if !Path::new(key).exists() {
                    anyhow::bail!("TLS key file not found: {key}");
                }
            }
            (None, None) => {}
        }

        Ok(())
    }

    /// Returns `true` when API key authentication is enabled.
    pub fn auth_enabled(&self) -> bool {
        !self.api_keys.is_empty()
    }

    /// Returns `true` when TLS is configured.
    pub fn tls_enabled(&self) -> bool {
        self.tls.is_enabled()
    }

    /// Returns `true` when rate limiting is enabled (rate_limit > 0).
    pub fn rate_limit_enabled(&self) -> bool {
        self.rate_limit > 0
    }

    /// Returns `true` when the bind host is reachable beyond the local machine.
    ///
    /// A loopback host is treated as private: `localhost`, any `127.0.0.0/8`
    /// address (`127.0.0.1`, `127.0.0.5`, …), IPv6 loopback `::1`, and the
    /// IPv4-mapped form `::ffff:127.0.0.1`. Matching is case-insensitive and
    /// tolerates surrounding whitespace and `[...]` brackets. Anything else —
    /// including the wildcards `0.0.0.0`/`::` and any routable address or
    /// hostname — is considered publicly reachable. Errs toward *over*-warning
    /// (an unrecognised host is treated as public), never under-warning.
    pub fn binds_publicly(&self) -> bool {
        // Case-insensitive, whitespace- and bracket-tolerant (`[::1]`).
        let host = self
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        if matches!(host.as_str(), "localhost" | "::1") || host.starts_with("127.") {
            return false;
        }
        // IPv4-mapped IPv6 loopback, e.g. `::ffff:127.0.0.1`.
        if let Some(v4) = host.strip_prefix("::ffff:") {
            if v4.starts_with("127.") {
                return false;
            }
        }
        true
    }
}

// ============================================================================
// CLI overrides (filled by clap in main.rs)
// ============================================================================

/// Values explicitly provided via CLI flags or environment variables.
/// `None` means "not provided — fall through to TOML or default".
#[derive(Debug, Default)]
pub struct CliOverrides {
    pub config_path: Option<PathBuf>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub data_dir: Option<String>,
    pub api_keys: Option<Vec<String>>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub rate_limit: Option<u32>,
}

// ============================================================================
// TOML file loader
// ============================================================================

fn load_toml_file(path: &Option<PathBuf>) -> anyhow::Result<FileConfig> {
    let candidate = match path {
        Some(p) => {
            if !p.exists() {
                anyhow::bail!("config file not found: {}", p.display());
            }
            p.clone()
        }
        None => {
            let default_path = PathBuf::from("velesdb.toml");
            if !default_path.exists() {
                return Ok(FileConfig::default());
            }
            default_path
        }
    };

    let contents = std::fs::read_to_string(&candidate)
        .map_err(|e| anyhow::anyhow!("failed to read config file {}: {e}", candidate.display()))?;

    let cfg: FileConfig = toml::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("failed to parse config file {}: {e}", candidate.display()))?;

    Ok(cfg)
}

// ============================================================================
// CORS resolution & layer builder
// ============================================================================

/// Merges a `CorsSection` (from TOML) over `CorsConfig` defaults.
fn resolve_cors(defaults: CorsConfig, section: CorsSection) -> CorsConfig {
    CorsConfig {
        allowed_origins: section.allowed_origins.unwrap_or(defaults.allowed_origins),
        allowed_methods: section.allowed_methods.unwrap_or(defaults.allowed_methods),
        allowed_headers: section.allowed_headers.unwrap_or(defaults.allowed_headers),
        allow_credentials: section
            .allow_credentials
            .unwrap_or(defaults.allow_credentials),
        max_age_secs: section.max_age_secs.unwrap_or(defaults.max_age_secs),
    }
}

/// Builds a [`tower_http::cors::CorsLayer`] from the resolved CORS config.
///
/// When `allowed_origins` contains `"*"`, returns `CorsLayer::permissive()`
/// for full backward compatibility. Otherwise, constructs a restrictive
/// layer with the specified origins, methods, and headers.
pub fn build_cors_layer(cors: &CorsConfig) -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};

    if cors.is_permissive() {
        return CorsLayer::permissive();
    }

    let origins: Vec<axum::http::HeaderValue> = cors
        .allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let methods: Vec<axum::http::Method> = cors
        .allowed_methods
        .iter()
        .filter_map(|m| m.parse().ok())
        .collect();

    let layer = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(methods)
        .max_age(std::time::Duration::from_secs(cors.max_age_secs));

    let layer = apply_cors_headers_policy(layer, cors);

    if cors.allow_credentials {
        layer.allow_credentials(true)
    } else {
        layer
    }
}

/// Applies the headers policy to a `CorsLayer`, honouring the CORS spec rule that
/// `allow_credentials=true` is incompatible with wildcard headers (browsers reject
/// the preflight). Logs a warning and falls back to default headers in that case.
fn apply_cors_headers_policy(
    layer: tower_http::cors::CorsLayer,
    cors: &CorsConfig,
) -> tower_http::cors::CorsLayer {
    use tower_http::cors::Any;

    let has_wildcard = cors.allowed_headers.iter().any(|h| h == "*");
    if has_wildcard && !cors.allow_credentials {
        return layer.allow_headers(Any);
    }
    if has_wildcard && cors.allow_credentials {
        tracing::warn!(
            "CORS: allow_credentials=true is incompatible with wildcard \
             headers per CORS spec. Falling back to default headers \
             (Content-Type, Authorization)."
        );
    }
    let headers: Vec<axum::http::HeaderName> = cors
        .allowed_headers
        .iter()
        .filter(|h| h.as_str() != "*")
        .filter_map(|h| h.parse().ok())
        .collect();
    if headers.is_empty() && cors.allow_credentials {
        layer.allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
    } else {
        layer.allow_headers(headers)
    }
}

// ============================================================================
// Helper: parse comma-separated API keys from env var
// ============================================================================

/// Parse `VELESDB_API_KEYS` env var (comma-separated) into a `Vec<String>`.
pub fn parse_api_keys_env() -> Option<Vec<String>> {
    let val = std::env::var("VELESDB_API_KEYS").ok()?;
    let keys: Vec<String> = val
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if keys.is_empty() {
        None
    } else {
        Some(keys)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
