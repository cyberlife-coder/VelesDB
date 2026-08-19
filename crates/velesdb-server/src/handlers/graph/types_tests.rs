use super::*;

/// Path edge IDs above 2^53 must serialize as a JSON string array.
#[test]
fn test_traversal_path_serialized_as_strings() {
    let above_safe = (1_u64 << 53) + 1; // 9_007_199_254_740_993
    let item = TraversalResultItem {
        target_id: 2,
        depth: 1,
        path: vec![above_safe],
    };
    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["path"], serde_json::json!(["9007199254740993"]));
}

/// Streamed node path edge IDs above 2^53 must serialize as strings.
#[test]
fn test_stream_node_event_path_serialized_as_strings() {
    let above_safe = (1_u64 << 53) + 1;
    let event = StreamNodeEvent {
        id: 1,
        depth: 1,
        path: vec![above_safe],
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["path"], serde_json::json!(["9007199254740993"]));
}

/// Node-list ids above 2^53 must serialize as a JSON string array.
#[test]
fn test_node_list_ids_serialized_as_strings() {
    let above_safe = (1_u64 << 53) + 1;
    let response = NodeListResponse {
        node_ids: vec![1, above_safe],
        count: 2,
    };
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(
        json["node_ids"],
        serde_json::json!(["1", "9007199254740993"])
    );
}

/// Parallel-traverse sources must deserialize from BOTH strings and numbers.
#[test]
fn test_parallel_sources_accepts_strings_and_numbers() {
    let from_strings: ParallelTraverseRequest =
        serde_json::from_value(serde_json::json!({ "sources": ["9007199254740993", "2"] }))
            .expect("string sources must deserialize");
    assert_eq!(from_strings.sources, vec![(1_u64 << 53) + 1, 2]);

    let from_numbers: ParallelTraverseRequest =
        serde_json::from_value(serde_json::json!({ "sources": [3, 4] }))
            .expect("numeric sources must still deserialize");
    assert_eq!(from_numbers.sources, vec![3, 4]);
}
