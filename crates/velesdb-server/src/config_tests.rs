use super::*;
use std::io::Write;

// ====================================================================
// `load_core_config` (issue #1549 — server `--config` wiring)
// ====================================================================

#[test]
fn test_load_core_config_none_returns_defaults_when_no_default_file() {
    // No explicit path, and no `velesdb.toml` in cwd (the test binary's
    // cwd is the crate root, which has no such file) — core defaults.
    let config = load_core_config(&None).expect("test: default core config");
    assert_eq!(
        config.limits.max_collections,
        velesdb_core::config::LimitsConfig::default().max_collections
    );
}

#[test]
fn test_load_core_config_explicit_missing_path_fails_fast() {
    let missing = PathBuf::from("/nonexistent/velesdb-issue-1549-server.toml");
    let err = load_core_config(&Some(missing))
        .expect_err("test: an explicit but missing config path must error, not silently default");
    assert!(
        err.to_string().contains("config file not found"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_load_core_config_explicit_invalid_value_fails_fast_typed() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let config_path = dir.path().join("velesdb.toml");
    // max_collections = 0 is out of range (validate_limits requires >= 1).
    std::fs::write(&config_path, "[limits]\nmax_collections = 0\n").expect("test: write config");

    let err = load_core_config(&Some(config_path))
        .expect_err("test: invalid value must fail fast with a typed ConfigError");
    assert!(
        err.to_string().contains("limits.max_collections"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_load_core_config_explicit_valid_path_applies_sections() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let config_path = dir.path().join("velesdb.toml");
    std::fs::write(
        &config_path,
        "[limits]\nmax_collections = 3\n\n[hnsw]\nm = 24\n",
    )
    .expect("test: write config");

    let config = load_core_config(&Some(config_path)).expect("test: load valid config");
    assert_eq!(config.limits.max_collections, 3);
    assert_eq!(config.hnsw.m, Some(24));
}

/// Regression test (Fable review finding on issue #1549): a legitimate
/// low HTTP bind port in this crate's own `[server]` section used to
/// also get parsed into `VelesConfig`'s own (unrelated) `server.port`
/// field and rejected by its `>= 1024` validation rule — a spurious
/// startup failure for a config file that was otherwise entirely
/// valid. `load_core_config` must ignore `[server]` (and any other
/// non-engine section) entirely, while still applying the real engine
/// sections from the same file.
#[test]
fn test_load_core_config_ignores_shell_owned_server_section_low_port() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let config_path = dir.path().join("velesdb.toml");
    std::fs::write(
        &config_path,
        "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
    )
    .expect("test: write config");

    let config = load_core_config(&Some(config_path))
        .expect("a shell-owned [server] port=443 must not fail core config loading");
    assert_eq!(config.limits.max_collections, 5);
}

#[test]
fn test_binds_publicly() {
    let mut cfg = ServerConfig::default();
    // Loopback hosts are private — including case variants, brackets,
    // whitespace, 127.0.0.0/8, and the IPv4-mapped IPv6 loopback.
    for host in [
        "127.0.0.1",
        "::1",
        "[::1]",
        "localhost",
        "LOCALHOST",
        " localhost ",
        "127.0.0.5",
        "::ffff:127.0.0.1",
    ] {
        cfg.host = host.to_string();
        assert!(!cfg.binds_publicly(), "{host} should be private");
    }
    // Wildcard and routable addresses are public.
    for host in ["0.0.0.0", "::", "192.168.1.10", "10.0.0.1", ""] {
        cfg.host = host.to_string();
        assert!(cfg.binds_publicly(), "{host} should be public");
    }
}

#[test]
fn test_defaults() {
    let cfg = ServerConfig::default();
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.data_dir, "./velesdb_data");
    assert!(cfg.api_keys.is_empty());
    assert!(cfg.tls.cert.is_none());
    assert!(cfg.tls.key.is_none());
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
    let cli = CliOverrides::default();
    let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

    assert_eq!(cfg.host, "0.0.0.0");
    assert_eq!(cfg.port, 9090);
    assert_eq!(cfg.data_dir, "/var/velesdb");
    assert_eq!(cfg.shutdown_timeout_secs, 60);
    assert_eq!(cfg.api_keys, vec!["key-alpha", "key-beta"]);
    assert_eq!(cfg.tls.cert.as_deref(), Some("/etc/ssl/cert.pem"));
    assert_eq!(cfg.tls.key.as_deref(), Some("/etc/ssl/key.pem"));
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
    let cli = CliOverrides::default();
    let cfg = ServerConfig::merge(ServerConfig::default(), file_cfg, cli);

    assert_eq!(cfg.port, 4000);
    assert_eq!(cfg.host, "127.0.0.1"); // default
    assert_eq!(cfg.data_dir, "./velesdb_data"); // default
}

#[test]
fn test_empty_toml_uses_all_defaults() {
    let file_cfg: FileConfig = toml::from_str("").expect("test: empty TOML parses to default");
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
        tls: TlsConfig {
            cert: Some("/tmp/cert.pem".to_string()),
            key: None,
        },
        ..ServerConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("tls_key is missing"));
}

#[test]
fn test_validate_tls_key_without_cert() {
    let cfg = ServerConfig {
        tls: TlsConfig {
            cert: None,
            key: Some("/tmp/key.pem".to_string()),
        },
        ..ServerConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("tls_cert is missing"));
}

#[test]
fn test_validate_tls_missing_cert_file() {
    let cfg = ServerConfig {
        tls: TlsConfig {
            cert: Some("/nonexistent/cert.pem".to_string()),
            key: Some("/nonexistent/key.pem".to_string()),
        },
        ..ServerConfig::default()
    };
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("cert file not found"));
}

#[test]
fn test_validate_tls_valid_files() {
    let dir = tempfile::tempdir().expect("test: create temp dir");
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::File::create(&cert_path)
        .expect("test: create cert file")
        .write_all(b"cert")
        .expect("test: write cert content");
    std::fs::File::create(&key_path)
        .expect("test: create key file")
        .write_all(b"key")
        .expect("test: write key content");

    let cfg = ServerConfig {
        tls: TlsConfig {
            cert: Some(cert_path.to_string_lossy().to_string()),
            key: Some(key_path.to_string_lossy().to_string()),
        },
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid FileConfig TOML");
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
