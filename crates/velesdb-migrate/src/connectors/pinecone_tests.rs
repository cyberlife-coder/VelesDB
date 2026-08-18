use super::*;

#[test]
#[allow(deprecated)]
fn test_pinecone_connector_new() {
    let config = PineconeConfig {
        api_key: "test-key".to_string(),
        environment: "us-east-1".to_string(),
        index: "test-index".to_string(),
        namespace: None,
        base_url: None,
    };

    let connector = PineconeConnector::new(config);
    assert_eq!(connector.source_type(), "pinecone");
    assert!(connector.host.is_none());
    assert!(connector.metric.is_none());
}

#[test]
fn test_normalise_pinecone_metric_maps_dotproduct_to_dot() {
    // Pinecone reports `dotproduct` for inner-product indexes but
    // VelesDB core identifies the same metric as `dot`. The
    // normalisation layer is what makes
    // `Pipeline::check_metric_fidelity` able to match Pinecone
    // sources against a destination collection created with
    // `metric: "dot"`.
    assert_eq!(
        PineconeConnector::normalise_pinecone_metric("dotproduct"),
        "dot"
    );
    assert_eq!(
        PineconeConnector::normalise_pinecone_metric("DotProduct"),
        "dot"
    );
}

#[test]
fn test_normalise_pinecone_metric_lowercases_known_values() {
    // Already-compatible values are lowercased so
    // `check_metric_fidelity` compares them against the core
    // identifier vocabulary verbatim without surprise.
    assert_eq!(
        PineconeConnector::normalise_pinecone_metric("Cosine"),
        "cosine"
    );
    assert_eq!(
        PineconeConnector::normalise_pinecone_metric("EUCLIDEAN"),
        "euclidean"
    );
}

#[test]
fn test_normalise_pinecone_metric_preserves_unknown_values() {
    // Unknown metric names are preserved (lowercased) rather than
    // silently normalised to `None` — this keeps mismatch errors
    // actionable instead of masking them behind a no-op schema.
    assert_eq!(
        PineconeConnector::normalise_pinecone_metric("manhattan"),
        "manhattan"
    );
}

#[test]
fn test_list_query_params_construction() {
    let batch_size: usize = 100;
    let pagination_token: Option<String> = Some("tok-abc".to_string());
    let namespace: Option<String> = Some("ns1".to_string());

    let mut params: Vec<(&str, String)> = vec![("limit", batch_size.to_string())];
    if let Some(ref token) = pagination_token {
        params.push(("paginationToken", token.clone()));
    }
    if let Some(ref ns) = namespace {
        params.push(("namespace", ns.clone()));
    }

    assert_eq!(params.len(), 3);
    assert_eq!(params[0], ("limit", "100".to_string()));
    assert_eq!(params[1], ("paginationToken", "tok-abc".to_string()));
    assert_eq!(params[2], ("namespace", "ns1".to_string()));
}

#[test]
fn test_list_query_params_without_optional_fields() {
    let batch_size: usize = 50;
    let pagination_token: Option<String> = None;
    let namespace: Option<String> = None;

    let mut params: Vec<(&str, String)> = vec![("limit", batch_size.to_string())];
    if let Some(ref token) = pagination_token {
        params.push(("paginationToken", token.clone()));
    }
    if let Some(ref ns) = namespace {
        params.push(("namespace", ns.clone()));
    }

    assert_eq!(params.len(), 1);
    assert_eq!(params[0], ("limit", "50".to_string()));
}

#[test]
fn test_fetch_query_params_with_namespace() {
    let ids = ["id-1".to_string(), "id-2".to_string(), "id-3".to_string()];
    let namespace: Option<String> = Some("ns1".to_string());

    let mut fetch_params: Vec<(&str, String)> = ids.iter().map(|id| ("ids", id.clone())).collect();
    if let Some(ref ns) = namespace {
        fetch_params.push(("namespace", ns.clone()));
    }

    assert_eq!(fetch_params.len(), 4);
    assert_eq!(fetch_params[0], ("ids", "id-1".to_string()));
    assert_eq!(fetch_params[1], ("ids", "id-2".to_string()));
    assert_eq!(fetch_params[2], ("ids", "id-3".to_string()));
    assert_eq!(fetch_params[3], ("namespace", "ns1".to_string()));
}

#[test]
fn test_pinecone_vector_with_sparse() {
    let json = r#"{
        "id": "vec-1",
        "values": [0.1, 0.2],
        "sparseValues": {
            "indices": [0, 5, 11],
            "values": [0.5, 0.3, 0.8]
        }
    }"#;

    let v: PineconeVector = serde_json::from_str(json).unwrap();
    assert_eq!(v.id, "vec-1");
    assert_eq!(v.values, vec![0.1, 0.2]);

    let sv = v.sparse_values.expect("sparse_values should be present");
    assert_eq!(sv.indices, vec![0, 5, 11]);
    assert_eq!(sv.values, vec![0.5, 0.3, 0.8]);
}

#[test]
fn test_pinecone_vector_without_sparse() {
    let json = r#"{
        "id": "vec-2",
        "values": [0.4, 0.5]
    }"#;

    let v: PineconeVector = serde_json::from_str(json).unwrap();
    assert_eq!(v.id, "vec-2");
    assert!(v.sparse_values.is_none());
}

#[test]
fn test_pinecone_sparse_extraction_in_point() {
    let v = PineconeVector {
        id: "vec-3".to_string(),
        values: vec![1.0, 2.0, 3.0],
        metadata: None,
        sparse_values: Some(PineconeSparseValues {
            indices: vec![2, 7],
            values: vec![0.9, 0.1],
        }),
    };

    let sparse = v.sparse_values.and_then(|sv| {
        if crate::connectors::common::is_valid_sparse_vector(&sv.indices, &sv.values) {
            Some(sv.indices.into_iter().zip(sv.values).collect::<Vec<_>>())
        } else {
            None
        }
    });

    let point = ExtractedPoint {
        id: v.id,
        vector: v.values,
        payload: v.metadata.unwrap_or_default(),
        sparse_vector: sparse,
    };

    assert_eq!(point.id, "vec-3");
    let sv = point.sparse_vector.expect("should have sparse vector");
    assert_eq!(sv, vec![(2, 0.9), (7, 0.1)]);
}
