use super::*;
use crate::config::DistanceMetric;

// ─────────────────────────────────────────────────────────────
// Issue #1542: golden vectors for `fnv1a64`, captured against the
// pre-refactor private implementation. `fnv1a64` is about to start
// delegating to `velesdb_core::hash_id_bytes` instead of re-declaring
// its own FNV-1a constants; these values must stay byte-identical
// after that change (checkpoint-resumed migrations depend on it — see
// `stable_point_id`'s cross-version stability guarantee).
// ─────────────────────────────────────────────────────────────

#[test]
fn test_fnv1a64_golden_vectors_unchanged_by_delegation() {
    let vectors: &[(&str, u64)] = &[
        ("", 0xcbf2_9ce4_8422_2325),
        ("a", 0xaf63_dc4c_8601_ec8c),
        ("hello", 0xa430_d846_80aa_bd0b),
        ("world", 0x4f59_ff5e_730c_8af3),
        ("tenant:acme", 0x434a_088f_8b77_5207),
        // Multi-byte UTF-8: 2-byte (é), 3-byte (CJK), and 4-byte (emoji)
        // sequences must hash over raw bytes, not code points.
        ("café", 0x48e8_823a_cfa4_0d89),
        ("日本語", 0xee9e_e2b5_c854_ef87),
        ("emoji:🚀", 0x5063_383e_8fb5_57fa),
        ("mixed-Ünïcödé-42", 0x3019_47e7_0a3d_8809),
        ("fact:the sky is blue", 0x5ff1_6ac5_c3bf_e13b),
    ];

    for (input, expected) in vectors {
        assert_eq!(
            fnv1a64(input.as_bytes()),
            *expected,
            "fnv1a64({input:?}) drifted from its pre-refactor golden vector"
        );
    }
}

#[test]
fn test_check_metric_fidelity_passes_when_source_is_none() {
    // File-backed connectors (JSON/CSV) and legacy connectors
    // that have not been extended yet report `None`. The check
    // must never fail in that case.
    assert!(check_metric_fidelity(None, DistanceMetric::Cosine, false).is_ok());
}

#[test]
fn test_check_metric_fidelity_passes_on_exact_match() {
    assert!(check_metric_fidelity(Some("cosine"), DistanceMetric::Cosine, false).is_ok());
    assert!(check_metric_fidelity(Some("euclidean"), DistanceMetric::Euclidean, false).is_ok());
    assert!(check_metric_fidelity(Some("dot"), DistanceMetric::Dot, false).is_ok());
}

#[test]
fn test_check_metric_fidelity_passes_on_alias_match() {
    // Source reports "l2" which canonicalises to "euclidean".
    assert!(check_metric_fidelity(Some("l2"), DistanceMetric::Euclidean, false).is_ok());
    // Source reports "ip" (inner product) which canonicalises to "dot".
    assert!(check_metric_fidelity(Some("ip"), DistanceMetric::Dot, false).is_ok());
    // Case-insensitive.
    assert!(check_metric_fidelity(Some("COSINE"), DistanceMetric::Cosine, false).is_ok());
}

#[test]
fn test_check_metric_fidelity_rejects_known_mismatch() {
    let result = check_metric_fidelity(Some("cosine"), DistanceMetric::Euclidean, false);
    assert!(result.is_err(), "cosine vs euclidean must fail");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cosine") && err.contains("euclidean"),
        "error must name both metrics, got: {err}"
    );
    assert!(
        err.contains("allow_metric_mismatch"),
        "error must point at the escape hatch, got: {err}"
    );
}

#[test]
fn test_check_metric_fidelity_rejects_unknown_label() {
    let result = check_metric_fidelity(Some("some_weird_metric"), DistanceMetric::Cosine, false);
    assert!(result.is_err(), "unknown metric must fail by default");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("some_weird_metric"),
        "error must preserve the raw source label for diagnostics, got: {err}"
    );
}

#[test]
fn test_check_metric_fidelity_allows_mismatch_when_opted_in() {
    // Explicit opt-in via allow_metric_mismatch bypasses the
    // hard error for both recognised-but-mismatching and
    // unrecognised source labels.
    assert!(
        check_metric_fidelity(Some("cosine"), DistanceMetric::Euclidean, true).is_ok(),
        "allow_metric_mismatch must permit known mismatches"
    );
    assert!(
        check_metric_fidelity(Some("weird"), DistanceMetric::Cosine, true).is_ok(),
        "allow_metric_mismatch must permit unknown source labels"
    );
}

