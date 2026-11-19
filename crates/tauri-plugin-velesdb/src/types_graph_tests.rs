//! Edge ids cross the IPC boundary as decimal strings: a derived edge id is
//! hashed and routinely above 2^53, which a JavaScript number rounds.

use serde_json::json;

use super::{
    AddEdgeRequest, AddEdgesBatchRequest, EdgeOutput, GetEdgesRequest, GetNodeDegreeRequest,
    NodeDegreeOutput, TraversalOutput, TraverseGraphParallelRequest, TraverseGraphRequest,
};

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
    assert_eq!(
        (&json["source"], &json["target"]),
        (&json!("1"), &json!("2")),
        "as REST's EdgeResponse"
    );
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
    assert_eq!(json["targetId"], "3", "as REST's TraversalResultItem");
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

/// Node ids cross the IPC boundary as REST's graph DTOs carry them: read as
/// a decimal string or a number, written as a decimal string, so a node id
/// past 2^53 is named exactly both ways.
#[test]
fn node_ids_are_read_as_strings_or_numbers() {
    let add: AddEdgeRequest = serde_json::from_value(json!({
        "collection": "g", "id": 1, "source": HASHED_STR, "target": 2, "label": "L"
    }))
    .expect("string source");
    assert_eq!((add.source, add.target), (HASHED, 2));

    let batch: AddEdgesBatchRequest = serde_json::from_value(json!({
        "collection": "g",
        "edges": [{"id": 1, "source": 1, "target": HASHED_STR, "label": "L"}]
    }))
    .expect("string target");
    assert_eq!(batch.edges[0].target, HASHED);

    let filter: GetEdgesRequest =
        serde_json::from_value(json!({"collection": "g", "source": HASHED_STR})).expect("filter");
    assert_eq!((filter.source, filter.target), (Some(HASHED), None));

    let traverse: TraverseGraphRequest =
        serde_json::from_value(json!({"collection": "g", "source": HASHED_STR})).expect("traverse");
    assert_eq!(traverse.source, HASHED);

    let degree: GetNodeDegreeRequest =
        serde_json::from_value(json!({"collection": "g", "nodeId": HASHED_STR})).expect("degree");
    assert_eq!(degree.node_id, HASHED);

    let parallel: TraverseGraphParallelRequest =
        serde_json::from_value(json!({"collection": "g", "sources": [HASHED_STR, 2]}))
            .expect("sources");
    assert_eq!(parallel.sources, [HASHED, 2]);
}

#[test]
fn a_node_degree_names_its_node_by_string() {
    let out = NodeDegreeOutput {
        node_id: HASHED,
        in_degree: 1,
        out_degree: 2,
    };
    assert_eq!(
        serde_json::to_value(&out).expect("json")["nodeId"],
        HASHED_STR
    );
}
