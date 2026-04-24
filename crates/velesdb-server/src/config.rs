//! Server configuration module.
//!
//! Loads configuration from multiple sources with priority:
//! CLI flags > environment variables > velesdb.toml > defaults.

use serde::Deserialize;
use std::path::{Path, PathBuf};

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

/// Final resolved server configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub data_dir: String,
    pub api_keys: Vec<String>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
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
            tls_cert: None,
            tls_key: None,
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
        let tls_cert = tls.cert.or(defaults.tls_cert);
        let tls_key = tls.key.or(defaults.tls_key);
        let cors = resolve_cors(defaults.cors, cors_section);

        // Layer: CLI/env over TOML (only override when explicitly set)
        let host = cli.host.unwrap_or(host);
        let port = cli.port.unwrap_or(port);
        let data_dir = cli.data_dir.unwrap_or(data_dir);
        let api_keys = cli.api_keys.unwrap_or(api_keys);
        let tls_cert = cli.tls_cert.or(tls_cert);
        let tls_key = cli.tls_key.or(tls_key);
        let rate_limit = cli.rate_limit.unwrap_or(rate_limit);

        Self {
            host,
            port,
            data_dir,
            api_keys,
            tls_cert,
            tls_key,
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
        match (&self.tls_cert, &self.tls_key) {
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
        self.tls_cert.is_some() && self.tls_key.is_some()
    }

    /// Returns `true` when rate limiting is enabled (rate_limit > 0).
    pub fn rate_limit_enabled(&self) -> bool {
        self.rate_limit > 0
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
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_defaults() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.data_dir, "./velesdb_data");
        assert!(cfg.api_keys.is_empty());
        assert!(cfg.tls_cert.is_none());
        assert!(cfg.tls_key.is_none());
        assert_eq!(cfg.shutdown_timeout_secs, 30);
        assert_eq!(cfg.rate_limit, 100);
        assert!(!cfg.auth_enabled());
        assert!(!cfg.tls_enabled());
        assert!(cfg.rate_limit_enabled());
        assert!(cfg.cors.is_permissive());
    }

    #[test]
    fn test_toml_overrides_defaults() {
        let toml_content = r#"
[server]
host = "0.0.0.0"
port = 9090
data_dir = "/var/velesdb"
shutdown_timeout_secs = 60

[auth]
api_keys = ["key-alpha", "key-beta"]

[tls]
cert = "/etc/ssl/cert.pem"
key = "/etc/ssl/key.pem"
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.data_dir, "/var/velesdb");
        assert_eq!(cfg.shutdown_timeout_secs, 60);
        assert_eq!(cfg.api_keys, vec!["key-alpha", "key-beta"]);
        assert_eq!(cfg.tls_cert.as_deref(), Some("/etc/ssl/cert.pem"));
        assert_eq!(cfg.tls_key.as_deref(), Some("/etc/ssl/key.pem"));
        assert!(cfg.auth_enabled());
        assert!(cfg.tls_enabled());
    }

    #[test]
    fn test_cli_overrides_toml() {
        let toml_content = r#"
[server]
host = "0.0.0.0"
port = 9090
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides {
            port: Some(3000),
            host: Some("10.0.0.1".to_string()),
            ..Default::default()
        };
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        // CLI wins over TOML
        assert_eq!(cfg.host, "10.0.0.1");
        assert_eq!(cfg.port, 3000);
        // TOML didn't set data_dir, so default applies
        assert_eq!(cfg.data_dir, "./velesdb_data");
    }

    #[test]
    fn test_partial_toml_uses_defaults_for_missing() {
        let toml_content = r#"
[server]
port = 4000
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.port, 4000);
        assert_eq!(cfg.host, "127.0.0.1"); // default
        assert_eq!(cfg.data_dir, "./velesdb_data"); // default
    }

    #[test]
    fn test_empty_toml_uses_all_defaults() {
        let file_cfg: FileConfig = toml::from_str("").unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg, ServerConfig::default());
    }

    #[test]
    fn test_validate_port_zero_rejected() {
        let cfg = ServerConfig {
            port: 0,
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn test_validate_empty_data_dir_rejected() {
        let cfg = ServerConfig {
            data_dir: String::new(),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("data_dir"));
    }

    #[test]
    fn test_validate_tls_cert_without_key() {
        let cfg = ServerConfig {
            tls_cert: Some("/tmp/cert.pem".to_string()),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("tls_key is missing"));
    }

    #[test]
    fn test_validate_tls_key_without_cert() {
        let cfg = ServerConfig {
            tls_key: Some("/tmp/key.pem".to_string()),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("tls_cert is missing"));
    }

    #[test]
    fn test_validate_tls_missing_cert_file() {
        let cfg = ServerConfig {
            tls_cert: Some("/nonexistent/cert.pem".to_string()),
            tls_key: Some("/nonexistent/key.pem".to_string()),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("cert file not found"));
    }

    #[test]
    fn test_validate_tls_valid_files() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        std::fs::File::create(&cert_path)
            .unwrap()
            .write_all(b"cert")
            .unwrap();
        std::fs::File::create(&key_path)
            .unwrap()
            .write_all(b"key")
            .unwrap();

        let cfg = ServerConfig {
            tls_cert: Some(cert_path.to_string_lossy().to_string()),
            tls_key: Some(key_path.to_string_lossy().to_string()),
            ..ServerConfig::default()
        };
        cfg.validate().expect("valid TLS config should pass");
    }

    #[test]
    fn test_parse_api_keys_env() {
        // Simulate by directly testing the parsing logic
        let input = "key1, key2 , key3";
        let keys: Vec<String> = input
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(keys, vec!["key1", "key2", "key3"]);
    }

    #[test]
    fn test_load_toml_file_not_found_explicit_path() {
        let result = load_toml_file(&Some(PathBuf::from("/nonexistent/velesdb.toml")));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("config file not found"));
    }

    #[test]
    fn test_load_toml_file_no_default_returns_empty() {
        // When no explicit path and no velesdb.toml in cwd, returns defaults
        let result = load_toml_file(&None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_full_priority_chain() {
        // Scenario: default=8080, TOML=9090, CLI=3000 → expect 3000
        let toml_content = r#"
[server]
port = 9090
host = "0.0.0.0"
data_dir = "/toml/data"
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides {
            port: Some(3000),
            // host not set in CLI → TOML should win
            ..Default::default()
        };
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.port, 3000); // CLI wins
        assert_eq!(cfg.host, "0.0.0.0"); // TOML wins (no CLI override)
        assert_eq!(cfg.data_dir, "/toml/data"); // TOML wins (no CLI override)
    }

    #[test]
    fn test_rate_limit_from_toml() {
        let toml_content = r#"
[server]
rate_limit = 50
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.rate_limit, 50);
        assert!(cfg.rate_limit_enabled());
    }

    #[test]
    fn test_rate_limit_disabled_via_toml() {
        let toml_content = r#"
[server]
rate_limit = 0
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.rate_limit, 0);
        assert!(!cfg.rate_limit_enabled());
    }

    #[test]
    fn test_rate_limit_cli_overrides_toml() {
        let toml_content = r#"
[server]
rate_limit = 50
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides {
            rate_limit: Some(200),
            ..Default::default()
        };
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.rate_limit, 200);
    }

    #[test]
    fn test_rate_limit_cli_disables() {
        let file_cfg = FileConfig::default();
        let cli = CliOverrides {
            rate_limit: Some(0),
            ..Default::default()
        };
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert_eq!(cfg.rate_limit, 0);
        assert!(!cfg.rate_limit_enabled());
    }

    // ====================================================================
    // CORS configuration tests
    // ====================================================================

    #[test]
    fn test_cors_default_is_permissive() {
        let cors = CorsConfig::default();
        assert!(cors.is_permissive());
        assert_eq!(cors.allowed_origins, vec!["*"]);
        assert_eq!(cors.allowed_headers, vec!["*"]);
        assert!(!cors.allow_credentials);
        assert_eq!(cors.max_age_secs, 3600);
    }

    #[test]
    fn test_cors_specific_origins_not_permissive() {
        let cors = CorsConfig {
            allowed_origins: vec![
                "https://app.example.com".to_string(),
                "https://admin.example.com".to_string(),
            ],
            ..CorsConfig::default()
        };
        assert!(!cors.is_permissive());
        assert_eq!(cors.allowed_origins.len(), 2);
    }

    #[test]
    fn test_cors_from_toml_specific_origins() {
        let toml_content = r#"
[cors]
allowed_origins = ["https://app.example.com", "https://admin.example.com"]
allowed_methods = ["GET", "POST"]
allowed_headers = ["Content-Type", "Authorization"]
allow_credentials = true
max_age_secs = 7200
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert!(!cfg.cors.is_permissive());
        assert_eq!(
            cfg.cors.allowed_origins,
            vec!["https://app.example.com", "https://admin.example.com"]
        );
        assert_eq!(cfg.cors.allowed_methods, vec!["GET", "POST"]);
        assert_eq!(
            cfg.cors.allowed_headers,
            vec!["Content-Type", "Authorization"]
        );
        assert!(cfg.cors.allow_credentials);
        assert_eq!(cfg.cors.max_age_secs, 7200);
    }

    #[test]
    fn test_cors_from_toml_partial_uses_defaults() {
        let toml_content = r#"
[cors]
allowed_origins = ["https://myapp.com"]
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert!(!cfg.cors.is_permissive());
        assert_eq!(cfg.cors.allowed_origins, vec!["https://myapp.com"]);
        // Other fields use defaults
        assert_eq!(cfg.cors.allowed_headers, vec!["*"]);
        assert!(!cfg.cors.allow_credentials);
        assert_eq!(cfg.cors.max_age_secs, 3600);
        assert_eq!(cfg.cors.allowed_methods.len(), 6); // default methods
    }

    #[test]
    fn test_cors_absent_from_toml_uses_permissive_default() {
        let toml_content = r#"
[server]
port = 9090
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert!(cfg.cors.is_permissive());
        assert_eq!(cfg.cors, CorsConfig::default());
    }

    #[test]
    fn test_cors_empty_section_uses_defaults() {
        let toml_content = r#"
[cors]
"#;
        let file_cfg: FileConfig = toml::from_str(toml_content).unwrap();
        let cli = CliOverrides::default();
        let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

        assert!(cfg.cors.is_permissive());
    }

    #[test]
    fn test_build_cors_layer_permissive() {
        let cors = CorsConfig::default();
        // Should not panic — produces a valid CorsLayer
        let _layer = build_cors_layer(&cors);
    }

    #[test]
    fn test_build_cors_layer_specific_origins() {
        let cors = CorsConfig {
            allowed_origins: vec![
                "https://app.example.com".to_string(),
                "http://localhost:3000".to_string(),
            ],
            allowed_methods: vec!["GET".to_string(), "POST".to_string()],
            allowed_headers: vec!["Content-Type".to_string(), "Authorization".to_string()],
            allow_credentials: true,
            max_age_secs: 600,
        };
        // Should not panic — produces a valid CorsLayer
        let _layer = build_cors_layer(&cors);
    }

    #[test]
    fn test_build_cors_layer_wildcard_headers() {
        let cors = CorsConfig {
            allowed_origins: vec!["https://myapp.com".to_string()],
            allowed_headers: vec!["*".to_string()],
            ..CorsConfig::default()
        };
        let _layer = build_cors_layer(&cors);
    }

    #[test]
    fn test_build_cors_layer_invalid_origin_skipped() {
        let cors = CorsConfig {
            allowed_origins: vec![
                "https://valid.com".to_string(),
                "not a valid \x00 origin".to_string(),
            ],
            ..CorsConfig::default()
        };
        // Invalid origins are silently filtered via filter_map
        let _layer = build_cors_layer(&cors);
    }

    #[test]
    fn test_server_config_default_includes_cors() {
        let cfg = ServerConfig::default();
        assert!(cfg.cors.is_permissive());
    }
}
