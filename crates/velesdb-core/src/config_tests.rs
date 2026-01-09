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
        assert_eq!(SearchMode::Fast.ef_search(), 64);
        assert_eq!(SearchMode::Balanced.ef_search(), 128);
        assert_eq!(SearchMode::Accurate.ef_search(), 256);
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
    fn test_config_effective_ef_search_from_mode() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let ef = config.effective_ef_search();

        // Assert
        assert_eq!(ef, 128); // Balanced mode
    }

    #[test]
    fn test_config_effective_ef_search_override() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(512);

        // Act
        let ef = config.effective_ef_search();

        // Assert
        assert_eq!(ef, 512);
    }

    // ========================================================================
    // TOML parsing tests
    // ========================================================================

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
data_dir = "/var/lib/velesdb"
storage_mode = "mmap"
mmap_cache_mb = 2048

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
        assert_eq!(config.storage.data_dir, "/var/lib/velesdb");
        assert_eq!(config.storage.mmap_cache_mb, 2048);
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
}
