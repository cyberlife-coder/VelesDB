//! Integration coverage for the engine `VelesConfig` wiring (issue #1549).
//!
//! Mirrors the server/CLI pattern from PR #1565: an explicit engine config
//! (given as a value or loaded engine-only from a TOML path) must reach
//! `Database::open_with_config` / `open_with_observer_and_config`, a missing
//! or invalid config path must fail fast with an actionable error (never a
//! silent fallback to defaults), and the no-config behaviour must stay
//! byte-for-byte identical to before.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;
use tauri_plugin_velesdb::{Builder, VelesDbState};
use velesdb_core::config::VelesConfig;

/// Engine config with `limits.max_collections = 1`, built through the same
/// engine-only parser the path loader uses.
fn one_collection_config() -> VelesConfig {
    VelesConfig::from_toml_engine_only("[limits]\nmax_collections = 1\n")
        .expect("test: valid engine-only config")
}

/// Proves a config is *enforced* by the opened database, not just stored:
/// the first collection fits under `max_collections = 1`, the second must be
/// refused by the engine.
fn assert_limit_of_one_enforced(state: &VelesDbState) {
    state
        .with_db(|db| {
            assert_eq!(db.config().limits.max_collections, 1);
            db.create_metadata_collection("first")?;
            Ok(())
        })
        .expect("test: first collection under the limit should succeed");

    let err = state
        .with_db(|db| {
            db.create_metadata_collection("second")?;
            Ok(())
        })
        .expect_err("test: second collection should be refused by the configured limit");
    assert!(
        err.to_string().contains("max_collections"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// (a) explicit config honoured
// ---------------------------------------------------------------------------

#[test]
fn test_state_open_honours_explicit_config() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let state = VelesDbState::new_with_config(dir.path().to_path_buf(), one_collection_config());

    // Act + Assert
    assert_limit_of_one_enforced(&state);
}

#[test]
fn test_state_open_honours_config_alongside_observer() {
    #[derive(Default)]
    struct CountingObserver {
        created: AtomicUsize,
    }
    impl velesdb_core::DatabaseObserver for CountingObserver {
        fn on_collection_created(
            &self,
            _name: &str,
            _kind: &velesdb_core::collection::CollectionType,
        ) {
            self.created.fetch_add(1, Ordering::SeqCst);
        }
    }

    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let observer = Arc::new(CountingObserver::default());
    let state = VelesDbState::new_with_observer_and_config(
        dir.path().to_path_buf(),
        observer.clone(),
        one_collection_config(),
    );

    // Act + Assert - the config is enforced...
    assert_limit_of_one_enforced(&state);

    // ...and the observer still received the successful create.
    assert_eq!(observer.created.load(Ordering::SeqCst), 1);
}

/// End-to-end through the plugin builder: a config passed to
/// `Builder::with_config` must reach the managed state and the opened engine.
#[test]
fn test_plugin_builder_with_config_reaches_managed_state() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let app = mock_builder()
        .plugin(
            Builder::new(dir.path())
                .with_config(one_collection_config())
                .build(),
        )
        .build(mock_context(noop_assets()))
        .expect("test: build mock app with plugin");

    // Act + Assert
    let state = app.state::<VelesDbState>();
    assert_limit_of_one_enforced(&state);
}

// ---------------------------------------------------------------------------
// (b) invalid / missing config path -> fail-fast, actionable, typed source
// ---------------------------------------------------------------------------

#[test]
fn test_with_config_path_missing_file_fails_fast() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let missing = dir.path().join("does-not-exist.toml");

    // Act
    let err = Builder::new(dir.path())
        .with_config_path(&missing)
        .expect_err("test: a missing explicit config path must be an immediate error");

    // Assert - actionable: the message names the offending path.
    assert!(
        err.to_string().contains("does-not-exist.toml"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_with_config_path_invalid_value_fails_fast_with_typed_source() {
    // Arrange - parses fine, fails engine validation (max_collections = 0).
    let dir = tempfile::tempdir().expect("test: temp dir");
    let config_path = dir.path().join("velesdb.toml");
    std::fs::write(&config_path, "[limits]\nmax_collections = 0\n").expect("test: write config");

    // Act
    let err = Builder::new(dir.path())
        .with_config_path(&config_path)
        .expect_err("test: an invalid engine value must be an immediate error");

    // Assert - the typed core ConfigError is preserved as the error source.
    let source = std::error::Error::source(&err).expect("test: error must expose a source");
    assert!(
        source
            .downcast_ref::<velesdb_core::config::ConfigError>()
            .is_some(),
        "source must be velesdb_core::config::ConfigError, got: {source}"
    );
}

/// Engine-only semantics: a TOML shared with `velesdb-server` may carry a
/// `[server]` table (its HTTP transport) whose values would be rejected by
/// `VelesConfig`'s own unrelated `server` validation. The plugin loader must
/// ignore non-engine sections instead of failing on them.
#[test]
fn test_with_config_path_ignores_non_engine_sections() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let config_path = dir.path().join("velesdb.toml");
    std::fs::write(
        &config_path,
        "[server]\nport = 443\n\n[limits]\nmax_collections = 1\n",
    )
    .expect("test: write config");

    // Act
    let builder = Builder::new(dir.path())
        .with_config_path(&config_path)
        .expect("test: shell-owned [server] section must not block the plugin");

    // Assert - the engine section was loaded...
    assert_eq!(
        builder
            .config()
            .expect("test: config must be loaded")
            .limits
            .max_collections,
        1
    );

    // ...and it is enforced end-to-end through the built plugin.
    let app = mock_builder()
        .plugin(builder.build())
        .build(mock_context(noop_assets()))
        .expect("test: build mock app with plugin");
    let state = app.state::<VelesDbState>();
    assert_limit_of_one_enforced(&state);
}

// ---------------------------------------------------------------------------
// (c) no config -> behaviour unchanged (core defaults)
// ---------------------------------------------------------------------------

#[test]
fn test_no_config_opens_with_core_defaults() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let state = VelesDbState::new(dir.path().to_path_buf());

    // Act + Assert
    state
        .with_db(|db| {
            assert_eq!(
                db.config().limits.max_collections,
                velesdb_core::config::LimitsConfig::default().max_collections
            );
            Ok(())
        })
        .expect("test: open without config");
}

#[test]
fn test_builder_without_config_opens_with_core_defaults() {
    // Arrange
    let dir = tempfile::tempdir().expect("test: temp dir");
    let app = mock_builder()
        .plugin(Builder::new(dir.path()).build())
        .build(mock_context(noop_assets()))
        .expect("test: build mock app with plugin");

    // Act + Assert
    let state = app.state::<VelesDbState>();
    state
        .with_db(|db| {
            assert_eq!(
                db.config().limits.max_collections,
                velesdb_core::config::LimitsConfig::default().max_collections
            );
            Ok(())
        })
        .expect("test: open without config");
}
