use super::*;

#[test]
fn test_config_defaults() {
    let options = MigrationOptions::default();
    assert_eq!(options.batch_size, 1000);
    assert_eq!(options.workers, 4);
    assert!(options.checkpoint_enabled);
    assert!(!options.dry_run);
}

#[test]
fn test_config_validate_dimension() {
    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: "http://localhost:6333".to_string(),
            collection: "test".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: PathBuf::from("./test_db"),
            collection: "test".to_string(),
            dimension: 0,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
            graph_collection: None,
        },
        options: MigrationOptions::default(),
        relations: vec![],
    };

    let result = config.validate();
    assert!(result.is_err());
}

#[test]
fn test_config_validate_batch_size() {
    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: "http://localhost:6333".to_string(),
            collection: "test".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: PathBuf::from("./test_db"),
            collection: "test".to_string(),
            dimension: 8,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
            graph_collection: None,
        },
        options: MigrationOptions {
            batch_size: 0,
            ..MigrationOptions::default()
        },
        relations: vec![],
    };

    let result = config.validate();
    assert!(result.is_err());
}

#[test]
fn test_config_validate_workers() {
    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: "http://localhost:6333".to_string(),
            collection: "test".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: PathBuf::from("./test_db"),
            collection: "test".to_string(),
            dimension: 8,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
            graph_collection: None,
        },
        options: MigrationOptions {
            workers: 0,
            ..MigrationOptions::default()
        },
        relations: vec![],
    };

    let result = config.validate();
    assert!(result.is_err());
}

#[test]
fn test_config_yaml_parse() {
    let yaml = r#"
source:
  type: qdrant
  url: http://localhost:6333
  collection: documents
destination:
  path: ./velesdb_data
  collection: docs
  dimension: 768
options:
  batch_size: 500
"#;
    let config: MigrationConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.destination.dimension, 768);
    assert_eq!(config.options.batch_size, 500);
}

/// Backward-compatibility guard: every TOML spelling/alias accepted before
/// the core-mapping was routed through `velesdb_core` must still
/// deserialize AND map to the same core variant.
#[test]
fn test_distance_metric_aliases_deserialize_and_map_to_core() {
    let cases: &[(&str, DistanceMetric, velesdb_core::DistanceMetric)] = &[
        (
            "cosine",
            DistanceMetric::Cosine,
            velesdb_core::DistanceMetric::Cosine,
        ),
        (
            "euclidean",
            DistanceMetric::Euclidean,
            velesdb_core::DistanceMetric::Euclidean,
        ),
        (
            "dot",
            DistanceMetric::Dot,
            velesdb_core::DistanceMetric::DotProduct,
        ),
        (
            "DotProduct",
            DistanceMetric::Dot,
            velesdb_core::DistanceMetric::DotProduct,
        ),
        (
            "dot_product",
            DistanceMetric::Dot,
            velesdb_core::DistanceMetric::DotProduct,
        ),
        (
            "hamming",
            DistanceMetric::Hamming,
            velesdb_core::DistanceMetric::Hamming,
        ),
        (
            "jaccard",
            DistanceMetric::Jaccard,
            velesdb_core::DistanceMetric::Jaccard,
        ),
    ];
    for (spelling, expected_cfg, expected_core) in cases {
        let parsed: DistanceMetric = serde_json::from_value(serde_json::json!(spelling))
            .unwrap_or_else(|e| panic!("alias '{spelling}' must still deserialize: {e}"));
        assert!(
            matches!(
                (parsed, *expected_cfg),
                (DistanceMetric::Cosine, DistanceMetric::Cosine)
                    | (DistanceMetric::Euclidean, DistanceMetric::Euclidean)
                    | (DistanceMetric::Dot, DistanceMetric::Dot)
                    | (DistanceMetric::Hamming, DistanceMetric::Hamming)
                    | (DistanceMetric::Jaccard, DistanceMetric::Jaccard)
            ),
            "alias '{spelling}' deserialized to the wrong config variant"
        );
        assert_eq!(
            velesdb_core::DistanceMetric::from(parsed),
            *expected_core,
            "alias '{spelling}' mapped to the wrong core metric"
        );
    }
}

/// Backward-compatibility guard for storage-mode TOML spellings/aliases.
#[test]
fn test_storage_mode_aliases_deserialize_and_map_to_core() {
    let cases: &[(&str, velesdb_core::StorageMode)] = &[
        ("full", velesdb_core::StorageMode::Full),
        ("sq8", velesdb_core::StorageMode::SQ8),
        ("binary", velesdb_core::StorageMode::Binary),
        ("pq", velesdb_core::StorageMode::ProductQuantization),
        (
            "product_quantization",
            velesdb_core::StorageMode::ProductQuantization,
        ),
        ("rabitq", velesdb_core::StorageMode::RaBitQ),
    ];
    for (spelling, expected_core) in cases {
        let parsed: StorageMode = serde_json::from_value(serde_json::json!(spelling))
            .unwrap_or_else(|e| panic!("alias '{spelling}' must still deserialize: {e}"));
        assert_eq!(
            velesdb_core::StorageMode::from(parsed),
            *expected_core,
            "alias '{spelling}' mapped to the wrong core storage mode"
        );
    }
}
