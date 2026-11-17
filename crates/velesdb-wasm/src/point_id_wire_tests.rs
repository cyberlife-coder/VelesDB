//! Point ids leave every search and read surface as their exact decimal
//! string, as REST sends them (`SearchResultResponse.id`, `/match`
//! `bindings`).
//!
//! The JSON-compatible serializer behind [`crate::js_value::to_js_value`]
//! throws on a `u64` past `Number.MAX_SAFE_INTEGER`, and a JSON number past
//! it is rounded by `JSON.parse`. Each scenario below stores a point under
//! 2^53 + 1 and reads it back through one surface family's native kernel:
//! the id must come back as `"9007199254740993"`, and any number beside it
//! (a score, a depth) must stay a number.

use serde_json::json;

use crate::database::DatabaseInner;
use crate::js_value::PointId;
use crate::store_insert::insert_with_payload;
use crate::store_state::StoreState;
use crate::velesql_exec::execute;

/// 2^53 + 1: the first integer an `f64` cannot hold.
const BIG: u64 = (1 << 53) + 1;
const BIG_TEXT: &str = "9007199254740993";

fn store_with_big_point() -> StoreState {
    let mut store = StoreState::new(2, "cosine").expect("test: store");
    insert_with_payload(
        &mut store,
        BIG,
        &[1.0, 0.0],
        Some(json!({"title": "hello rust"})),
    );
    store
}

#[test]
fn a_point_id_writes_its_decimal_string_and_reads_a_string_or_a_safe_number() {
    assert_eq!(
        serde_json::to_value(PointId(BIG)).expect("test: serialize"),
        json!(BIG_TEXT)
    );
    let read: Vec<(PointId, f32)> =
        serde_json::from_value(json!([[BIG_TEXT, 0.5], [7, 0.25]])).expect("test: deserialize");
    assert_eq!(read, vec![(PointId(BIG), 0.5), (PointId(7), 0.25)]);
    assert!(
        serde_json::from_value::<PointId>(json!(-1)).is_err(),
        "a negative id is refused"
    );
    assert!(
        serde_json::from_value::<PointId>(json!("12a")).is_err(),
        "a non-decimal string is refused"
    );
}

/// `search`, `search_with_quality`, `similarity_search`,
/// `multi_query_search` and `batch_search`: `[[id, score], …]`.
#[test]
fn dense_hits_carry_the_id_as_a_string_and_the_score_as_a_number() {
    let store = store_with_big_point();
    let hits = store.top_k(&[1.0, 0.0], 1).expect("test: search");
    let value = serde_json::to_value(crate::store_search::scored_points(hits)).expect("test");
    assert_eq!(value[0][0], json!(BIG_TEXT));
    assert!(value[0][1].is_number(), "the score stays a number: {value}");
}

/// `search_with_filter` and `hybrid_search`: `[{id, score, payload}, …]`.
#[test]
fn scored_records_carry_the_id_as_a_string() {
    let store = store_with_big_point();
    let rows = crate::store_search::scored_triples_json(crate::store_search::hybrid_search_scored(
        &[1.0, 0.0],
        "hello",
        &store.ids,
        &store.data,
        &store.payloads,
        store.dimension,
        store.metric,
        1,
        None,
    ));
    assert_eq!(rows[0]["id"], json!(BIG_TEXT));
    assert!(rows[0]["score"].is_number());
    assert_eq!(rows[0]["payload"]["title"], json!("hello rust"));
}

#[test]
fn text_search_records_carry_the_id_as_a_string() {
    let store = store_with_big_point();
    let rows =
        crate::store_search::text_search_matches("hello", &store.ids, &store.payloads, None, 5);
    assert_eq!(rows[0]["id"], json!(BIG_TEXT));
    assert!(rows[0]["score"].is_null());
}

#[test]
fn get_carries_the_id_as_a_string() {
    let store = store_with_big_point();
    let point = crate::store_get::point_json(&store, BIG).expect("test: the point is there");
    assert_eq!(point["id"], json!(BIG_TEXT));
    assert_eq!(point["vector"], json!([1.0, 0.0]));
    assert!(crate::store_get::point_json(&store, 1).is_none());
}

