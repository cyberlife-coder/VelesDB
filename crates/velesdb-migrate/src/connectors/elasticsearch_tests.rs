//! Tests for Elasticsearch/OpenSearch connector.

use super::*;
use crate::connectors::SourceConnector;

fn test_config() -> ElasticsearchConfig {
    ElasticsearchConfig {
        url: "http://localhost:9200".to_string(),
        index: "vectors".to_string(),
        vector_field: "embedding".to_string(),
        id_field: "_id".to_string(),
        payload_fields: vec![],
        username: None,
        password: None,
        api_key: None,
        query: None,
    }
}

#[test]
fn test_elasticsearch_config_defaults() {
    let json = r#"{"url":"http://localhost:9200","index":"vectors"}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.vector_field, "embedding");
    assert_eq!(config.id_field, "_id");
    assert!(config.query.is_none());
}

#[test]
fn test_elasticsearch_config_with_auth() {
    let json =
        r#"{"url":"http://localhost:9200","index":"v","username":"elastic","password":"secret"}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.username, Some("elastic".to_string()));
}

#[test]
fn test_elasticsearch_config_with_api_key() {
    let json = r#"{"url":"https://cloud.es:9243","index":"v","api_key":"base64key"}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.api_key, Some("base64key".to_string()));
}

#[test]
fn test_elasticsearch_config_with_query() {
    let json =
        r#"{"url":"http://localhost:9200","index":"v","query":{"term":{"status":"active"}}}"#;
    let config: ElasticsearchConfig = serde_json::from_str(json).unwrap();
    assert!(config.query.is_some());
}

#[test]
fn test_elasticsearch_connector_new() {
    let connector = ElasticsearchConnector::new(test_config());
    assert_eq!(connector.source_type(), "elasticsearch");
}

#[test]
fn test_elasticsearch_build_search_url() {
    let mut config = test_config();
    config.index = "my-vectors".to_string();
    let connector = ElasticsearchConnector::new(config);
    assert_eq!(
        connector.build_search_url(),
        "http://localhost:9200/my-vectors/_search"
    );
}

#[test]
fn test_elasticsearch_build_search_url_trailing_slash() {
    let mut config = test_config();
    config.url = "http://localhost:9200/".to_string();
    let connector = ElasticsearchConnector::new(config);
    assert_eq!(
        connector.build_search_url(),
        "http://localhost:9200/vectors/_search"
    );
}

#[test]
fn test_elasticsearch_build_count_url() {
    let connector = ElasticsearchConnector::new(test_config());
    assert_eq!(
        connector.build_count_url(),
        "http://localhost:9200/vectors/_count"
    );
}

#[test]
fn test_elasticsearch_parse_vector_success() {
    let connector = ElasticsearchConnector::new(test_config());
    let source = serde_json::json!({"embedding": [0.1, 0.2, 0.3], "title": "Test"});
    let vector = connector.parse_vector(&source).unwrap();
    assert_eq!(vector, vec![0.1, 0.2, 0.3]);
}

#[test]
fn test_elasticsearch_parse_vector_missing() {
    let connector = ElasticsearchConnector::new(test_config());
    let source = serde_json::json!({"title": "No vector"});
    assert!(connector.parse_vector(&source).is_err());
}

#[test]
fn test_elasticsearch_extract_payload() {
    let connector = ElasticsearchConnector::new(test_config());
    let source = serde_json::json!({"embedding": [0.1], "title": "Test", "count": 42});
    let payload = connector.extract_payload(&source);
    assert_eq!(payload.len(), 2);
    assert!(!payload.contains_key("embedding"));
}

#[test]
fn test_elasticsearch_extract_payload_filtered() {
    let mut config = test_config();
    config.payload_fields = vec!["title".to_string()];
    let connector = ElasticsearchConnector::new(config);
    let source = serde_json::json!({"embedding": [0.1], "title": "T", "count": 42});
    let payload = connector.extract_payload(&source);
    assert_eq!(payload.len(), 1);
    assert!(payload.contains_key("title"));
}

#[test]
fn test_search_request_serialization() {
    let req = SearchRequest {
        query: Some(serde_json::json!({"match_all": {}})),
        size: Some(100),
        from: None,
        sort: Some(vec![serde_json::json!({"_id": "asc"})]),
        search_after: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["size"], 100);
    assert!(json.get("from").is_none());
}

#[test]
fn test_search_request_with_search_after() {
    let req = SearchRequest {
        query: Some(serde_json::json!({"match_all": {}})),
        size: Some(50),
        from: None,
        sort: Some(vec![serde_json::json!({"_id": "asc"})]),
        search_after: Some(vec![serde_json::json!("last_id")]),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["search_after"][0], "last_id");
}

#[test]
fn test_search_response_deserialization() {
    let json = r#"{"hits":{"total":{"value":1000},"hits":[{"_id":"doc1","_source":{"embedding":[0.1],"title":"T"},"sort":["doc1"]}]}}"#;
    let response: SearchResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.hits.hits.len(), 1);
    assert_eq!(response.hits.hits[0].id, "doc1");
}
