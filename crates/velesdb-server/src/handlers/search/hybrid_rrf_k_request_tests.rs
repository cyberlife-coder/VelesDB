use crate::types::HybridSearchRequestWithRrf;

#[test]
fn rrf_k_deserializes_when_present() {
    let body: HybridSearchRequestWithRrf = serde_json::from_value(serde_json::json!({
        "vector": [0.5, 0.6],
        "query": "vector database",
        "rrf_k": 30
    }))
    .expect("test: a body carrying rrf_k must deserialize");
    assert_eq!(body.rrf_k, Some(30));
    assert_eq!(body.base.top_k, 10);
}

#[test]
fn rrf_k_defaults_to_none_when_omitted() {
    let body: HybridSearchRequestWithRrf = serde_json::from_value(serde_json::json!({
        "vector": [0.5, 0.6],
        "query": "vector database"
    }))
    .expect("test: a body without rrf_k must deserialize");
    assert_eq!(body.rrf_k, None);
}
