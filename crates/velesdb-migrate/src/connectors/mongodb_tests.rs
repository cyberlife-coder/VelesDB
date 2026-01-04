//! Tests for MongoDB Atlas Vector Search connector.

use super::*;
use crate::connectors::SourceConnector;

fn test_config() -> MongoDBConfig {
    MongoDBConfig {
        data_api_url: "https://example.com".to_string(),
        api_key: "key".to_string(),
        database: "db".to_string(),
        collection: "col".to_string(),
        vector_field: "embedding".to_string(),
        id_field: "_id".to_string(),
        payload_fields: vec![],
        filter: None,
    }
}

#[test]
fn test_mongodb_config_defaults() {
    let json =
        r#"{"data_api_url":"https://test.com","api_key":"k","database":"d","collection":"c"}"#;
    let config: MongoDBConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.vector_field, "embedding");
    assert_eq!(config.id_field, "_id");
}

#[test]
fn test_mongodb_connector_new() {
    let connector = MongoDBConnector::new(test_config());
    assert_eq!(connector.source_type(), "mongodb");
}

#[test]
fn test_mongodb_build_url() {
    let mut config = test_config();
    config.data_api_url = "https://data.mongodb-api.com/app/test/endpoint/data/v1".to_string();
    let connector = MongoDBConnector::new(config);
    let url = connector.build_url("find");
    assert_eq!(
        url,
        "https://data.mongodb-api.com/app/test/endpoint/data/v1/action/find"
    );
}

#[test]
fn test_mongodb_parse_vector_success() {
    let connector = MongoDBConnector::new(test_config());
    let doc = serde_json::json!({"_id": "1", "embedding": [0.1, 0.2, 0.3]});
    let vector = connector.parse_vector(&doc).unwrap();
    assert_eq!(vector, vec![0.1, 0.2, 0.3]);
}

#[test]
fn test_mongodb_parse_vector_missing() {
    let connector = MongoDBConnector::new(test_config());
    let doc = serde_json::json!({"_id": "1"});
    assert!(connector.parse_vector(&doc).is_err());
}

#[test]
fn test_mongodb_extract_id_string() {
    let connector = MongoDBConnector::new(test_config());
    let doc = serde_json::json!({"_id": "my-id", "embedding": [0.1]});
    assert_eq!(connector.extract_id(&doc), "my-id");
}

#[test]
fn test_mongodb_extract_id_objectid() {
    let connector = MongoDBConnector::new(test_config());
    let doc = serde_json::json!({"_id": {"$oid": "507f1f77bcf86cd799439011"}});
    assert_eq!(connector.extract_id(&doc), "507f1f77bcf86cd799439011");
}

#[test]
fn test_mongodb_extract_payload() {
    let connector = MongoDBConnector::new(test_config());
    let doc = serde_json::json!({"_id": "1", "embedding": [0.1], "title": "Test", "count": 42});
    let payload = connector.extract_payload(&doc);
    assert_eq!(payload.len(), 2);
    assert!(!payload.contains_key("_id"));
    assert!(!payload.contains_key("embedding"));
}

#[test]
fn test_mongodb_extract_payload_filtered() {
    let mut config = test_config();
    config.payload_fields = vec!["title".to_string()];
    let connector = MongoDBConnector::new(config);
    let doc = serde_json::json!({"_id": "1", "embedding": [0.1], "title": "T", "count": 42});
    let payload = connector.extract_payload(&doc);
    assert_eq!(payload.len(), 1);
    assert!(payload.contains_key("title"));
}

#[test]
fn test_find_request_serialization() {
    let req = FindRequest {
        data_source: "atlas".to_string(),
        database: "db".to_string(),
        collection: "col".to_string(),
        filter: Some(serde_json::json!({"status": "active"})),
        projection: None,
        skip: Some(10),
        limit: Some(50),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["dataSource"], "atlas");
    assert_eq!(json["skip"], 10);
}

#[test]
fn test_aggregate_request_serialization() {
    let req = AggregateRequest {
        data_source: "atlas".to_string(),
        database: "db".to_string(),
        collection: "col".to_string(),
        pipeline: vec![serde_json::json!({"$count": "total"})],
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["pipeline"].as_array().unwrap().len(), 1);
}
