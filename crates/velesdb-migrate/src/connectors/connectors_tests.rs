use super::*;

#[test]
fn test_extracted_point_serialization() {
    let point = ExtractedPoint {
        id: "test-123".to_string(),
        vector: vec![0.1, 0.2, 0.3],
        payload: HashMap::from([
            ("title".to_string(), serde_json::json!("Test Document")),
            ("score".to_string(), serde_json::json!(0.95)),
        ]),
        sparse_vector: None,
    };

    let json = serde_json::to_string(&point).unwrap();
    let parsed: ExtractedPoint = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.id, "test-123");
    assert_eq!(parsed.vector.len(), 3);
}

#[test]
fn test_source_schema() {
    let schema = SourceSchema {
        source_type: "qdrant".to_string(),
        collection: "documents".to_string(),
        dimension: 768,
        total_count: Some(10000),
        fields: vec![FieldInfo {
            name: "title".to_string(),
            field_type: "string".to_string(),
            indexed: true,
        }],
        ..Default::default()
    };

    assert_eq!(schema.dimension, 768);
    assert_eq!(schema.total_count, Some(10000));
}
