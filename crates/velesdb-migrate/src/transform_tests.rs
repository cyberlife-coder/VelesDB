use super::*;

#[test]
fn test_transform_point_no_mapping() {
    let transformer = Transformer::default();

    let point = ExtractedPoint {
        id: "1".to_string(),
        vector: vec![0.1, 0.2],
        payload: HashMap::from([("title".to_string(), serde_json::json!("Test"))]),
        sparse_vector: None,
    };

    let result = transformer.transform_point(point);
    assert!(result.payload.contains_key("title"));
}

#[test]
fn test_transform_point_with_mapping() {
    let mappings = HashMap::from([("old_name".to_string(), "new_name".to_string())]);
    let transformer = Transformer::new(mappings);

    let point = ExtractedPoint {
        id: "1".to_string(),
        vector: vec![0.1, 0.2],
        payload: HashMap::from([("old_name".to_string(), serde_json::json!("Test"))]),
        sparse_vector: None,
    };

    let result = transformer.transform_point(point);
    assert!(result.payload.contains_key("new_name"));
    assert!(!result.payload.contains_key("old_name"));
}

#[test]
fn test_normalize_vector() {
    let vec = vec![3.0, 4.0];
    let normalized = Transformer::normalize_vector(&vec);

    assert!((normalized[0] - 0.6).abs() < 0.001);
    assert!((normalized[1] - 0.8).abs() < 0.001);

    // Check unit length
    let norm: f32 = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 0.001);
}

#[test]
fn test_normalize_zero_vector() {
    let vec = vec![0.0, 0.0, 0.0];
    let normalized = Transformer::normalize_vector(&vec);
    assert_eq!(normalized, vec![0.0, 0.0, 0.0]);
}

#[test]
fn test_quantize_sq8() {
    let vec = vec![0.0, 0.5, 1.0];
    let quantized = Transformer::quantize_sq8(&vec);

    assert_eq!(quantized[0], 0);
    assert_eq!(quantized[1], 127); // ~128
    assert_eq!(quantized[2], 255);
}

#[test]
fn test_quantize_binary() {
    let vec = vec![1.0, -1.0, 0.5, -0.5, 1.0, -1.0, 0.1, -0.1];
    let binary = Transformer::quantize_binary(&vec);

    // First byte: 1 0 1 0 1 0 1 0 = 0xAA = 170
    assert_eq!(binary.len(), 1);
    assert_eq!(binary[0], 0b10101010);
}

#[test]
fn test_transform_batch() {
    let transformer = Transformer::default();

    let points = vec![
        ExtractedPoint {
            id: "1".to_string(),
            vector: vec![0.1],
            payload: HashMap::new(),
            sparse_vector: None,
        },
        ExtractedPoint {
            id: "2".to_string(),
            vector: vec![0.2],
            payload: HashMap::new(),
            sparse_vector: None,
        },
    ];

    let result = transformer.transform_batch(points);
    assert_eq!(result.len(), 2);
}
