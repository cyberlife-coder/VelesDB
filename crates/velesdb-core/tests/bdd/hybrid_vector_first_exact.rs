//! BDD tests pinning the EXACT ordered result of vector-first hybrid queries
//! (NEAR + graph MATCH, with and without a scalar filter).
//!
//! `match_vector_first.rs` only asserts membership (`ids.contains(&n)`); these
//! tests lock the FULL ordered id list and the exact complement set, so any
//! regression in similarity ranking, graph anchoring, or scalar filtering is
//! caught — not just a dropped/added row.
//!
//! All ground truth is hand-computed from a controlled dataset: vectors are
//! `[1, off, 0, 0]` with strictly increasing `off`, so cosine similarity to the
//! query `[1, 0, 0, 0]` strictly DECREASES with `off`. The CITES graph and the
//! `category` payload are fully enumerated below, so the expected ordered ids
//! are deterministic.

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Builds the `bibliography` collection (4-dim, cosine).
///
/// Vectors are `[1, off, 0, 0]`; cosine similarity to the query `[1, 0, 0, 0]`
/// strictly DECREASES as `off` increases, so the full similarity order
/// (descending) of the candidate nodes is `1 > 2 > 3 > 4 > 5`, and the hub
/// (id 100, vector `[0, 0, 1, 0]`, cosine 0) is the least similar of all.
///
/// | id  | off | category | outgoing CITES |
/// |-----|-----|----------|----------------|
/// | 1   | 0.0 | physics  | yes (-> 100)   |
/// | 2   | 0.3 | physics  | no             |
/// | 3   | 0.7 | biology  | yes (-> 100)   |
/// | 4   | 1.2 | physics  | yes (-> 100)   |
/// | 5   | 3.0 | biology  | no             |
/// | 100 | hub | --       | no             |
///
/// Citing nodes = {1, 3, 4}; non-citing nodes = {2, 5, 100}.
fn setup_bibliography(db: &Database) {
    db.create_vector_collection("bibliography", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create bibliography collection");
    let vc = db
        .get_vector_collection("bibliography")
        .expect("test: get bibliography collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "physics"})),
        ),
        Point::new(
            2,
            vec![1.0, 0.3, 0.0, 0.0],
            Some(json!({"category": "physics"})),
        ),
        Point::new(
            3,
            vec![1.0, 0.7, 0.0, 0.0],
            Some(json!({"category": "biology"})),
        ),
        Point::new(
            4,
            vec![1.0, 1.2, 0.0, 0.0],
            Some(json!({"category": "physics"})),
        ),
        Point::new(
            5,
            vec![1.0, 3.0, 0.0, 0.0],
            Some(json!({"category": "biology"})),
        ),
        Point::new(
            100,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(json!({"category": "hub"})),
        ),
    ])
    .expect("test: upsert bibliography corpus");

    // Only nodes 1, 3, 4 cite the hub. 2 and 5 cite nothing.
    for (edge_id, source) in [(900u64, 1u64), (901, 3), (902, 4)] {
        let edge = GraphEdge::new(edge_id, source, 100, "CITES").expect("test: create CITES edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }
}

/// Collects the ordered ids of a result list (order is meaningful here because
/// the similarity ranking is unambiguous over well-separated offsets).
fn ordered_ids(results: &[velesdb_core::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

const QUERY: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

// =========================================================================
// A. NEAR + MATCH (vector-first hybrid) — exact ordered ids
// =========================================================================

/// GIVEN the bibliography corpus (citing nodes {1,3,4}, sim order 1>2>3>4>5)
/// WHEN `vector NEAR $v AND MATCH (a)-[:CITES]->(x)`
/// THEN exactly the citing nodes are returned, ordered by similarity DESC:
///      the off offsets 0.0 < 0.7 < 1.2 give cosine 1>3>4, so ids = [1, 3, 4].
#[test]
fn test_near_and_match_returns_exact_ordered_citing_ids() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT * FROM bibliography AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) LIMIT 10";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + MATCH must execute");

    assert_eq!(
        ordered_ids(&results),
        vec![1, 3, 4],
        "only citing nodes {{1,3,4}}, in strict similarity order"
    );
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score, "scores must be non-increasing");
    }
}

/// GIVEN the corpus, with the most similar citing node restricted by LIMIT
/// WHEN `vector NEAR $v AND MATCH (a)-[:CITES]->(x) LIMIT 1`
/// THEN only id 1 comes back (off 0.0 is the closest of {1,3,4} to the query).
#[test]
fn test_near_and_match_limit_one_keeps_most_similar_citer() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT * FROM bibliography AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) LIMIT 1";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + MATCH LIMIT 1 must execute");

    assert_eq!(ordered_ids(&results), vec![1], "best-similarity citer only");
}

/// GIVEN the corpus, ordered explicitly by similarity DESC with a prefix LIMIT
/// WHEN `... AND MATCH (a)-[:CITES]->(x) ORDER BY similarity() DESC LIMIT 2`
/// THEN exactly the top-2 citing nodes: ids [1, 3] (off 0.0 then 0.7; 4 drops).
#[test]
fn test_near_match_orderby_similarity_limit_keeps_exact_prefix() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT a.*, similarity() FROM bibliography AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) \
               ORDER BY similarity() DESC LIMIT 2";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + MATCH + ORDER BY similarity() DESC must execute");

    assert_eq!(
        ordered_ids(&results),
        vec![1, 3],
        "top-2 citing nodes by similarity, id 4 truncated by LIMIT"
    );
    assert!(
        results[0].score >= results[1].score,
        "ORDER BY similarity() DESC must hold"
    );
}

