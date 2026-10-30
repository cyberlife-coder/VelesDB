//! Server configuration module.
//!
//! Loads configuration from multiple sources with priority:
//! CLI flags > environment variables > velesdb.toml > defaults.
//!
//! The file sections this module owns are `[server]`, `[auth]`, `[tls]`,
//! `[cors]` and `[logging]`. A key in one of them that no field below names,
//! or a top-level table that neither this module nor the engine nor the
//! update check reads, is not dropped in silence:
//! [`ServerConfig::ignored_entries`] lists it and the binary warns about it
//! at startup.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use crate::cors::{apply_cors, build_cors_layer, CorsConfig};
use crate::cors::{resolve_cors, CorsSection};
pub use crate::host_guard::apply_browser_policy;
use crate::logging::{LoggingConfig, LoggingSection};

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
/// `[storage]`/`[limits]`/`[quantization]`) and silently
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
                // No file, but the `VELESDB_*` engine variables still apply.
                return velesdb_core::config::VelesConfig::load_engine_only_from_env()
                    .map_err(|e| anyhow::anyhow!("failed to load VelesDB core config: {e}"));
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

/// The keys of a server-owned section that none of its fields name. Captured
/// with `#[serde(flatten)]`, so the set of known keys is the struct itself —
/// there is no second list to keep in step with it.
pub(crate) type UnknownKeys = BTreeMap<String, toml::Value>;

/// Root structure for `velesdb.toml`. Every other top-level table lands in
/// `other`; [`FileConfig::unknown_entries`] reports those nobody reads.
#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    server: Option<ServerSection>,
    auth: Option<AuthSection>,
    tls: Option<TlsSection>,
    cors: Option<CorsSection>,
    logging: Option<LoggingSection>,
    #[serde(flatten)]
    other: UnknownKeys,
}

/// Whether a top-level table outside this module's five is read by someone
/// else: the engine (its sections, and removed tables it warns about
/// itself), or the update check when this build has it.
fn read_elsewhere(table: &str) -> bool {
    velesdb_core::config::VelesConfig::is_engine_table(table)
        || (cfg!(feature = "update-check") && table == "update_check")
}

#[derive(Debug, Deserialize, Default)]
struct ServerSection {
    host: Option<String>,
    port: Option<u16>,
    data_dir: Option<String>,
    shutdown_timeout_secs: Option<u64>,
    rate_limit: Option<u32>,
    workers: Option<usize>,
    max_body_size: Option<usize>,
    cors_enabled: Option<bool>,
    cors_origins: Option<Vec<String>>,
    allowed_hosts: Option<Vec<String>>,
    #[serde(flatten)]
    unknown: UnknownKeys,
}

#[derive(Debug, Deserialize, Default)]
struct AuthSection {
    api_keys: Option<Vec<String>>,
    #[serde(flatten)]
    unknown: UnknownKeys,
}

#[derive(Debug, Deserialize, Default)]
struct TlsSection {
    cert: Option<String>,
    key: Option<String>,
    #[serde(flatten)]
    unknown: UnknownKeys,
}

impl FileConfig {
    /// Every key of a server-owned section that no field reads, as
    /// `section.key`, sorted within each section, then every top-level table
    /// nobody reads, as `[table]`.
    fn unknown_entries(&self) -> Vec<String> {
        let mut entries = self.unknown_section_keys();
        entries.extend(
            self.other
                .iter()
                .filter(|(name, _)| !read_elsewhere(name))
                .map(|(name, value)| {
                    if value.is_table() {
                        format!("[{name}]")
                    } else {
                        name.clone()
                    }
                }),
        );
        entries
    }

    fn unknown_section_keys(&self) -> Vec<String> {
        let sections: [(&str, Option<&UnknownKeys>); 5] = [
            ("server", self.server.as_ref().map(|s| &s.unknown)),
            ("auth", self.auth.as_ref().map(|s| &s.unknown)),
            ("tls", self.tls.as_ref().map(|s| &s.unknown)),
            ("cors", self.cors.as_ref().map(|s| &s.unknown)),
            ("logging", self.logging.as_ref().map(|s| &s.unknown)),
        ];
        sections
            .into_iter()
            .filter_map(|(name, keys)| Some((name, keys?)))
            .flat_map(|(name, keys)| keys.keys().map(move |key| format!("{name}.{key}")))
            .collect()
    }
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

