//! `VectorStore.multiQuerySearch` against the two other engines that fuse
//! one branch per query vector: core's `Collection::multi_query_search` and
//! the WASM executor's `NEAR_FUSED`. All three must read the same depth from
//! each branch, or a point ranked just past it by one vector and well inside
//! it by another drops out of one engine's fusion only.
//!
//! Host-only (`store_search.rs`): the core side needs core's `persistence`
//! feature.

use serde_json::json;
use velesdb_core::{Database, DistanceMetric as CoreMetric, FusionStrategy, Point};

use super::query_branches;
use crate::database::DatabaseInner;
use crate::velesql_exec::execute;
use crate::DistanceMetric;

/// Cosine to `[1, 0]` ranks 1, 4, 6, 3, 7, 5, 2; to `[0, 1]` the mirror
/// order 2, 5, 7, 3, 6, 4, 1. Point 3 (0.707 in both) has the best average
/// over the whole collection; cut each branch to its two best points and 3
/// is in neither, while 1 averages its one score (0.9988) and wins.
const POINTS: [(u64, [f32; 2]); 7] = [
    (1, [1.0, 0.05]),
    (2, [0.05, 1.0]),
    (3, [1.0, 1.0]),
    (4, [1.0, 0.3]),
    (5, [0.3, 1.0]),
    (6, [1.0, 0.6]),
    (7, [0.6, 1.0]),
];

const QUERIES: [[f32; 2]; 2] = [[1.0, 0.0], [0.0, 1.0]];

/// The ids `VectorStore.multiQuerySearch` returns for `k`, fused by `strategy`.
fn store_ids(k: usize, strategy: &str) -> Vec<u64> {
    let ids: Vec<u64> = POINTS.iter().map(|(id, _)| *id).collect();
    let data: Vec<f32> = POINTS.iter().flat_map(|(_, v)| *v).collect();
    let vectors: Vec<f32> = QUERIES.iter().flatten().copied().collect();
    let metric = DistanceMetric::Cosine;
    let branches =
        query_branches(&vectors, QUERIES.len(), &ids, &data, 2, metric, k).expect("test: branches");
    crate::fusion::fuse_results(&branches, strategy, 60, None, None, metric)
        .expect("test: fuse")
        .into_iter()
        .take(k)
        .map(|(id, _)| id)
        .collect()
}

/// The ids core's `Collection::multi_query_search` returns for `k`.
fn core_ids(k: usize, strategy: FusionStrategy) -> Vec<u64> {
    let dir = tempfile::TempDir::new().expect("test: temp dir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("c", 2, CoreMetric::Cosine)
        .expect("test: create");
    let collection = db.get_vector_collection("c").expect("test: collection");
    collection
        .upsert(
            POINTS
                .iter()
                .map(|(id, v)| Point::new(*id, v.to_vec(), None)),
        )
        .expect("test: upsert");
    let queries: Vec<&[f32]> = QUERIES.iter().map(<[f32; 2]>::as_slice).collect();
    collection
        .multi_query_search(&queries, k, strategy, None)
        .expect("test: core search")
        .iter()
        .map(|r| r.point.id)
        .collect()
}

/// The ids the WASM executor's `NEAR_FUSED` returns for `LIMIT k`.
fn near_fused_ids(k: usize, strategy: &str) -> Vec<u64> {
    let mut db = DatabaseInner::new();
    db.create_collection("c", 2, "cosine")
        .expect("test: create");
    for (id, vector) in POINTS {
        execute(
            &mut db,
            &format!("INSERT INTO c (id, vector) VALUES ({id}, $v)"),
            Some(&json!({ "v": vector }).to_string()),
        )
        .expect("test: insert");
    }
    let sql = format!(
        "SELECT * FROM c WHERE vector NEAR_FUSED [$a, $b] USING FUSION '{strategy}' LIMIT {k}"
    );
    let params = json!({ "a": QUERIES[0], "b": QUERIES[1] }).to_string();
    let result = execute(&mut db, &sql, Some(&params)).expect("test: NEAR_FUSED");
    (0..result.row_count() as usize)
        .map(|i| result.row(i).expect("test: row").id())
        .collect()
}

#[test]
fn multi_query_search_fuses_as_deep_a_branch_as_core_and_near_fused() {
    let core = core_ids(1, FusionStrategy::Average);
    assert_eq!(core, vec![3], "core's answer on this dataset");
    assert_eq!(near_fused_ids(1, "average"), core, "NEAR_FUSED");
    assert_eq!(
        store_ids(1, "average"),
        core,
        "VectorStore.multiQuerySearch"
    );
}

#[test]
fn each_branch_holds_cores_branch_depth() {
    let ids: Vec<u64> = POINTS.iter().map(|(id, _)| *id).collect();
    let data: Vec<f32> = POINTS.iter().flat_map(|(_, v)| *v).collect();
    let vectors: Vec<f32> = QUERIES.iter().flatten().copied().collect();
    for k in [1, 2, 3] {
        let branches =
            query_branches(&vectors, 2, &ids, &data, 2, DistanceMetric::Cosine, k).expect("test");
        let depth = velesdb_core::fusion::branch_depth(k).min(POINTS.len());
        assert!(
            branches.iter().all(|b| b.len() == depth),
            "k = {k}: branch lengths {:?}, core's depth {depth}",
            branches.iter().map(Vec::len).collect::<Vec<_>>()
        );
    }
}