/// `SparseIndex.search`, `VectorStore.search_sparse` / `sparse_search`, and
/// `hybrid_search_fuse`, which also takes the ids back in either form.
#[test]
fn sparse_hits_carry_the_doc_id_as_a_string_and_fusion_takes_it_back() {
    let mut index = crate::sparse::SparseIndex::new();
    index
        .insert(BIG, &[10], &[1.0])
        .expect("test: sparse insert");
    let hits = index.search_scored(&[10], &[1.0], 5).expect("test: search");
    assert_eq!(hits[0].doc_id(), BIG);
    let value = serde_json::to_value(&hits).expect("test: serialize");
    assert_eq!(value[0]["doc_id"], json!(BIG_TEXT));

    let dense: Vec<(PointId, f32)> =
        serde_json::from_value(json!([[BIG_TEXT, 0.9], [3, 0.5]])).expect("test: dense");
    let sparse: Vec<(PointId, f32)> =
        serde_json::from_value(json!([[value[0]["doc_id"].clone(), 1.0]])).expect("test");
    let fused = crate::sparse::fuse_scored(dense, sparse, 60, 10).expect("test: fuse");
    let fused = serde_json::to_value(&fused).expect("test: serialize");
    assert_eq!(fused[0]["doc_id"], json!(BIG_TEXT), "{fused}");
    assert_eq!(fused[1]["doc_id"], json!("3"), "{fused}");
}

/// `VectorStore.query` and `SemanticMemory.query`.
#[test]
fn query_and_semantic_results_carry_the_id_as_a_string() {
    let result = crate::QueryResult {
        node_id: BIG,
        vector_score: Some(0.5),
        graph_score: None,
        fused_score: 0.5,
        bindings: serde_json::Value::Null,
        column_data: None,
    };
    let value = serde_json::to_value(&result).expect("test: serialize");
    assert_eq!(value["nodeId"], json!(BIG_TEXT));
    let back: crate::QueryResult = serde_json::from_value(value).expect("test: read back");
    assert_eq!(back.node_id, BIG);

    let fact = crate::agent::SemanticResult {
        id: BIG,
        score: 0.5,
        content: "fact".to_string(),
    };
    let value = serde_json::to_value(&fact).expect("test: serialize");
    assert_eq!(value["id"], json!(BIG_TEXT));
    let back: crate::agent::SemanticResult =
        serde_json::from_value(json!({"id": 7, "score": 0.5, "content": "f"})).expect("test");
    assert_eq!(back.id, 7, "a numeric id still reads");
}

/// `GraphStore.bfs_traverse` / `dfs_traverse`: `[[nodeId, depth], …]`.
#[test]
fn traversals_carry_the_node_id_as_a_string_and_the_depth_as_a_number() {
    let mut graph = crate::GraphStore::new();
    graph.add_node(crate::GraphNode::new(1, "A"));
    graph.add_node(crate::GraphNode::new(BIG, "B"));
    graph
        .add_edge(crate::GraphEdge::new(100, 1, BIG, "NEXT").expect("test: edge"))
        .expect("test: add edge");
    for rows in [graph.bfs(1, 2, 10), graph.dfs(1, 2, 10)] {
        assert_eq!(
            serde_json::to_value(rows).expect("test: serialize"),
            json!([[BIG_TEXT, 1]])
        );
    }
}

/// `WasmDatabase.executeMatch`: `bindings` as REST `/match` sends them;
/// `projected` keeps core's VelesQL row contract (a node id is a number).
#[test]
fn execute_match_bindings_carry_the_node_id_as_a_string() {
    let mut db = DatabaseInner::new();
    db.create_metadata_collection("docs")
        .expect("test: create docs");
    {
        let store = db.get_shared_store("docs").expect("test: docs store");
        for id in [1, BIG] {
            insert_with_payload(
                &mut store.borrow_mut(),
                id,
                &[],
                Some(json!({"_labels": ["Doc"]})),
            );
        }
    }
    execute(
        &mut db,
        &format!("INSERT EDGE INTO docs (id = 10, source = 1, target = {BIG}, label = 'CITES')"),
        None,
    )
    .expect("test: edge");
    let rows = crate::velesql_match_core::execute_match_rows(
        &mut db,
        "MATCH (a:Doc)-[:CITES]->(b:Doc) RETURN b.id",
        Some(r#"{"_collection": "docs"}"#),
        None,
    )
    .expect("test: MATCH");
    assert_eq!(rows[0]["bindings"], json!({"a": "1", "b": BIG_TEXT}));
    assert_eq!(rows[0]["projected"]["b.id"], json!(BIG));
}
