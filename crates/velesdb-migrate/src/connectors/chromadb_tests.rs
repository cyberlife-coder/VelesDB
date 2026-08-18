use super::*;

#[test]
fn test_chromadb_connector_new() {
    let config = ChromaDBConfig {
        url: "http://localhost:8000".to_string(),
        collection: "test".to_string(),
        tenant: None,
        database: None,
    };

    let connector = ChromaDBConnector::new(config);
    assert_eq!(connector.source_type(), "chromadb");
    assert!(connector.collection_id.is_none());
}

#[test]
fn test_get_request_serialization() {
    let req = GetRequest {
        ids: None,
        limit: Some(100),
        offset: Some(0),
        include: vec!["embeddings".to_string()],
    };

    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"limit\":100"));
    assert!(json.contains("\"include\":[\"embeddings\"]"));
    assert!(!json.contains("\"ids\"")); // Skip None
}

#[test]
fn test_base_url_with_tenant() {
    let config = ChromaDBConfig {
        url: "http://localhost:8000".to_string(),
        collection: "test".to_string(),
        tenant: Some("my_tenant".to_string()),
        database: Some("my_db".to_string()),
    };

    let connector = ChromaDBConnector::new(config);
    let url = connector.base_url();
    assert!(url.contains("tenants/my_tenant/databases/my_db"));
}
