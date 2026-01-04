//! Tests for JSON file connector.

use super::*;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_json_connector_simple_array() {
    let json_content = r#"[
        {"id": "1", "vector": [0.1, 0.2, 0.3], "title": "Doc 1"},
        {"id": "2", "vector": [0.4, 0.5, 0.6], "title": "Doc 2"}
    ]"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(json_content.as_bytes()).unwrap();

    let config = JsonFileConfig {
        path: file.path().to_path_buf(),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    };

    let mut connector = JsonFileConnector::new(config);
    connector.connect().await.unwrap();
    let schema = connector.get_schema().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(schema.dimension, 3);
    assert_eq!(schema.total_count, Some(2));
    assert_eq!(batch.points.len(), 2);
    assert_eq!(batch.points[0].id, "1");
    assert_eq!(batch.points[0].vector, vec![0.1, 0.2, 0.3]);
}

#[tokio::test]
async fn test_json_connector_nested_path() {
    let json_content = r#"{"data": {"vectors": [{"id": "a", "vector": [1.0, 2.0]}]}}"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(json_content.as_bytes()).unwrap();

    let config = JsonFileConfig {
        path: file.path().to_path_buf(),
        array_path: "data.vectors".to_string(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    };

    let mut connector = JsonFileConnector::new(config);
    connector.connect().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(batch.points.len(), 1);
    assert_eq!(batch.points[0].id, "a");
}

#[tokio::test]
async fn test_json_connector_pagination() {
    let mut items = Vec::new();
    for i in 0..100 {
        items.push(serde_json::json!({"id": format!("id_{}", i), "vector": [i as f32 * 0.1]}));
    }
    let json_content = serde_json::to_string(&items).unwrap();

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(json_content.as_bytes()).unwrap();

    let config = JsonFileConfig {
        path: file.path().to_path_buf(),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    };

    let mut connector = JsonFileConnector::new(config);
    connector.connect().await.unwrap();

    let batch1 = connector.extract_batch(None, 30).await.unwrap();
    assert_eq!(batch1.points.len(), 30);
    assert!(batch1.has_more);

    let batch2 = connector
        .extract_batch(batch1.next_offset, 30)
        .await
        .unwrap();
    assert_eq!(batch2.points.len(), 30);
}

#[tokio::test]
async fn test_json_connector_auto_generated_ids() {
    let json_content = r#"[{"vector": [0.1, 0.2]}, {"vector": [0.3, 0.4]}]"#;

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(json_content.as_bytes()).unwrap();

    let config = JsonFileConfig {
        path: file.path().to_path_buf(),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    };

    let mut connector = JsonFileConnector::new(config);
    connector.connect().await.unwrap();
    let batch = connector.extract_batch(None, 10).await.unwrap();

    assert_eq!(batch.points[0].id, "row_0");
    assert_eq!(batch.points[1].id, "row_1");
}

#[tokio::test]
async fn test_json_connector_invalid_json() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"not valid json").unwrap();

    let config = JsonFileConfig {
        path: file.path().to_path_buf(),
        array_path: String::new(),
        id_field: "id".to_string(),
        vector_field: "vector".to_string(),
        payload_fields: vec![],
    };

    let mut connector = JsonFileConnector::new(config);
    let result = connector.connect().await;

    assert!(result.is_err());
}
