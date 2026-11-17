//! Edge ids cross the IPC boundary as decimal strings: a derived edge id is
//! hashed and routinely above 2^53, which a JavaScript number rounds.

use serde_json::json;

use super::{AddEdgeRequest, AddEdgesBatchRequest, EdgeOutput, TraversalOutput};

/// 2^53 + 1: the first integer a JavaScript number cannot hold.
const HASHED: u64 = (1_u64 << 53) + 1;
const HASHED_STR: &str = "9007199254740993";

#[test]
fn an_edge_is_answered_with_its_exact_string_id() {
    let edge = EdgeOutput {
        id: HASHED,
        source: 1,
        target: 2,
        label: "KNOWS".to_string(),
        properties: json!({}),
    };
    let json = serde_json::to_value(&edge).expect("json");
    assert_eq!(json["id"], HASHED_STR);
    assert_eq!(json["source"], 1, "node ids stay numbers");
}

#[test]
fn a_traversal_path_lists_edge_ids_as_strings() {
    let out = TraversalOutput {
        target_id: 3,
        depth: 2,
        path: vec![HASHED, 4],
    };
    let json = serde_json::to_value(&out).expect("json");
    assert_eq!(json["path"], json!([HASHED_STR, "4"]));
    assert_eq!(json["targetId"], 3);
}

#[test]
fn an_edge_id_is_sent_as_a_string_or_a_number() {
    let request: AddEdgeRequest = serde_json::from_value(json!({
        "collection": "g", "id": HASHED_STR, "source": 1, "target": 2, "label": "L"
    }))
    .expect("a string id");
    assert_eq!(request.id, HASHED);
    let batch: AddEdgesBatchRequest = serde_json::from_value(json!({
        "collection": "g",
        "edges": [
            {"id": HASHED_STR, "source": 1, "target": 2, "label": "L"},
            {"id": 7, "source": 1, "target": 2, "label": "L"},
        ]
    }))
    .expect("string and number ids");
    assert_eq!(
        batch.edges.iter().map(|e| e.id).collect::<Vec<_>>(),
        [HASHED, 7]
    );
}
