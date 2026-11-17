//! `IndexedDB` persistence in a real browser (headless Chrome): what a graph
//! save leaves behind, which graph a name addresses, and when a save is
//! durable.
//!
//! Run with `wasm-pack test --headless --chrome` (or the
//! `wasm-bindgen-test-runner` with `CHROMEDRIVER` set).

#![cfg(target_arch = "wasm32")]

use velesdb_wasm::{GraphEdge, GraphNode, GraphPersistence, GraphStore};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn graph(nodes: &[u64], edges: &[(u64, u64, u64)]) -> GraphStore {
    let mut g = GraphStore::new();
    for &id in nodes {
        g.add_node(GraphNode::new(id, "N"));
    }
    for &(id, source, target) in edges {
        g.add_edge(GraphEdge::new(id, source, target, "E").expect("edge"))
            .expect("add edge");
    }
    g
}

async fn persistence() -> GraphPersistence {
    let mut p = GraphPersistence::new();
    p.init().await.expect("init");
    p
}

/// Saving a graph replaces what was saved under its name: a node or edge
/// removed since the last save does not come back on load.
#[wasm_bindgen_test]
async fn a_save_replaces_the_previous_save() {
    let p = persistence().await;
    p.save("replace", &graph(&[1, 2, 3], &[(10, 1, 2), (11, 2, 3)]))
        .await
        .expect("first save");
    p.save("replace", &graph(&[1, 2], &[(10, 1, 2)]))
        .await
        .expect("second save");

    let loaded = p.load("replace").await.expect("load");
    assert_eq!(
        loaded.node_count(),
        2,
        "node 3 was removed before the second save"
    );
    assert_eq!(
        loaded.edge_count(),
        1,
        "edge 11 was removed before the second save"
    );
    p.delete_graph("replace").await.expect("cleanup");
}

/// A graph name addresses that graph only: graph `a` neither loads nor
/// deletes the rows of graph `a:b`, whose keys sort inside `a`'s range.
#[wasm_bindgen_test]
async fn a_graph_name_is_not_a_prefix_of_another() {
    let p = persistence().await;
    p.save("a", &graph(&[1], &[])).await.expect("save a");
    p.save("a:b", &graph(&[2, 3], &[])).await.expect("save a:b");

    assert_eq!(p.load("a").await.expect("load a").node_count(), 1);
    p.delete_graph("a").await.expect("delete a");
    assert_eq!(p.load("a:b").await.expect("load a:b").node_count(), 2);
    p.delete_graph("a:b").await.expect("cleanup");
}

/// A save that fails leaves the previous save intact: a node id past 2^53
/// cannot cross to JS, and that failure used to come after the previous
/// rows were deleted — the transaction then committed the deletion alone.
#[wasm_bindgen_test]
async fn a_failed_save_leaves_the_previous_save_intact() {
    let p = persistence().await;
    p.save("intact", &graph(&[1, 2], &[(10, 1, 2)]))
        .await
        .expect("first save");

    let unsafe_id = 1_u64 << 53;
    assert!(
        p.save("intact", &graph(&[1, unsafe_id], &[]))
            .await
            .is_err(),
        "an id past 2^53 cannot be saved"
    );

    let loaded = p.load("intact").await.expect("load");
    assert_eq!(loaded.node_count(), 2, "the first save's nodes survive");
    assert_eq!(loaded.edge_count(), 1, "the first save's edge survives");
    p.delete_graph("intact").await.expect("cleanup");
}

/// A derived edge id is hashed and routinely above 2^53: the save writes it
/// as its decimal string (a JSON number threw there) and the load reads the
/// same id back.
#[wasm_bindgen_test]
async fn a_hashed_edge_id_survives_a_save_and_a_load() {
    let hashed = (1_u64 << 53) + 1;
    let p = persistence().await;
    p.save("hashed", &graph(&[1, 2], &[(hashed, 1, 2)]))
        .await
        .expect("a hashed edge id saves");
    let loaded = p.load("hashed").await.expect("load");
    assert!(loaded.has_edge(hashed), "the exact id comes back");
    p.delete_graph("hashed").await.expect("cleanup");
}
