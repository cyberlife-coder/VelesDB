//! Tests for config module

#[cfg(test)]
mod tests {
    use crate::config::*;

    // ========================================================================
    // SearchMode tests
    // ========================================================================

    #[test]
    fn test_search_mode_ef_search_values() {
        // Arrange & Act & Assert
        assert_eq!(SearchMode::Fast.ef_search(), 96);
        assert_eq!(SearchMode::Balanced.ef_search(), 160);
        assert_eq!(SearchMode::Accurate.ef_search(), 512);
        assert_eq!(SearchMode::Perfect.ef_search(), usize::MAX);
    }

    #[test]
    fn test_search_mode_default_is_balanced() {
        // Arrange & Act
        let mode = SearchMode::default();

        // Assert
        assert_eq!(mode, SearchMode::Balanced);
    }

    #[test]
    fn test_search_mode_serialization() {
        // Arrange
        let mode = SearchMode::Accurate;

        // Act
        let json = serde_json::to_string(&mode).expect("serialize");
        let deserialized: SearchMode = serde_json::from_str(&json).expect("deserialize");

        // Assert
        assert_eq!(json, "\"accurate\"");
        assert_eq!(deserialized, mode);
    }

    // ========================================================================
    // VelesConfig default tests
    // ========================================================================

    #[test]
    fn test_config_default_values() {
        // Arrange & Act
        let config = VelesConfig::default();

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::Balanced);
        assert_eq!(config.search.max_results, 1000);
        assert_eq!(config.search.query_timeout_ms, 30000);
        assert!(config.search.ef_search.is_none());
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.storage.storage_mode, "mmap");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_config_from_toml_minimal() {
        // Arrange
        let toml = r#"
[search]
default_mode = "fast"
"#;

        // Act
        let config = VelesConfig::from_toml(toml).expect("parse");

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::Fast);
        // Other values should be defaults
        assert_eq!(config.server.port, 8080);
    }

    #[test]
    fn test_config_from_toml_full() {
        // Arrange
        let toml = r#"
[search]
default_mode = "accurate"
ef_search = 512
max_results = 500
query_timeout_ms = 60000

[hnsw]
m = 48
ef_construction = 600

[storage]
storage_mode = "mmap"

[limits]
max_dimensions = 2048
max_perfect_mode_vectors = 100000

[server]
host = "0.0.0.0"
port = 9090
workers = 8

[logging]
level = "debug"
format = "json"
"#;

        // Act
        let config = VelesConfig::from_toml(toml).expect("parse");

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::Accurate);
        assert_eq!(config.search.ef_search, Some(512));
        assert_eq!(config.search.max_results, 500);
        assert_eq!(config.hnsw.m, Some(48));
        assert_eq!(config.hnsw.ef_construction, Some(600));
        assert_eq!(config.storage.storage_mode, "mmap");
        assert_eq!(config.limits.max_dimensions, 2048);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.server.workers, 8);
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.format, "json");
    }

    #[test]
    fn test_config_from_toml_invalid_mode() {
        // Arrange
        let toml = r#"
[search]
default_mode = "ultra_fast"
"#;

        // Act
        let result = VelesConfig::from_toml(toml);

        // Assert
        assert!(result.is_err());
    }

    // ========================================================================
    // Validation tests
    // ========================================================================

    #[test]
    fn test_config_validate_success() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_config_validate_ef_search_too_low() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(10);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("search.ef_search"));
    }

    #[test]
    fn test_config_validate_ef_search_too_high() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(5000);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validate_invalid_storage_mode() {
        // Arrange
        let mut config = VelesConfig::default();
        config.storage.storage_mode = "disk".to_string();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("storage.storage_mode"));
    }

    #[test]
    fn test_config_validate_invalid_log_level() {
        // Arrange
        let mut config = VelesConfig::default();
        config.logging.level = "verbose".to_string();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("logging.level"));
    }

    #[test]
    fn test_config_validate_port_too_low() {
        // Arrange
        let mut config = VelesConfig::default();
        config.server.port = 80;

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("server.port"));
    }

    #[test]
    fn test_config_validate_hnsw_m_out_of_range() {
        // Arrange
        let mut config = VelesConfig::default();
        config.hnsw.m = Some(2);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validate_search_ef_search_out_of_range() {
        // Arrange: both ends of [16, 4096] pass, the values just past them fail (#2274)
        for (ef, valid) in [(15, false), (16, true), (4096, true), (4097, false)] {
            let mut config = VelesConfig::default();
            config.search.ef_search = Some(ef);

            // Act
            let result = config.validate();

            // Assert
            assert_eq!(result.is_ok(), valid, "search.ef_search = {ef}");
        }
    }

    #[test]
    fn test_from_toml_rejects_zero_max_collections() {
        // Regression (#907): loaders previously skipped validate(), silently
        // accepting out-of-range values. A zero capacity must now be rejected.
        let toml = r"
[limits]
max_collections = 0
";

        // Act
        let result = VelesConfig::from_toml(toml);

        // Assert
        assert!(result.is_err(), "max_collections = 0 must be rejected");
        let err = result.unwrap_err();
        assert!(err.to_string().contains("limits.max_collections"));
    }

    #[test]
    fn test_from_toml_default_config_validates_via_loader() {
        // Regression (#907): the DEFAULT config must still pass validation
        // when produced through the loader (no false-positive rejection).
        let result = VelesConfig::from_toml("");

        // Assert
        assert!(
            result.is_ok(),
            "default config must validate via loader: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_config_validate_query_timeout_just_under_cap() {
        // 24h cap (#907 follow-up): a long-but-finite batch timeout just below
        // the cap must validate (the old 1h cap rejected such legit configs).
        let mut config = VelesConfig::default();
        config.search.query_timeout_ms = 86_400_000 - 1; // just under 24h

        assert!(
            config.validate().is_ok(),
            "timeout just under the cap must validate"
        );
    }

    #[test]
    fn test_config_validate_query_timeout_zero_disables() {
        // `0` means "disabled" and must always validate.
        let mut config = VelesConfig::default();
        config.search.query_timeout_ms = 0;

        assert!(config.validate().is_ok(), "0 must disable, not reject");
    }

    #[test]
    fn test_config_validate_query_timeout_absurd_rejected() {
        // An absurdly large finite timeout (above the 24h cap) is rejected.
        let mut config = VelesConfig::default();
        config.search.query_timeout_ms = u64::MAX;

        let result = config.validate();
        assert!(result.is_err(), "absurd timeout must be rejected");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("search.query_timeout_ms"));
    }

    #[test]
    fn test_config_validate_workers_zero_is_auto() {
        // `0` workers means "auto" (derive from CPU count) and must validate.
        let mut config = VelesConfig::default();
        config.server.workers = 0;

        assert!(config.validate().is_ok(), "workers = 0 means auto");
    }

    #[test]
    fn test_config_validate_workers_too_large_rejected() {
        let mut config = VelesConfig::default();
        config.server.workers = usize::MAX;

        let result = config.validate();
        assert!(result.is_err(), "absurd worker count must be rejected");
        assert!(result.unwrap_err().to_string().contains("server.workers"));
    }

    // ========================================================================
    // Serialization tests
    // ========================================================================

    #[test]
    fn test_config_to_toml() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let toml_str = config.to_toml().expect("serialize");

        // Assert
        assert!(toml_str.contains("[search]"));
        assert!(toml_str.contains("default_mode"));
        assert!(toml_str.contains("[server]"));
        assert!(toml_str.contains("port = 8080"));
    }

    #[test]
    fn test_config_roundtrip() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.default_mode = SearchMode::Accurate;
        config.search.ef_search = Some(300);
        config.server.port = 9000;

        // Act
        let toml_str = config.to_toml().expect("serialize");
        let parsed = VelesConfig::from_toml(&toml_str).expect("parse");

        // Assert
        assert_eq!(parsed.search.default_mode, SearchMode::Accurate);
        assert_eq!(parsed.search.ef_search, Some(300));
        assert_eq!(parsed.server.port, 9000);
    }

    // ========================================================================
    // QuantizationType enum tests (PQ-06)
    // ========================================================================

    #[test]
    fn test_quantization_type_old_format_sq8() {
        // Old config.json format with default_type string
        let json = r#"{
            "default_type": "sq8",
            "rerank_enabled": true,
            "rerank_multiplier": 2,
            "auto_quantization": true,
            "auto_quantization_threshold": 10000
        }"#;
        let config: QuantizationConfig = serde_json::from_str(json).expect("deserialize old sq8");
        assert_eq!(*config.mode(), QuantizationType::SQ8);
        assert!(config.rerank_enabled);
        assert_eq!(config.rerank_multiplier, 2);
    }

    #[test]
    fn test_quantization_type_old_format_none() {
        let json = r#"{"default_type": "none"}"#;
        let config: QuantizationConfig = serde_json::from_str(json).expect("deserialize old none");
        assert_eq!(*config.mode(), QuantizationType::None);
    }

    #[test]
    fn test_quantization_type_old_format_binary() {
        let json = r#"{"default_type": "binary"}"#;
        let config: QuantizationConfig =
            serde_json::from_str(json).expect("deserialize old binary");
        assert_eq!(*config.mode(), QuantizationType::Binary);
    }

    #[test]
    fn test_quantization_type_new_format_pq() {
        let json = r#"{
            "mode": {"type": "pq", "m": 8, "k": 256, "opq_enabled": false, "oversampling": 4}
        }"#;
        let config: QuantizationConfig = serde_json::from_str(json).expect("deserialize new pq");
        match config.mode() {
            QuantizationType::PQ {
                m,
                k,
                opq_enabled,
                oversampling,
            } => {
                assert_eq!(*m, 8);
                assert_eq!(*k, 256);
                assert!(!opq_enabled);
                assert_eq!(*oversampling, Some(4));
            }
            other => panic!("Expected PQ, got {other:?}"),
        }
    }

    #[test]
    fn test_quantization_type_new_format_rabitq() {
        let json = r#"{"mode": {"type": "rabitq"}}"#;
        let config: QuantizationConfig =
            serde_json::from_str(json).expect("deserialize new rabitq");
        assert_eq!(*config.mode(), QuantizationType::RaBitQ);
    }

    #[test]
    fn test_quantization_type_roundtrip() {
        let config = QuantizationConfig {
            mode: QuantizationType::PQ {
                m: 16,
                k: 128,
                opq_enabled: true,
                oversampling: Some(8),
            },
            rerank_enabled: false,
            rerank_multiplier: 4,
            auto_quantization: false,
            auto_quantization_threshold: 50_000,
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let roundtripped: QuantizationConfig =
            serde_json::from_str(&json).expect("deserialize roundtrip");
        assert_eq!(*roundtripped.mode(), *config.mode());
        assert_eq!(roundtripped.rerank_enabled, config.rerank_enabled);
        assert_eq!(roundtripped.rerank_multiplier, config.rerank_multiplier);
    }

    #[test]
    fn test_quantization_type_default() {
        let config = QuantizationConfig::default();
        assert_eq!(*config.mode(), QuantizationType::None);
        assert!(config.rerank_enabled);
    }

    #[test]
    fn test_quantization_type_should_quantize() {
        let config = QuantizationConfig::default();
        assert!(config.should_quantize(15_000));
        assert!(!config.should_quantize(5_000));
    }

    #[test]
    fn test_quantization_type_is_pq() {
        let pq = QuantizationType::PQ {
            m: 8,
            k: 256,
            opq_enabled: false,
            oversampling: Some(4),
        };
        assert!(pq.is_pq());
        assert!(!pq.is_rabitq());
        assert!(!QuantizationType::SQ8.is_pq());
    }

    #[test]
    fn test_quantization_type_is_rabitq() {
        assert!(QuantizationType::RaBitQ.is_rabitq());
        assert!(!QuantizationType::RaBitQ.is_pq());
        assert!(!QuantizationType::None.is_rabitq());
    }

    #[test]
    fn test_quantization_type_pq_defaults() {
        // PQ with minimal JSON should fill in defaults for k and oversampling
        let json = r#"{"mode": {"type": "pq", "m": 8}}"#;
        let config: QuantizationConfig =
            serde_json::from_str(json).expect("deserialize pq defaults");
        match config.mode() {
            QuantizationType::PQ {
                m,
                k,
                opq_enabled,
                oversampling,
            } => {
                assert_eq!(*m, 8);
                assert_eq!(*k, 256); // default
                assert!(!opq_enabled); // default
                assert_eq!(*oversampling, Some(4)); // default
            }
            other => panic!("Expected PQ, got {other:?}"),
        }
    }

    // ========================================================================
    // Keys removed in 7.0.0 (#2174, #2087): a 6.x file must keep loading
    // ========================================================================

    /// Every key 7.0.0 removed, in one 6.x-shaped file. `mmap_cache_mb = 0` is
    /// the value 6.x refused at load; once the key is gone there is nothing
    /// left to range-check it against.
    const FILE_WITH_REMOVED_KEYS: &str = r#"
[hnsw]
m = 24

[storage]
storage_mode = "mmap"
data_dir = "/var/lib/velesdb"
mmap_cache_mb = 0
vector_alignment = 32

[wal_batch]
enabled = true
commit_delay_us = 250
max_batch_size = 64
"#;

    #[test]
    fn test_a_file_with_removed_keys_still_loads_through_both_parsers() {
        for (loader, result) in [
            ("from_toml", VelesConfig::from_toml(FILE_WITH_REMOVED_KEYS)),
            (
                "from_toml_engine_only",
                VelesConfig::from_toml_engine_only(FILE_WITH_REMOVED_KEYS),
            ),
        ] {
            let config = result.unwrap_or_else(|e| panic!("{loader} refused a 6.x file: {e}"));
            // The keys that still exist are applied next to the removed ones.
            assert_eq!(config.hnsw.m, Some(24), "for {loader}");
            assert_eq!(config.storage.storage_mode, "mmap", "for {loader}");
        }
    }

    #[test]
    fn test_a_file_with_removed_keys_still_loads_from_disk_through_both_loaders() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let path = dir.path().join("velesdb.toml");
        std::fs::write(&path, FILE_WITH_REMOVED_KEYS).expect("test: write toml");

        for (loader, result) in [
            ("load_from_path", VelesConfig::load_from_path(&path)),
            (
                "load_from_path_engine_only",
                VelesConfig::load_from_path_engine_only(&path),
            ),
        ] {
            let config = result.unwrap_or_else(|e| panic!("{loader} refused a 6.x file: {e}"));
            assert_eq!(config.hnsw.m, Some(24), "for {loader}");
        }
    }

    #[test]
    fn test_serialized_config_no_longer_writes_the_removed_keys() {
        // `to_toml` is how a config gets written back out; a removed key it
        // still emitted would be a key the next load warns about.
        let toml = VelesConfig::default().to_toml().expect("serialize");
        for removed in ["wal_batch", "data_dir", "mmap_cache_mb", "vector_alignment"] {
            assert!(
                !toml.contains(removed),
                "{removed} is still serialized:\n{toml}"
            );
        }
    }

    #[test]
    fn test_a_removed_key_is_found_whichever_source_sets_it() {
        // The environment reaches the same paths a file does —
        // `VELESDB_WAL_BATCH_ENABLED` maps to `wal_batch.enabled`
        // (config_env_tests) — so a value at a removed path is named whichever
        // provider put it there.
        use figment::{providers::Serialized, Figment};
        let figment = Figment::new()
            .merge(Serialized::defaults(VelesConfig::default()))
            .merge(Serialized::defaults(serde_json::json!({
                "wal_batch": { "enabled": true },
                "storage": { "vector_alignment": 32 },
            })));
        assert_eq!(
            crate::config_removed::removed_keys_set(&figment),
            vec!["wal_batch", "storage.vector_alignment"]
        );
    }

    #[test]
    fn test_the_defaults_set_no_removed_key() {
        // Every loader merges the defaults first; a removed key surviving in
        // them would make every load warn.
        use figment::{providers::Serialized, Figment};
        let figment = Figment::from(Serialized::defaults(VelesConfig::default()));
        assert!(crate::config_removed::removed_keys_set(&figment).is_empty());
    }

    #[cfg(feature = "persistence")]
    mod removed_key_warning {
        use crate::config::VelesConfig;
        use crate::test_fixtures::fixtures::capture_warns;

        const REMOVED: [&str; 4] = [
            "wal_batch",
            "storage.data_dir",
            "storage.mmap_cache_mb",
            "storage.vector_alignment",
        ];

        fn removal_warnings(warns: &[String]) -> Vec<&String> {
            warns.iter().filter(|w| w.contains("7.0.0")).collect()
        }

        #[test]
        fn test_each_parser_warns_once_naming_every_removed_key() {
            type Parser = fn(&str) -> Result<VelesConfig, crate::config::ConfigError>;
            let parsers: [(&str, Parser); 2] = [
                ("from_toml", VelesConfig::from_toml),
                ("from_toml_engine_only", VelesConfig::from_toml_engine_only),
            ];
            for (loader, parse) in parsers {
                let (result, warns) = capture_warns(|| parse(super::FILE_WITH_REMOVED_KEYS));
                result.unwrap_or_else(|e| panic!("{loader} refused a 6.x file: {e}"));
                let removal = removal_warnings(&warns);
                assert_eq!(
                    removal.len(),
                    1,
                    "{loader}: expected exactly one removal warning, got {warns:?}"
                );
                for key in REMOVED {
                    assert!(
                        removal[0].contains(key),
                        "{loader}: the removal warning does not name {key}: {}",
                        removal[0]
                    );
                }
            }
        }

        #[test]
        fn test_a_disabled_wal_batch_table_is_still_named() {
            // 6.x warned only on `enabled = true`; the table itself is what
            // is gone now, whatever it says.
            let (result, warns) =
                capture_warns(|| VelesConfig::from_toml("[wal_batch]\nenabled = false\n"));
            result.expect("a 6.x [wal_batch] table loads");
            let removal = removal_warnings(&warns);
            assert_eq!(removal.len(), 1, "got {warns:?}");
            assert!(removal[0].contains("wal_batch"), "got {warns:?}");
        }

        #[test]
        fn test_a_file_without_removed_keys_warns_about_nothing() {
            let (result, warns) = capture_warns(|| {
                VelesConfig::from_toml("[hnsw]\nm = 24\n\n[storage]\nstorage_mode = \"mmap\"\n")
            });
            result.expect("a current file loads");
            assert!(warns.is_empty(), "nothing to warn about, got {warns:?}");
        }
    }
}
