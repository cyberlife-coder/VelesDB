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

fn page(points: usize, next_offset: Option<serde_json::Value>, has_more: bool) -> ExtractedBatch {
    let point = ExtractedPoint {
        id: "p".to_string(),
        vector: vec![0.0],
        payload: HashMap::new(),
        sparse_vector: None,
    };
    ExtractedBatch {
        points: vec![point; points],
        next_offset,
        has_more,
    }
}

#[test]
fn test_next_page_done_when_source_exhausted() {
    let batch = page(3, None, false);
    assert_eq!(batch.next_page(None).unwrap(), NextPage::Done);
}

#[test]
fn test_next_page_continues_past_empty_page_with_more() {
    let current = serde_json::json!(10);
    let batch = page(0, Some(serde_json::json!(20)), true);
    assert_eq!(
        batch.next_page(Some(&current)).unwrap(),
        NextPage::At(Some(serde_json::json!(20)))
    );
}

#[test]
fn test_next_page_rejects_cursor_that_did_not_advance() {
    let current = serde_json::json!("cursor-a");
    let stalled = page(0, Some(current.clone()), true);
    assert!(stalled.next_page(Some(&current)).is_err());

    // First page with no cursor that claims more but offers none.
    let no_cursor = page(1, None, true);
    assert!(no_cursor.next_page(None).is_err());
}
