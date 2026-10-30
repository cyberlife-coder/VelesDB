//! `[server] workers`, `max_body_size`, `cors_enabled`, `cors_origins`, the
//! `[logging]` section and the unread-key report: parsing, precedence and
//! validation. What the binary does with each value is pinned end to end by
//! `tests/server_settings_wiring_tests.rs`.

use super::*;
use crate::logging::{LogFormat, LogLevel};

fn merged(toml_content: &str, cli: CliOverrides) -> anyhow::Result<ServerConfig> {
    let file_cfg: FileConfig = toml::from_str(toml_content).expect("test: valid TOML");
    ServerConfig::merge(ServerConfig::default(), file_cfg, cli)
}

fn from_file(toml_content: &str) -> ServerConfig {
    merged(toml_content, CliOverrides::default()).expect("test: merge")
}

// ---------------------------------------------------------------------------
// Defaults: an unconfigured server behaves as it did before these keys
// were read.
// ---------------------------------------------------------------------------

#[test]
fn test_defaults_keep_the_behaviour_of_an_unconfigured_server() {
    let cfg = from_file("");
    assert_eq!(cfg.workers, 0, "0 = one worker per CPU, as #[tokio::main]");
    assert_eq!(cfg.max_body_size, 100 * 1024 * 1024);
    assert!(
        cfg.cors.enabled && !cfg.cors.is_active(),
        "same-origin only"
    );
    assert_eq!(cfg.logging.level, None);
    assert_eq!(cfg.logging.format, LogFormat::Text);
    assert_eq!(cfg.logging.file, None);
    assert!(cfg.ignored_entries.is_empty());
    cfg.validate().expect("test: defaults are valid");
}

