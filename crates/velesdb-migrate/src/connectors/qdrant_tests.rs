use super::*;

#[test]
fn test_normalise_qdrant_metric_maps_euclid_to_euclidean() {
    // Qdrant reports `Euclid` for L2 distance; VelesDB core uses
    // `euclidean`. The mapping is what allows check_metric_fidelity
    // to honestly compare a Qdrant source against a core collection
    // created with metric: "euclidean".
    assert_eq!(
        QdrantConnector::normalise_qdrant_metric("Euclid"),
        "euclidean"
    );
    assert_eq!(
        QdrantConnector::normalise_qdrant_metric("EUCLID"),
        "euclidean"
    );
}

#[test]
fn test_normalise_qdrant_metric_lowercases_known_values() {
    assert_eq!(QdrantConnector::normalise_qdrant_metric("Cosine"), "cosine");
    assert_eq!(QdrantConnector::normalise_qdrant_metric("Dot"), "dot");
}

#[test]
fn test_normalise_qdrant_metric_preserves_unknown_values() {
    // Manhattan is a valid Qdrant metric (1.8+) but not supported
    // by VelesDB core — preserved verbatim so mismatch errors are
    // actionable rather than masked.
    assert_eq!(
        QdrantConnector::normalise_qdrant_metric("Manhattan"),
        "manhattan"
    );
}

#[test]
fn test_qdrant_point_id_display() {
    let num_id = QdrantPointId::Num(12345);
    assert_eq!(num_id.to_string(), "12345");

    let uuid_id = QdrantPointId::Uuid("abc-123".to_string());
    assert_eq!(uuid_id.to_string(), "abc-123");
}

#[test]
fn test_qdrant_vector_into_dense() {
    let single = QdrantVector::Single(vec![0.1, 0.2, 0.3]);
    assert_eq!(single.into_dense(), vec![0.1, 0.2, 0.3]);

    let named = QdrantVector::Named(HashMap::from([(
        "default".to_string(),
        QdrantNamedVectorValue::Dense(vec![0.4, 0.5, 0.6]),
    )]));
    assert_eq!(named.into_dense(), vec![0.4, 0.5, 0.6]);
}

#[test]
fn test_scroll_request_serialization() {
    let req = ScrollRequest {
        limit: 100,
        with_payload: true,
        with_vector: true,
        offset: None,
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"limit\":100"));
    assert!(!json.contains("offset")); // Skip serializing None
}

#[test]
fn test_qdrant_sparse_vector_deserialization() {
    let json = r#"{"indices":[10,45],"values":[0.5,0.3]}"#;
    let sv: QdrantSparseVector = serde_json::from_str(json).unwrap();
    assert_eq!(sv.indices, vec![10, 45]);
    assert_eq!(sv.values, vec![0.5, 0.3]);
}

#[test]
fn test_qdrant_named_vector_with_sparse() {
    let named = QdrantVector::Named(HashMap::from([
        (
            "dense".to_string(),
            QdrantNamedVectorValue::Dense(vec![0.1, 0.2]),
        ),
        (
            "sparse".to_string(),
            QdrantNamedVectorValue::Sparse(QdrantSparseVector {
                indices: vec![3, 7, 42],
                values: vec![0.9, 0.1, 0.5],
            }),
        ),
    ]));

    let sparse = named.extract_sparse();
    assert!(sparse.is_some());
    let pairs = sparse.unwrap();
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0].0, 3);
    assert!((pairs[0].1 - 0.9).abs() < f32::EPSILON);
}

#[test]
fn test_qdrant_single_vector_no_sparse() {
    let single = QdrantVector::Single(vec![1.0, 2.0, 3.0]);
    assert!(single.extract_sparse().is_none());
}

#[test]
fn test_qdrant_sparse_mismatched_lengths() {
    let named = QdrantVector::Named(HashMap::from([(
        "bad_sparse".to_string(),
        QdrantNamedVectorValue::Sparse(QdrantSparseVector {
            indices: vec![1, 2, 3],
            values: vec![0.5, 0.3],
        }),
    )]));

    assert!(named.extract_sparse().is_none());
}

#[test]
fn test_qdrant_sparse_only_into_dense_is_empty() {
    let json = r#"{"sparse":{"indices":[1,2],"values":[0.5,0.3]}}"#;

    let map: HashMap<String, QdrantNamedVectorValue> =
        serde_json::from_str(json).expect("valid JSON");
    let v = QdrantVector::Named(map);
    assert!(v.extract_sparse().is_some());

    // `into_dense` consumes, so test separately
    let map2: HashMap<String, QdrantNamedVectorValue> =
        serde_json::from_str(json).expect("valid JSON");
    let v2 = QdrantVector::Named(map2);
    assert!(v2.into_dense().is_empty());
}

#[test]
fn test_connect_rejects_file_url() {
    let config = QdrantConfig {
        url: "file:///etc/passwd".to_string(),
        collection: "test".to_string(),
        api_key: None,
        payload_fields: vec![],
    };
    let connector = QdrantConnector::new(config);
    // validate_url rejects file:// synchronously at connect time
    assert!(crate::connectors::common::validate_url(&connector.config.url).is_err());
}