    /// Both paths or neither, and each one that is set must exist.
    fn validate(&self) -> anyhow::Result<()> {
        match (&self.cert, &self.key) {
            (Some(_), None) => anyhow::bail!("tls_cert is set but tls_key is missing"),
            (None, Some(_)) => anyhow::bail!("tls_key is set but tls_cert is missing"),
            (Some(cert), Some(key)) => {
                if !Path::new(cert).exists() {
                    anyhow::bail!("TLS cert file not found: {cert}");
                }
                if !Path::new(key).exists() {
                    anyhow::bail!("TLS key file not found: {key}");
                }
                Ok(())
            }
            (None, None) => Ok(()),
        }
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
    /// Tokio runtime worker threads (`[server] workers`). `0`, the default,
    /// lets Tokio start one per CPU, as `#[tokio::main]` always did.
    pub workers: usize,
    /// The largest request body the server accepts, in bytes
    /// (`[server] max_body_size`). The point-upload routes accept up to this;
    /// every other route is capped at the smaller of this and
    /// [`crate::routes::JSON_BODY_LIMIT`]. See
    /// [`crate::routes::api_routes_with_body_limit`].
    pub max_body_size: usize,
    /// The `[logging]` section, with `VELESDB_LOGGING_LEVEL` applied.
    pub logging: LoggingConfig,
    /// Host names the server answers to besides the loopback names and its
    /// bind host (`[server] allowed_hosts`, `--allowed-hosts`,
    /// `VELESDB_ALLOWED_HOSTS`); `["*"]` turns the `Host` check off. See
    /// `host_guard`.
    pub allowed_hosts: Vec<String>,
    /// Keys of the server-owned sections that nothing reads, as
    /// `section.key` — a typo, or a setting this build does not have. The
    /// binary warns about them at startup instead of dropping them silently.
    pub ignored_entries: Vec<String>,
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
            tls: TlsConfig::default(),
            shutdown_timeout_secs: 30,
            rate_limit: DEFAULT_RATE_LIMIT,
            cors: CorsConfig::default(),
            workers: 0,
            max_body_size: crate::routes::DEFAULT_MAX_BODY_SIZE,
            logging: LoggingConfig::default(),
            allowed_hosts: Vec::new(),
            ignored_entries: Vec::new(),
        }
    }
}

/// Ceiling for `[server] workers`: the engine's own `server.workers` rule,
/// imported rather than copied.
pub use velesdb_core::config::WORKERS_CAP;

/// Ceiling for `[server] max_body_size`: 1 GiB. A point-upload body is
/// buffered whole before it is parsed, so this bounds one request's memory.
pub const MAX_BODY_SIZE_CAP: usize = 1024 * 1024 * 1024;

// ============================================================================
// Loading logic
// ============================================================================

impl ServerConfig {
    /// Load configuration with priority: CLI > env > TOML file > defaults.
    ///
    /// `cli` contains values from clap (which merges CLI flags + env vars).
    /// `cli_sources` indicates which fields were explicitly set via CLI/env
    /// (as opposed to falling back to clap defaults).
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or parsed, when a
    /// `[logging]` value (or `VELESDB_LOGGING_LEVEL`) is not one the server
    /// knows, or when both `[server] cors_origins` and
    /// `[cors] allowed_origins` are set.
    pub fn load(cli: CliOverrides) -> anyhow::Result<Self> {
        let defaults = Self::default();
        let file_cfg = load_toml_file(&cli.config_path)?;
        Self::merge(defaults, file_cfg, cli)
    }

    fn merge(defaults: Self, file: FileConfig, cli: CliOverrides) -> anyhow::Result<Self> {
        let ignored_entries = file.unknown_entries();
        let logging =
            LoggingConfig::resolve(&file.logging.unwrap_or_default(), cli.log_level.as_deref())?;
        let mut server = file.server.unwrap_or_default();
        let cors = resolve_cors(
            &defaults.cors,
            file.cors.unwrap_or_default(),
            server.cors_enabled,
            server.cors_origins.take(),
        )?;
        let workers = server.workers.unwrap_or(defaults.workers);
        let max_body_size = server.max_body_size.unwrap_or(defaults.max_body_size);
        let transport = Self::merge_transport(
            defaults,
            server,
            file.auth.unwrap_or_default(),
            file.tls.unwrap_or_default(),
            cli,
        );
        Ok(Self {
            cors,
            workers,
            max_body_size,
            logging,
            ignored_entries,
            ..transport
        })
    }