#[test]
fn test_server_section_keys_are_read() {
    let cfg = from_file(
        r#"
[server]
workers = 8
max_body_size = 1048576
cors_enabled = false
cors_origins = ["https://app.example.com"]
"#,
    );
    assert_eq!(cfg.workers, 8);
    assert_eq!(cfg.max_body_size, 1_048_576);
    assert!(!cfg.cors.enabled);
    assert_eq!(cfg.cors.allowed_origins, vec!["https://app.example.com"]);
    assert!(cfg.ignored_entries.is_empty(), "{:?}", cfg.ignored_entries);
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

#[test]
fn test_cors_origins_and_allowed_origins_set_together_are_refused() {
    let err = merged(
        r#"
[server]
cors_origins = ["https://a.example.com"]

[cors]
allowed_origins = ["https://b.example.com"]
"#,
        CliOverrides::default(),
    )
    .expect_err("test: two names for one list must not race");
    assert!(err.to_string().contains("cors_origins"), "{err}");
}

#[test]
fn test_cors_origins_combine_with_the_rest_of_the_cors_section() {
    let cfg = from_file(
        r#"
[server]
cors_origins = ["https://app.example.com"]

[cors]
allowed_methods = ["GET"]
"#,
    );
    assert_eq!(cfg.cors.allowed_origins, vec!["https://app.example.com"]);
    assert_eq!(cfg.cors.allowed_methods, vec!["GET"]);
}

fn cors_with_origins(origins: &[&str]) -> CorsConfig {
    CorsConfig {
        allowed_origins: origins.iter().map(ToString::to_string).collect(),
        ..CorsConfig::default()
    }
}

#[test]
fn test_cors_validation_accepts_exact_origins_and_a_lone_wildcard() {
    for origins in [
        &["*"][..],
        &["https://app.example.com", "http://localhost:3000"][..],
        &[][..],
    ] {
        cors_with_origins(origins)
            .validate()
            .unwrap_or_else(|e| panic!("{origins:?}: {e}"));
    }
}

#[test]
fn test_cors_validation_refuses_what_could_never_match_or_would_allow_all() {
    for origins in [
        &["https://app.example.com", "*"][..],
        &["https://app.example.com/"][..],
        &["https://app.example.com/path"][..],
        &["app.example.com"][..],
        &["https://"][..],
    ] {
        assert!(
            cors_with_origins(origins).validate().is_err(),
            "{origins:?} must be refused"
        );
    }
}

#[test]
fn test_cors_validation_refuses_unparseable_methods_and_headers() {
    let bad_method = CorsConfig {
        allowed_methods: vec!["GE T".to_string()],
        ..CorsConfig::default()
    };
    assert!(bad_method.validate().is_err());
    let bad_header = CorsConfig {
        allowed_headers: vec!["Content Type".to_string()],
        ..CorsConfig::default()
    };
    assert!(bad_header.validate().is_err());
}

// ---------------------------------------------------------------------------
// workers / max_body_size validation
// ---------------------------------------------------------------------------

#[test]
fn test_workers_accepts_zero_for_auto_up_to_the_cap() {
    for workers in [0, 1, WORKERS_CAP] {
        let cfg = ServerConfig {
            workers,
            ..ServerConfig::default()
        };
        cfg.validate().expect("test: in range");
    }
    let cfg = ServerConfig {
        workers: WORKERS_CAP + 1,
        ..ServerConfig::default()
    };
    let err = cfg.validate().expect_err("test: above the cap");
    assert!(err.to_string().contains("server.workers"), "{err}");
}

#[test]
fn test_max_body_size_must_be_positive_and_at_most_one_gib() {
    for max_body_size in [1, MAX_BODY_SIZE_CAP] {
        let cfg = ServerConfig {
            max_body_size,
            ..ServerConfig::default()
        };
        cfg.validate().expect("test: in range");
    }
    for max_body_size in [0, MAX_BODY_SIZE_CAP + 1] {
        let cfg = ServerConfig {
            max_body_size,
            ..ServerConfig::default()
        };
        let err = cfg.validate().expect_err("test: out of range");
        assert!(err.to_string().contains("server.max_body_size"), "{err}");
    }
}

// ---------------------------------------------------------------------------
// [logging] and VELESDB_LOGGING_LEVEL
// ---------------------------------------------------------------------------

#[test]
fn test_logging_section_is_read() {
    let cfg = from_file(
        r#"
[logging]
level = "debug"
format = "json"
file = "/var/log/velesdb/velesdb.log"
"#,
    );
    assert_eq!(cfg.logging.level.map(LogLevel::as_str), Some("debug"));
    assert_eq!(cfg.logging.format, LogFormat::Json);
    assert_eq!(
        cfg.logging.file,
        Some(PathBuf::from("/var/log/velesdb/velesdb.log"))
    );
}

#[test]
fn test_empty_logging_file_means_stdout() {
    let cfg = from_file("[logging]\nfile = \"\"\n");
    assert_eq!(cfg.logging.file, None);
}

#[test]
fn test_velesdb_logging_level_overrides_the_file_level() {
    let cli = CliOverrides {
        log_level: Some("warn".to_string()),
        ..CliOverrides::default()
    };
    let cfg = merged("[logging]\nlevel = \"debug\"\n", cli).expect("test: merge");
    assert_eq!(cfg.logging.level.map(LogLevel::as_str), Some("warn"));
}

#[test]
fn test_invalid_logging_values_are_refused_naming_their_source() {
    let cases = [
        ("[logging]\nlevel = \"verbose\"\n", None, "logging.level"),
        ("[logging]\nformat = \"xml\"\n", None, "logging.format"),
        // Same spelling rule as velesdb-core, which reads this variable too.
        ("", Some("DEBUG"), "VELESDB_LOGGING_LEVEL"),
    ];
    for (toml_content, env, source) in cases {
        let cli = CliOverrides {
            log_level: env.map(str::to_string),
            ..CliOverrides::default()
        };
        let err = merged(toml_content, cli).expect_err("test: invalid value");
        assert!(err.to_string().contains(source), "{source}: {err}");
    }
}

// ---------------------------------------------------------------------------
// Keys nothing reads
// ---------------------------------------------------------------------------

#[test]
fn test_unread_keys_of_server_owned_sections_are_reported() {
    let cfg = from_file(
        r#"
[server]
port = 9090
worker = 4

[auth]
api_key = "k"

[tls]
certificate = "c.pem"

[cors]
allowed_origin = ["https://a.example.com"]

[logging]
colour = true

[search]
not_ours_to_report = 1
"#,
    );
    assert_eq!(
        cfg.ignored_entries,
        vec![
            "server.worker",
            "auth.api_key",
            "tls.certificate",
            "cors.allowed_origin",
            "logging.colour",
        ]
    );
    assert_eq!(
        cfg.port, 9090,
        "known keys still apply next to unknown ones"
    );
}

// ---------------------------------------------------------------------------
// "*" and the settings the permissive policy ignores
// ---------------------------------------------------------------------------

#[test]
fn test_wildcard_with_settings_it_would_ignore_is_refused() {
    for (setting, expected) in [
        ("allow_credentials = true", "allow_credentials"),
        ("allowed_methods = [\"GET\"]", "allowed_methods"),
        ("allowed_headers = [\"content-type\"]", "allowed_headers"),
        ("max_age_secs = 60", "max_age_secs"),
    ] {
        let cfg = from_file(&format!(
            "[server]\ncors_origins = [\"*\"]\n\n[cors]\n{setting}\n"
        ));
        let err = cfg.validate().expect_err("test: ignored setting");
        assert!(err.to_string().contains(expected), "{setting}: {err}");
    }
}

#[test]
fn test_settings_the_wildcard_ignores_are_fine_with_an_explicit_list() {
    let cfg = from_file(
        "[server]\ncors_origins = [\"https://app.example.com\"]\n\n\
         [cors]\nallow_credentials = true\nallowed_methods = [\"GET\"]\nmax_age_secs = 60\n",
    );
    cfg.validate()
        .expect("test: every setting applies to an explicit list");
}

// ---------------------------------------------------------------------------
// Top-level tables nothing reads
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_top_level_tables_are_reported() {
    let cfg = from_file(
        r#"
[sever]
port = 9090

[search]
default_mode = "fast"

[update_check]
enabled = false

[premium]
hot_reload = false

[wal_batch]
max_size = 1
"#,
    );
    // `[search]` is the engine's and a removed table such as `[wal_batch]` is
    // named by the engine's own removal warning; `[update_check]` is read
    // when the feature is built. Nothing in this build reads `[premium]`.
    let mut expected = vec!["[premium]", "[sever]"];
    if !cfg!(feature = "update-check") {
        expected.push("[update_check]");
    }
    assert_eq!(cfg.ignored_entries, expected);
}

// ---------------------------------------------------------------------------
// [server] allowed_hosts
// ---------------------------------------------------------------------------

#[test]
fn test_allowed_hosts_cli_or_env_beats_the_file() {
    let file = "[server]\nallowed_hosts = [\"file.example.com\"]\n";
    assert_eq!(from_file(file).allowed_hosts, vec!["file.example.com"]);
    let cli = CliOverrides {
        allowed_hosts: Some(vec!["env.example.com".to_string()]),
        ..CliOverrides::default()
    };
    let cfg = merged(file, cli).expect("test: merge");
    assert_eq!(cfg.allowed_hosts, vec!["env.example.com"]);
}

#[test]
fn test_allowed_hosts_entries_must_be_bare_hosts() {
    for good in [&["db.example.com", "10.0.0.5", "fe80::1"][..], &["*"][..]] {
        let cfg = ServerConfig {
            allowed_hosts: good.iter().map(ToString::to_string).collect(),
            ..ServerConfig::default()
        };
        cfg.validate().unwrap_or_else(|e| panic!("{good:?}: {e}"));
    }
    for bad in [
        &["https://db.example.com"][..],
        &["db.example.com/"][..],
        &[""][..],
        &["*", "db.example.com"][..],
    ] {
        let cfg = ServerConfig {
            allowed_hosts: bad.iter().map(ToString::to_string).collect(),
            ..ServerConfig::default()
        };
        assert!(cfg.validate().is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn test_a_wildcard_bind_without_allowed_hosts_is_flagged() {
    for host in ["0.0.0.0", "::"] {
        let cfg = ServerConfig {
            host: host.to_string(),
            ..ServerConfig::default()
        };
        assert!(
            cfg.answers_only_loopback_names_on_a_wildcard_bind(),
            "{host}"
        );
    }
    let listed = ServerConfig {
        host: "0.0.0.0".to_string(),
        allowed_hosts: vec!["velesdb".to_string()],
        ..ServerConfig::default()
    };
    assert!(!listed.answers_only_loopback_names_on_a_wildcard_bind());
    assert!(!ServerConfig::default().answers_only_loopback_names_on_a_wildcard_bind());
}
