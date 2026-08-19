use super::*;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::StorageMode;

#[test]
fn test_parse_metric_valid() {
    assert!(matches!(parse_metric("cosine"), Ok(DistanceMetric::Cosine)));
    assert!(matches!(
        parse_metric("EUCLIDEAN"),
        Ok(DistanceMetric::Euclidean)
    ));
    assert!(matches!(parse_metric("l2"), Ok(DistanceMetric::Euclidean)));
    assert!(matches!(
        parse_metric("dot"),
        Ok(DistanceMetric::DotProduct)
    ));
}

#[test]
fn test_parse_metric_invalid() {
    assert!(parse_metric("unknown").is_err());
}

#[test]
fn test_parse_storage_mode_valid() {
    assert!(matches!(parse_storage_mode("full"), Ok(StorageMode::Full)));
    assert!(matches!(parse_storage_mode("sq8"), Ok(StorageMode::SQ8)));
    assert!(matches!(
        parse_storage_mode("binary"),
        Ok(StorageMode::Binary)
    ));
    assert!(matches!(
        parse_storage_mode("pq"),
        Ok(StorageMode::ProductQuantization)
    ));
    assert!(matches!(
        parse_storage_mode("rabitq"),
        Ok(StorageMode::RaBitQ)
    ));
    // Case-insensitive (delegates to core `StorageMode::from_str`).
    assert!(matches!(
        parse_storage_mode("RaBitQ"),
        Ok(StorageMode::RaBitQ)
    ));
}

#[test]
fn test_metric_roundtrip() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ] {
        let s = metric_to_string(metric);
        assert_eq!(parse_metric(s).unwrap(), metric);
    }
}

#[test]
fn test_storage_mode_roundtrip() {
    for mode in [
        StorageMode::Full,
        StorageMode::SQ8,
        StorageMode::Binary,
        StorageMode::ProductQuantization,
        StorageMode::RaBitQ,
    ] {
        let s = storage_mode_to_string(mode);
        assert_eq!(parse_storage_mode(s).unwrap(), mode);
    }
}

#[cfg(feature = "persistence")]
#[test]
fn test_parse_search_quality_none_returns_none() {
    assert!(parse_search_quality(&None)
        .expect("test: should succeed for None")
        .is_none());
}

#[cfg(feature = "persistence")]
#[test]
fn test_parse_search_quality_named_modes() {
    for mode in ["fast", "balanced", "accurate", "perfect", "auto"] {
        assert!(
            parse_search_quality(&Some(mode.to_string()))
                .expect("test: named mode should succeed")
                .is_some(),
            "mode '{mode}' should parse successfully"
        );
    }
}

#[cfg(feature = "persistence")]
#[test]
fn test_parse_search_quality_custom_and_adaptive() {
    let custom =
        parse_search_quality(&Some("custom:256".to_string())).expect("test: custom should succeed");
    assert_eq!(custom, Some(velesdb_core::SearchQuality::Custom(256)));

    let adaptive = parse_search_quality(&Some("adaptive:32:512".to_string()))
        .expect("test: adaptive should succeed");
    assert_eq!(
        adaptive,
        Some(velesdb_core::SearchQuality::Adaptive {
            min_ef: 32,
            max_ef: 512,
        })
    );
}

#[cfg(feature = "persistence")]
#[test]
fn test_parse_search_quality_invalid() {
    assert!(parse_search_quality(&Some("nonexistent".to_string())).is_err());
    assert!(parse_search_quality(&Some(String::new())).is_err());
    assert!(parse_search_quality(&Some("custom:abc".to_string())).is_err());
    assert!(parse_search_quality(&Some("adaptive:512:32".to_string())).is_err());
}

// =====================================================================
// Fusion strategy tests
// =====================================================================

#[test]
fn test_parse_fusion_strategy_valid_strategies() {
    use velesdb_core::fusion::FusionStrategy;

    assert!(matches!(
        parse_fusion_strategy("rrf", None),
        Ok(FusionStrategy::RRF { k: 60 })
    ));
    assert!(matches!(
        parse_fusion_strategy("average", None),
        Ok(FusionStrategy::Average)
    ));
    assert!(matches!(
        parse_fusion_strategy("maximum", None),
        Ok(FusionStrategy::Maximum)
    ));
    assert!(matches!(
        parse_fusion_strategy("weighted", None),
        Ok(FusionStrategy::Weighted { .. })
    ));
    assert!(matches!(
        parse_fusion_strategy("relative_score", None),
        Ok(FusionStrategy::RelativeScore { .. })
    ));
    assert!(matches!(
        parse_fusion_strategy("rsf", None),
        Ok(FusionStrategy::RelativeScore { .. })
    ));
}

#[test]
fn test_parse_fusion_strategy_rrf_custom_k() {
    use velesdb_core::fusion::FusionStrategy;

    let params = serde_json::json!({ "k": 30 });
    let result = parse_fusion_strategy("rrf", Some(&params)).expect("test: valid RRF k");
    assert!(matches!(result, FusionStrategy::RRF { k: 30 }));
}

#[test]
fn test_parse_fusion_strategy_unknown_returns_error() {
    let result = parse_fusion_strategy("nonexistent", None);
    assert!(result.is_err(), "unknown strategy should return error");
}

#[test]
fn test_parse_fusion_strategy_case_insensitive() {
    assert!(parse_fusion_strategy("RRF", None).is_ok());
    assert!(parse_fusion_strategy("Average", None).is_ok());
    assert!(parse_fusion_strategy("MAXIMUM", None).is_ok());
}

// =====================================================================
// require_collection type-check tests
// =====================================================================

#[cfg(feature = "persistence")]
#[test]
fn test_require_collection_rejects_graph_collection() {
    let tmp = tempfile::TempDir::new().expect("test: create temp dir");
    let db = velesdb_core::Database::open(tmp.path().to_str().expect("test: path"))
        .expect("test: open db");
    db.create_graph_collection("kg", velesdb_core::GraphSchema::schemaless())
        .expect("test: create graph collection");

    let result = require_collection(&db, "kg");
    assert!(
        result.is_err(),
        "require_collection should reject graph collections"
    );
}
