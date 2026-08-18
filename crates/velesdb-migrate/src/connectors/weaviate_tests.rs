use super::*;

#[test]
fn test_normalise_weaviate_metric_maps_l2_squared_to_euclidean() {
    // Weaviate reports 'l2-squared' for L2 distance; VelesDB core
    // uses 'euclidean'. The mapping is what allows
    // check_metric_fidelity to honestly compare a Weaviate source
    // against a core collection created with metric: "euclidean".
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("l2-squared"),
        "euclidean"
    );
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("L2-Squared"),
        "euclidean"
    );
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("l2_squared"),
        "euclidean"
    );
}

#[test]
fn test_normalise_weaviate_metric_lowercases_known_values() {
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("Cosine"),
        "cosine"
    );
    assert_eq!(WeaviateConnector::normalise_weaviate_metric("Dot"), "dot");
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("Hamming"),
        "hamming"
    );
}

#[test]
fn test_normalise_weaviate_metric_preserves_unknown_values() {
    // Manhattan is a valid Weaviate metric but not supported by
    // VelesDB core — preserved verbatim so mismatch errors are
    // actionable rather than masked.
    assert_eq!(
        WeaviateConnector::normalise_weaviate_metric("manhattan"),
        "manhattan"
    );
}

#[test]
fn test_weaviate_connector_new() {
    let config = WeaviateConfig {
        url: "http://localhost:8080".to_string(),
        class_name: "Document".to_string(),
        api_key: None,
        properties: vec!["title".to_string()],
    };

    let connector = WeaviateConnector::new(config);
    assert_eq!(connector.source_type(), "weaviate");
}

#[test]
fn test_graphql_query_serialization() {
    let query = GraphQLQuery {
        query: "{ Get { Document { title } } }".to_string(),
    };

    let json = serde_json::to_string(&query).unwrap();
    assert!(json.contains("Get"));
}

#[test]
fn test_connect_rejects_file_url() {
    assert!(crate::connectors::common::validate_url("file:///etc/passwd").is_err());
}
