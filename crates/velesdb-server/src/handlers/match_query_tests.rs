use super::*;

#[test]
fn test_match_query_request_deserialize() {
    let json = r#"{
        "query": "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name",
        "params": {}
    }"#;

    let request: MatchQueryRequest = serde_json::from_str(json).unwrap();
    assert!(request.query.contains("MATCH"));
    assert!(request.params.is_empty());
}

#[test]
fn test_match_query_response_serialize() {
    let response = MatchQueryResponse {
        results: vec![MatchQueryResultItem {
            bindings: HashMap::from([("a".to_string(), 123)]),
            score: Some(0.95),
            depth: 1,
            projected: HashMap::new(),
        }],
        took_ms: 15,
        count: 1,
        meta: MatchQueryMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("bindings"));
    assert!(json.contains("0.95"));
}

#[test]
fn test_match_query_bindings_serialized_as_strings() {
    let above_safe = (1_u64 << 53) + 1; // 9_007_199_254_740_993
    let response = MatchQueryResponse {
        results: vec![MatchQueryResultItem {
            bindings: HashMap::from([("a".to_string(), above_safe)]),
            score: None,
            depth: 0,
            projected: HashMap::new(),
        }],
        took_ms: 0,
        count: 1,
        meta: MatchQueryMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
        },
    };

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(
        json["results"][0]["bindings"]["a"],
        serde_json::json!("9007199254740993"),
        "binding IDs must serialize as JSON strings for JS precision safety"
    );
}

#[test]
fn test_match_query_response_with_projected_properties() {
    let mut projected = HashMap::new();
    projected.insert("author.name".to_string(), serde_json::json!("John Doe"));

    let response = MatchQueryResponse {
        results: vec![MatchQueryResultItem {
            bindings: HashMap::from([("author".to_string(), 42)]),
            score: Some(0.92),
            depth: 1,
            projected,
        }],
        took_ms: 10,
        count: 1,
        meta: MatchQueryMeta {
            velesql_contract_version: VELESQL_CONTRACT_VERSION.to_string(),
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("John Doe"));
    assert!(json.contains("author.name"));
}

/// Regression (parity backlog #1): the graph REST `/match` handler must honor
/// `RETURN ... ORDER BY`, matching the SQL `/query` pipeline. This exercises
/// the exact handler path (`parse_match_clause` -> `execute_match`) that
/// previously bypassed the ordering finalize step and returned raw traversal
/// order. Ages are scrambled vs id order so traversal order != requested
/// age-descending order.
#[test]
fn test_match_handler_applies_return_order_by() {
    use velesdb_core::collection::VectorCollection;
    use velesdb_core::{DistanceMetric, Point, StorageMode};

    let temp = tempfile::tempdir().expect("temp dir");
    let coll = VectorCollection::create(
        temp.path().to_path_buf(),
        "people",
        4,
        DistanceMetric::Cosine,
        StorageMode::default(),
    )
    .expect("create collection");

    let ages = [(1_u64, 30), (2, 10), (3, 50), (4, 20), (5, 40)];
    let points: Vec<Point> = ages
        .iter()
        .map(|(id, age)| {
            Point::new(
                *id,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(serde_json::json!({"_labels": ["Person"], "age": age})),
            )
        })
        .collect();
    coll.upsert(points).expect("upsert Person nodes");

    let collection = MatchCollection::Vector(coll);
    let request = MatchQueryRequest {
        query: "MATCH (n:Person) RETURN n ORDER BY n.age DESC LIMIT 10".to_string(),
        params: HashMap::new(),
        vector: None,
        threshold: None,
    };
    let clause = parse_match_clause(&request.query).expect("parse MATCH clause");
    let results = execute_match(&collection, &clause, &request).expect("execute_match");

    let ids: Vec<u64> = results
        .iter()
        .map(|r| *r.bindings.get("n").expect("binding 'n'"))
        .collect();
    assert_eq!(
        ids,
        vec![3, 5, 1, 4, 2],
        "/match must honor RETURN ORDER BY n.age DESC (ages 50,40,30,20,10)"
    );
}