#[test]
fn test_check_metric_fidelity_hamming_and_jaccard() {
    assert!(check_metric_fidelity(Some("hamming"), DistanceMetric::Hamming, false).is_ok());
    assert!(check_metric_fidelity(Some("jaccard"), DistanceMetric::Jaccard, false).is_ok());
    // Cross-metric mismatch.
    assert!(
        check_metric_fidelity(Some("hamming"), DistanceMetric::Jaccard, false).is_err(),
        "hamming vs jaccard must fail"
    );
}

#[test]
fn test_migration_stats_throughput() {
    let stats = MigrationStats {
        extracted: 1000,
        loaded: 1000,
        failed: 0,
        batches: 10,
        duration_secs: 2.0,
        edges_created: 0,
        edges_failed: 0,
        relations_processed: 0,
    };

    assert!((stats.throughput() - 500.0).abs() < 0.001);
}

#[test]
fn test_migration_stats_zero_duration() {
    let stats = MigrationStats::default();
    assert_eq!(stats.throughput(), 0.0);
}

#[test]
fn test_stable_point_id_is_deterministic_for_text_ids() {
    let first = crate::pipeline_points::stable_point_id("doc-alpha");
    let second = crate::pipeline_points::stable_point_id("doc-alpha");
    let other = crate::pipeline_points::stable_point_id("doc-beta");

    assert_eq!(first, second);
    assert_ne!(first, other);
}

#[tokio::test]
async fn test_pipeline_dry_run_loaded_stays_zero() {
    use crate::config::{
        DestinationConfig, DistanceMetric, MigrationConfig, MigrationOptions, SourceConfig,
        StorageMode,
    };
    use crate::connectors::json_file::JsonFileConfig;
    use tempfile::TempDir;

    let dir = TempDir::new().expect("test: create tempdir");
    let json_path = dir.path().join("test_data.json");
    // 3 points with 2-dimensional vectors
    let json_content = serde_json::json!([
        {"id": "1", "vector": [0.1, 0.2], "payload": {}},
        {"id": "2", "vector": [0.3, 0.4], "payload": {}},
        {"id": "3", "vector": [0.5, 0.6], "payload": {}}
    ]);
    std::fs::write(&json_path, json_content.to_string()).expect("test: write json");

    let config = MigrationConfig {
        source: SourceConfig::JsonFile(JsonFileConfig {
            path: json_path,
            array_path: String::new(),
            id_field: "id".to_string(),
            vector_field: "vector".to_string(),
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: dir.path().to_path_buf(),
            collection: "dry_run_test".to_string(),
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
            graph_collection: None,
        },
        options: MigrationOptions {
            dry_run: true,
            ..MigrationOptions::default()
        },
        relations: vec![],
    };

    let mut pipeline = crate::Pipeline::new(config).expect("test: create pipeline");
    let stats = pipeline.run().await.expect("test: run pipeline");

    assert_eq!(stats.extracted, 3, "Should extract 3 points");
    assert_eq!(stats.loaded, 0, "dry_run must not increment loaded");
}

#[test]
fn test_checkpoint_path_uses_explicit_path_when_present() {
    let config = MigrationConfig {
        source: crate::config::SourceConfig::Qdrant(crate::config::QdrantConfig {
            url: "http://localhost:6333".to_string(),
            collection: "docs".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: crate::config::DestinationConfig {
            path: std::path::PathBuf::from("./data"),
            collection: "docs".to_string(),
            dimension: 3,
            metric: crate::config::DistanceMetric::Cosine,
            storage_mode: crate::config::StorageMode::Full,
            graph_collection: None,
        },
        options: crate::config::MigrationOptions {
            checkpoint_path: Some(std::path::PathBuf::from("./custom-checkpoint.json")),
            ..crate::config::MigrationOptions::default()
        },
        relations: vec![],
    };

    assert_eq!(
        checkpoint_path(&config),
        std::path::PathBuf::from("./custom-checkpoint.json")
    );
}