    /// The settings that have a CLI flag or an environment variable, layered
    /// CLI/env > TOML > default. The rest of the returned value is
    /// `defaults`'.
    fn merge_transport(
        defaults: Self,
        server: ServerSection,
        auth: AuthSection,
        tls: TlsSection,
        cli: CliOverrides,
    ) -> Self {
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

        // Layer: CLI/env over TOML (only override when explicitly set)
        Self {
            host: cli.host.unwrap_or(host),
            port: cli.port.unwrap_or(port),
            data_dir: cli.data_dir.unwrap_or(data_dir),
            api_keys: cli.api_keys.unwrap_or(api_keys),
            tls: TlsConfig {
                cert: cli.tls_cert.or(tls.cert),
                key: cli.tls_key.or(tls.key),
            },
            shutdown_timeout_secs,
            rate_limit: cli.rate_limit.unwrap_or(rate_limit),
            allowed_hosts: cli
                .allowed_hosts
                .or(server.allowed_hosts)
                .unwrap_or(defaults.allowed_hosts),
            ..defaults
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
        if self.workers > WORKERS_CAP {
            anyhow::bail!(
                "server.workers: value {} is out of range: 0 (one per CPU) or 1..={WORKERS_CAP}",
                self.workers
            );
        }
        if self.max_body_size == 0 || self.max_body_size > MAX_BODY_SIZE_CAP {
            anyhow::bail!(
                "server.max_body_size: value {} is out of range [1, {MAX_BODY_SIZE_CAP}] (bytes)",
                self.max_body_size
            );
        }
        self.tls.validate()?;
        validate_allowed_hosts(&self.allowed_hosts)?;
        self.cors.validate()
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

    /// Returns `true` when the server is bound to a wildcard address with no
    /// `allowed_hosts`: reachable from everywhere, yet answering only the
    /// loopback names — worth saying at startup, since a client using any
    /// other name gets `421`.
    pub fn answers_only_loopback_names_on_a_wildcard_bind(&self) -> bool {
        self.allowed_hosts.is_empty() && crate::host_guard::binds_wildcard(&self.host)
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

/// Each `allowed_hosts` entry is a bare host name or address — what a
/// `Host` header carries minus the port — or `"*"` on its own.
fn validate_allowed_hosts(hosts: &[String]) -> anyhow::Result<()> {
    if hosts.iter().any(|h| h == "*") && hosts.len() > 1 {
        anyhow::bail!(
            "server.allowed_hosts {hosts:?}: \"*\" turns the Host check off and cannot \
             be combined with names"
        );
    }
    for host in hosts.iter().filter(|h| *h != "*") {
        let bare = !host.is_empty()
            && !host.contains("://")
            && !host.contains('/')
            && !host.chars().any(char::is_whitespace);
        if !bare {
            anyhow::bail!(
                "server.allowed_hosts: '{host}' is not a host name or address \
                 (no scheme, no path, e.g. \"db.example.com\")"
            );
        }
    }
    Ok(())
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
    /// `VELESDB_LOGGING_LEVEL` (see [`crate::logging::level_from_env`]).
    pub log_level: Option<String>,
    /// `--allowed-hosts` / `VELESDB_ALLOWED_HOSTS` (comma-separated).
    pub allowed_hosts: Option<Vec<String>>,
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
// Update check
// ============================================================================

/// What the server's update check runs with: the `[update_check]` section of
/// the config file the server was started with (`--config` /
/// `VELESDB_CONFIG`, else `./velesdb.toml`), and a context that names the
/// binary `server` and lets the install id fall back to `data_dir`.
#[cfg(feature = "update-check")]
#[must_use]
pub fn update_check_plan(
    config_path: Option<&Path>,
    data_dir: &str,
) -> (
    velesdb_core::UpdateCheckConfig,
    velesdb_core::UpdateCheckContext,
) {
    (
        velesdb_core::UpdateCheckConfig::from_config_file(config_path),
        velesdb_core::UpdateCheckContext::new("server", "core").with_data_dir(data_dir),
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "config_server_settings_tests.rs"]
mod server_settings_tests;
