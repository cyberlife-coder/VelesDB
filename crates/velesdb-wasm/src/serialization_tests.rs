use super::*;

#[test]
fn test_metric_roundtrip() {
    let metrics = [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
        DistanceMetric::Hamming,
        DistanceMetric::Jaccard,
    ];
    for metric in metrics {
        let byte = metric_to_byte(metric);
        let result = byte_to_metric(byte).unwrap();
        assert_eq!(metric, result);
    }
}

#[test]
fn test_v2_roundtrip_full_preserves_payloads() {
    let store = VectorStore {
        ids: vec![1, 2],
        data: vec![1.0, 2.0, 3.0, 4.0],
        data_sq8: Vec::new(),
        data_binary: Vec::new(),
        sq8_mins: Vec::new(),
        sq8_scales: Vec::new(),
        payloads: vec![Some(serde_json::json!({"k": "v"})), None],
        dimension: 2,
        metric: DistanceMetric::Cosine,
        storage_mode: StorageMode::Full,
        sparse_index: None,
    };
    let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
    assert_eq!(restored.ids, vec![1, 2]);
    assert_eq!(restored.data, vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(
        mode_to_byte(restored.storage_mode),
        mode_to_byte(StorageMode::Full)
    );
    assert_eq!(restored.payloads[0], Some(serde_json::json!({"k": "v"})));
    assert_eq!(restored.payloads[1], None);
}

#[test]
fn test_v2_roundtrip_sq8_preserves_mode_buffers_payload() {
    // v1 dropped the SQ8 buffers/mode (reloaded as Full) and all payloads.
    let store = VectorStore {
        ids: vec![10],
        data: Vec::new(),
        data_sq8: vec![200, 100],
        data_binary: Vec::new(),
        sq8_mins: vec![0.5],
        sq8_scales: vec![0.01],
        payloads: vec![Some(serde_json::json!({"x": 1}))],
        dimension: 2,
        metric: DistanceMetric::Euclidean,
        storage_mode: StorageMode::SQ8,
        sparse_index: None,
    };
    let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
    assert_eq!(
        mode_to_byte(restored.storage_mode),
        mode_to_byte(StorageMode::SQ8)
    );
    assert_eq!(restored.data_sq8, vec![200, 100]);
    assert_eq!(restored.sq8_mins, vec![0.5]);
    assert_eq!(restored.sq8_scales, vec![0.01]);
    assert_eq!(restored.payloads[0], Some(serde_json::json!({"x": 1})));
}

#[test]
fn test_v2_roundtrip_binary_does_not_panic() {
    // v1 panicked here by indexing the empty `data` buffer in Binary mode.
    let store = VectorStore {
        ids: vec![7],
        data: Vec::new(),
        data_sq8: Vec::new(),
        data_binary: vec![0b1010_1010],
        sq8_mins: Vec::new(),
        sq8_scales: Vec::new(),
        payloads: vec![None],
        dimension: 8,
        metric: DistanceMetric::Hamming,
        storage_mode: StorageMode::Binary,
        sparse_index: None,
    };
    let restored = import_from_bytes(&export_to_bytes(&store)).unwrap();
    assert_eq!(
        mode_to_byte(restored.storage_mode),
        mode_to_byte(StorageMode::Binary)
    );
    assert_eq!(restored.data_binary, vec![0b1010_1010]);
}