// =========================================================================
// B. NEAR + MATCH + scalar filter (triple hybrid) — exact ordered ids
// =========================================================================

/// GIVEN the corpus (physics citers = {1,4}; 3 is biology, 2 doesn't cite)
/// WHEN `vector NEAR $v AND MATCH (a)-[:CITES]->(x) AND category = 'physics'`
/// THEN exactly ids [1, 4] in similarity order (off 0.0 < 1.2 → 1 before 4).
#[test]
fn test_near_match_scalar_physics_returns_exact_ordered_ids() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT * FROM bibliography AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) \
               AND category = 'physics' LIMIT 10";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + MATCH + scalar must execute");

    assert_eq!(
        ordered_ids(&results),
        vec![1, 4],
        "physics citers {{1,4}} in similarity order; biology citer 3 excluded"
    );
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score, "scores must be non-increasing");
    }
}

/// GIVEN the corpus (biology citers = {3} only; 5 is biology but doesn't cite)
/// WHEN `... AND MATCH (a)-[:CITES]->(x) AND category = 'biology'`
/// THEN exactly [3]: it is the sole node that is BOTH a citer AND biology.
#[test]
fn test_near_match_scalar_biology_returns_single_id() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT * FROM bibliography AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) \
               AND category = 'biology' LIMIT 10";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + MATCH + biology scalar must execute");

    assert_eq!(
        ordered_ids(&results),
        vec![3],
        "id 3 is the only biology citer (5 is biology but cites nothing)"
    );
}

// =========================================================================
// C. NEAR + NOT MATCH — exact complement, by similarity
// =========================================================================

/// GIVEN the corpus (non-citing nodes = {2, 5, 100})
/// WHEN `vector NEAR $v AND NOT MATCH (a)-[:CITES]->(x)`
/// THEN exactly the non-citers, ordered by similarity DESC: id 2 (off 0.3) >
///      id 5 (off 3.0) > id 100 (cosine 0). This is the exact complement of
///      the citing set {1,3,4} from `test_near_and_match_*`.
#[test]
fn test_near_not_match_returns_exact_complement_by_similarity() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let sql = "SELECT * FROM bibliography AS a \
               WHERE vector NEAR $v AND NOT MATCH (a)-[:CITES]->(x) LIMIT 10";
    let results = execute_sql_with_params(&db, sql, &vector_param(&QUERY))
        .expect("NEAR + NOT MATCH must execute");

    assert_eq!(
        ordered_ids(&results),
        vec![2, 5, 100],
        "non-citing nodes in similarity order; complement of {{1,3,4}}"
    );
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score, "scores must be non-increasing");
    }
}

/// GIVEN the corpus
/// WHEN the positive and the negated MATCH are run with the same NEAR
/// THEN their id sets are disjoint and their union is the whole collection —
///      proving NOT MATCH is the exact complement of MATCH over the candidates.
#[test]
fn test_near_match_and_not_match_partition_the_collection() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let positive = execute_sql_with_params(
        &db,
        "SELECT * FROM bibliography AS a WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(x) LIMIT 10",
        &vector_param(&QUERY),
    )
    .expect("positive MATCH must execute");
    let negative = execute_sql_with_params(
        &db,
        "SELECT * FROM bibliography AS a WHERE vector NEAR $v AND NOT MATCH (a)-[:CITES]->(x) LIMIT 10",
        &vector_param(&QUERY),
    )
    .expect("NOT MATCH must execute");

    let pos = result_ids(&positive);
    let neg = result_ids(&negative);
    assert!(
        pos.is_disjoint(&neg),
        "MATCH and NOT MATCH must not overlap"
    );
    let union: std::collections::HashSet<u64> = pos.union(&neg).copied().collect();
    assert_eq!(
        union,
        [1u64, 2, 3, 4, 5, 100].into_iter().collect(),
        "MATCH | NOT MATCH must cover every node"
    );
}

// =========================================================================
// D. MATCH (and MATCH + scalar) WITHOUT NEAR — exact set
// =========================================================================

/// GIVEN the corpus (citing nodes = {1, 3, 4})
/// WHEN `MATCH (a)-[:CITES]->(x)` with NO NEAR (graph-only retrieval)
/// THEN the exact citing set {1, 3, 4} is returned (order is graph-scan driven,
///      so we assert the set, not the order).
#[test]
fn test_match_without_near_returns_exact_citing_set() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM bibliography AS a WHERE MATCH (a)-[:CITES]->(x) LIMIT 10",
    )
    .expect("MATCH without NEAR must execute");

    assert_eq!(
        result_ids(&results),
        [1u64, 3, 4].into_iter().collect(),
        "exactly the nodes with an outgoing CITES edge"
    );
}

/// GIVEN the corpus (physics citers = {1, 4})
/// WHEN `category = 'physics' AND MATCH (a)-[:CITES]->(x)` with NO NEAR
/// THEN the exact set {1, 4}: the intersection of physics and the citers.
#[test]
fn test_match_scalar_without_near_returns_exact_set() {
    let (_dir, db) = create_test_db();
    setup_bibliography(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM bibliography AS a \
         WHERE category = 'physics' AND MATCH (a)-[:CITES]->(x) LIMIT 10",
    )
    .expect("metadata + MATCH without NEAR must execute");

    assert_eq!(
        result_ids(&results),
        [1u64, 4].into_iter().collect(),
        "physics AND citing = {{1,4}} (3 is biology, 2 cites nothing)"
    );
}
