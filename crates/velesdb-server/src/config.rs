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
}

/// Default burst budget for rate limiting (requests per second per IP).
const DEFAULT_RATE_LIMIT: u32 = 100;

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
}
