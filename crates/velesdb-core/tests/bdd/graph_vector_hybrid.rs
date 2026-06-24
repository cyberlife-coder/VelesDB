//! BDD tests for hybrid SELECT queries combining vector NEAR, graph MATCH
//! predicates, and scalar filters in a single WHERE clause.
//!
//! Regression coverage for the production panic where graph predicates forced
//! `execution_limit = MAX_LIMIT` (100k) and the downstream oversampling clamp
//! hit `f64::clamp` with `min > max` ("triple hybrid" showcase query), and for
//! the V011 anchor rule (explicit binding, implicit binding with guards
//! G1/G2/G3, and the G1 validation error).
//!
//! All tests exercise the full pipeline: SQL string -> parse -> validate ->
//! execute -> verify.

use serde_json::json;
use velesdb_core::{Database, GraphEdge, Point};

use super::helpers::{
    create_test_db, execute_sql, execute_sql_with_params, result_ids, vector_param,
};

// =========================================================================
// Module-specific setup
// =========================================================================

/// Creates an "articles" collection mixing vectors, payloads, and graph edges.
///
/// Graph topology (CITES):
/// ```text
///   (1)--[:CITES]-->(2)
///   (3)--[:CITES]-->(2)
///   (4)--[:CITES]-->(2)
///   (5)--[:CITES]-->(2)
///   (2) has no outgoing edge
/// ```
///
/// Vectors (4-dim, cosine), query is `[1, 0, 0, 0]`:
///
/// | id | vector            | category | has outgoing CITES |
/// |----|-------------------|----------|--------------------|
/// | 1  | `[1.0,0,0,0]`     | science  | yes                |
/// | 2  | `[0.9,0.1,0,0]`   | science  | no                 |
/// | 3  | `[0.85,0.15,0,0]` | science  | yes                |
/// | 4  | `[0.8,0.2,0,0]`   | tech     | yes                |
/// | 5  | `[0.75,0.25,0,0]` | science  | yes                |
fn setup_articles_with_edges(db: &Database) {
    db.create_vector_collection("articles", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create articles collection");
    let vc = db
        .get_vector_collection("articles")
        .expect("test: get articles collection");

    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Quantum"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Chemistry"})),
        ),
        Point::new(
            3,
            vec![0.85, 0.15, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Biology"})),
        ),
        Point::new(
            4,
            vec![0.8, 0.2, 0.0, 0.0],
            Some(json!({"category": "tech", "title": "Rust"})),
        ),
        Point::new(
            5,
            vec![0.75, 0.25, 0.0, 0.0],
            Some(json!({"category": "science", "title": "Geology"})),
        ),
    ])
    .expect("test: upsert articles corpus");

    for (edge_id, source) in [(100u64, 1u64), (101, 3), (102, 4), (103, 5)] {
        let edge = GraphEdge::new(edge_id, source, 2, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }
}

// =========================================================================
// A. Nominal: triple hybrid (NEAR + graph MATCH + scalar) must not panic
// =========================================================================

/// GIVEN articles with vectors, categories, and CITES edges
/// WHEN running the showcase triple-hybrid query
///      `SELECT a.*, similarity() ... WHERE vector NEAR $v
///       AND MATCH (a)-[:CITES]->(r) AND category = 'science'
///       ORDER BY similarity() DESC LIMIT 2`
/// THEN it returns exactly the top-2 similarity-ordered nodes that satisfy
///      BOTH the graph predicate and the scalar filter (no panic, LIMIT kept).
#[test]
fn test_near_graph_match_scalar_orderby_similarity_respects_limit() {
    let (_dir, db) = create_test_db();
    setup_articles_with_edges(&db);

    let sql = "SELECT a.*, similarity() FROM articles AS a \
               WHERE vector NEAR $v AND MATCH (a)-[:CITES]->(r) AND category = 'science' \
               ORDER BY similarity() DESC LIMIT 2";
    let results = execute_sql_with_params(&db, sql, &vector_param(&[1.0, 0.0, 0.0, 0.0]))
        .expect("triple hybrid NEAR + MATCH + scalar must not fail");

    // Candidates passing graph + scalar filters: 1, 3, 5 (2 lacks an outgoing
    // edge, 4 is tech). LIMIT 2 keeps the two most similar: 1 then 3.
    assert_eq!(results.len(), 2, "LIMIT 2 must be respected");
    assert_eq!(results[0].point.id, 1, "highest similarity first");
    assert_eq!(results[1].point.id, 3, "second highest similarity");
    assert!(
        results[0].score >= results[1].score,
        "ORDER BY similarity() DESC must hold"
    );
}

/// GIVEN the same hybrid corpus
/// WHEN the graph predicate anchors on the FROM table without an alias
///      (`FROM articles WHERE MATCH (a)-[:CITES]->(r)`)
/// THEN the query still executes (anchor check only applies when FROM/JOIN
///      aliases are declared) and returns only nodes with outgoing edges.
#[test]
fn test_graph_match_without_from_alias_still_executes() {
    let (_dir, db) = create_test_db();
    setup_articles_with_edges(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM articles WHERE MATCH (a)-[:CITES]->(r) LIMIT 10",
    )
    .expect("MATCH anchor on unaliased FROM must keep working");

    let ids: std::collections::HashSet<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        [1u64, 3, 4, 5].into_iter().collect(),
        "only nodes with an outgoing CITES edge match"
    );
}

/// GIVEN documents with vectors and CITES edges
/// WHEN running README showcase query #2 verbatim — bare FROM alias, no AS:
///      `SELECT doc.*, similarity() FROM documents doc
///       WHERE vector NEAR $query AND MATCH (doc)-[:CITES]->(ref)
///       ORDER BY similarity() DESC`
/// THEN it parses, validates (V011 anchor = bare alias `doc`), executes, and
///      returns exactly the citing documents ordered by similarity DESC —
///      identical to the `FROM documents AS doc` form.
#[test]
fn test_showcase_bare_from_alias_near_match_executes() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("documents", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create documents collection");
    let vc = db
        .get_vector_collection("documents")
        .expect("test: get documents collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"title": "A"}))),
        Point::new(2, vec![0.9, 0.1, 0.0, 0.0], Some(json!({"title": "B"}))),
        Point::new(3, vec![0.8, 0.2, 0.0, 0.0], Some(json!({"title": "C"}))),
        Point::new(4, vec![0.7, 0.3, 0.0, 0.0], Some(json!({"title": "D"}))),
    ])
    .expect("test: upsert documents");
    // 1 and 3 cite 2; 2 and 4 cite nothing.
    for (edge_id, source) in [(300u64, 1u64), (301, 3)] {
        let edge = GraphEdge::new(edge_id, source, 2, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }

    let bare = "SELECT doc.*, similarity() FROM documents doc \
                WHERE vector NEAR $query AND MATCH (doc)-[:CITES]->(ref) \
                ORDER BY similarity() DESC";
    let mut params = std::collections::HashMap::new();
    params.insert(
        "query".to_string(),
        json!([1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32]),
    );

    let results =
        execute_sql_with_params(&db, bare, &params).expect("showcase query #2 must execute");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 3], "citing docs only, similarity DESC order");
    assert!(
        results[0].score >= results[1].score,
        "ORDER BY similarity() DESC must hold"
    );

    // Strict equivalence with the AS form.
    let with_as = bare.replace("FROM documents doc", "FROM documents AS doc");
    let as_results = execute_sql_with_params(&db, &with_as, &params).expect("AS form must execute");
    let as_ids: Vec<u64> = as_results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, as_ids, "bare alias must behave exactly like AS alias");
}

/// Creates a "documents" collection of 20 docs with decreasing similarity to
/// `[1, 0, 0, 0]`, where ids 1..=16 cite id 20 (ids 17..=20 cite nothing).
fn setup_documents_with_16_citing(db: &Database) {
    db.create_vector_collection("documents", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create documents collection");
    let vc = db
        .get_vector_collection("documents")
        .expect("test: get documents collection");
    let points: Vec<Point> = (1..=20u8)
        .map(|i| {
            Point::new(
                u64::from(i),
                vec![1.0, f32::from(i) * 0.01, 0.0, 0.0],
                Some(json!({"title": format!("doc-{i}")})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert documents");
    for (edge_id, source) in (400u64..).zip(1u64..=16) {
        let edge = GraphEdge::new(edge_id, source, 20, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }
}

/// GIVEN 20 documents of which 16 cite another document
/// WHEN running showcase query #2 verbatim WITHOUT a LIMIT clause
/// THEN exactly 10 rows come back (engine default LIMIT 10), and with an
///      explicit `LIMIT 15` all 15 best citing docs come back — proving the
///      MATCH anchor set is exhaustive and nothing was lost upstream of the
///      final truncation.
#[test]
fn test_showcase_near_match_without_limit_defaults_to_10() {
    let (_dir, db) = create_test_db();
    setup_documents_with_16_citing(&db);

    let sql = "SELECT doc.*, similarity() FROM documents doc \
               WHERE vector NEAR $query AND MATCH (doc)-[:CITES]->(ref) \
               ORDER BY similarity() DESC";
    let mut params = std::collections::HashMap::new();
    params.insert(
        "query".to_string(),
        json!([1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32]),
    );

    let results =
        execute_sql_with_params(&db, sql, &params).expect("showcase query #2 must execute");
    assert_eq!(
        results.len(),
        10,
        "no LIMIT clause: engine default LIMIT 10 must apply"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        (1..=10u64).collect::<Vec<_>>(),
        "the 10 most similar citing docs must be kept, in similarity order"
    );

    let with_limit = format!("{sql} LIMIT 15");
    let results = execute_sql_with_params(&db, &with_limit, &params)
        .expect("showcase query #2 with LIMIT 15 must execute");
    assert_eq!(
        results.len(),
        15,
        "LIMIT 15 must be filled: MATCH anchors are exhaustive, no upstream loss"
    );
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        (1..=15u64).collect::<Vec<_>>(),
        "rows 11..=15 must be the next most similar citing docs"
    );
}

// =========================================================================
// B. Completeness: graph MATCH without NEAR must scan past the ranked
//    over-fetch window (regression: graph_overfetch_limit applied to
//    unranked scans made results depend on insertion order)
// =========================================================================

/// Creates a "library" collection of 2000 points where ONLY the last 50
/// inserted ids (1951..=2000) carry an outgoing REFS edge (all pointing at
/// id 1). Every point shares `category = "common"` so a metadata filter
/// alone does not narrow the candidate set.
fn setup_large_collection_with_late_edges(db: &Database) {
    db.create_vector_collection("library", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create library collection");
    let vc = db
        .get_vector_collection("library")
        .expect("test: get library collection");

    let points: Vec<Point> = (1..=2000u16)
        .map(|i| {
            Point::new(
                u64::from(i),
                vec![1.0, f32::from(i) * 0.001, 0.0, 0.0],
                Some(json!({"category": "common"})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert library corpus");

    for (edge_id, source) in (10_000u64..).zip(1951u64..=2000) {
        let edge = GraphEdge::new(edge_id, source, 1, "REFS").expect("test: create edge");
        vc.add_edge(edge).expect("test: add REFS edge");
    }
}

/// GIVEN 2000 points where only the 50 last-inserted ids have outgoing edges
/// WHEN running `SELECT * WHERE MATCH (a)-[:REFS]->(b) LIMIT 10` (no NEAR)
/// THEN 10 rows are returned — the unranked scan must not stop at the
///      ranked over-fetch window (100 candidates for LIMIT 10) and silently
///      drop every match because of insertion order.
#[test]
fn test_graph_match_without_near_scans_beyond_overfetch_window() {
    let (_dir, db) = create_test_db();
    setup_large_collection_with_late_edges(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM library WHERE MATCH (a)-[:REFS]->(b) LIMIT 10",
    )
    .expect("graph MATCH without NEAR must execute");

    assert_eq!(
        results.len(),
        10,
        "LIMIT 10 must be filled: edges on late-inserted ids must be found"
    );
    for r in &results {
        assert!(
            (1951..=2000).contains(&r.point.id),
            "only ids with an outgoing REFS edge may match, got {}",
            r.point.id
        );
    }
}

/// GIVEN the same corpus, where every point matches `category = 'common'`
/// WHEN combining the metadata filter with a graph MATCH and no NEAR
/// THEN the metadata fetch window must not be capped at the ranked
///      over-fetch bound either — 10 rows are returned.
#[test]
fn test_metadata_and_graph_match_without_near_scans_beyond_overfetch_window() {
    let (_dir, db) = create_test_db();
    setup_large_collection_with_late_edges(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM library \
         WHERE category = 'common' AND MATCH (a)-[:REFS]->(b) LIMIT 10",
    )
    .expect("metadata + graph MATCH without NEAR must execute");

    assert_eq!(
        results.len(),
        10,
        "LIMIT 10 must be filled: metadata fetch must cover the whole collection"
    );
    for r in &results {
        assert!(
            (1951..=2000).contains(&r.point.id),
            "only ids with an outgoing REFS edge may match, got {}",
            r.point.id
        );
    }
}

/// GIVEN 25 anchors (> the former 10×LIMIT over-fetch window of 20) where
///       similarity to the query INCREASES with the id — the most similar
///       anchor has the LARGEST id
/// WHEN running `MATCH ... ORDER BY similarity(vector, $q) DESC LIMIT 2` (no NEAR)
/// THEN the most similar anchor (largest id) must be first: the anchored
///      fetch hydrates ascending ids, so a bounded window would drop it
///      before the downstream similarity sort ever sees it.
#[test]
fn test_orderby_similarity_without_near_scores_all_anchors() {
    let (_dir, db) = create_test_db();
    db.create_vector_collection("papers", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create papers collection");
    let vc = db
        .get_vector_collection("papers")
        .expect("test: get papers collection");

    // id 26 is the hub (no outgoing edge); ids 1..=25 all cite it.
    // Similarity to [1,0,0,0] increases with id: id 25 is the most similar.
    let mut points: Vec<Point> = (1..=25u8)
        .map(|i| {
            Point::new(
                u64::from(i),
                vec![1.0, f32::from(26 - i) * 0.05, 0.0, 0.0],
                Some(json!({"title": format!("paper-{i}")})),
            )
        })
        .collect();
    points.push(Point::new(26, vec![0.0, 1.0, 0.0, 0.0], Some(json!({}))));
    vc.upsert(points).expect("test: upsert papers corpus");
    for (edge_id, source) in (500u64..).zip(1u64..=25) {
        let edge = GraphEdge::new(edge_id, source, 26, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }

    let sql = "SELECT * FROM papers AS p \
               WHERE MATCH (p)-[:CITES]->(h) \
               ORDER BY similarity(vector, $q) DESC LIMIT 2";
    let mut params = std::collections::HashMap::new();
    params.insert("q".to_string(), json!([1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32]));

    let results = execute_sql_with_params(&db, sql, &params)
        .expect("MATCH + ORDER BY similarity() without NEAR must execute");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![25, 24],
        "the most similar anchors (largest ids) must win: every anchor must \
         be scored, not just the first window in ascending-id order"
    );
}

// =========================================================================
// B2. Anchored hybrid (NEAR + graph MATCH + text MATCH) honors FUSION strategy
// =========================================================================

/// Creates a "notes" collection where every note cites a hub (id 99) and the
/// text/vector score streams disagree, so RRF rank-fusion and Maximum
/// score-fusion rank the anchors differently.
///
/// | id | vector       | content              | vector rank | bm25 rank |
/// |----|--------------|----------------------|-------------|-----------|
/// | 1  | `[1.0, 0.0]` | "learning"           | 1 (closest) | low       |
/// | 2  | `[0.2, 0.98]`| "learning learning   | low         | 1 (best)  |
/// |    |              |  learning systems"   |             |           |
fn setup_anchored_hybrid_notes(db: &Database) {
    db.create_vector_collection("notes", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create notes collection");
    let vc = db
        .get_vector_collection("notes")
        .expect("test: get notes collection");
    vc.upsert(vec![
        Point::new(1, vec![1.0, 0.0], Some(json!({"content": "learning"}))),
        Point::new(
            2,
            vec![0.2, 0.98],
            Some(json!({"content": "learning learning learning systems"})),
        ),
        // Hub: cited by 1 and 2, matches neither branch strongly.
        Point::new(99, vec![0.0, 0.01], Some(json!({"content": "hub"}))),
    ])
    .expect("test: upsert notes corpus");
    for (edge_id, source) in [(700u64, 1u64), (701, 2)] {
        let edge = GraphEdge::new(edge_id, source, 99, "CITES").expect("test: create edge");
        vc.add_edge(edge).expect("test: add CITES edge");
    }
}

/// GIVEN notes whose vector-similarity and BM25 score streams disagree, all
///       sharing an AND-required graph CITES anchor
/// WHEN running the anchored hybrid `NEAR + MATCH (graph) + content MATCH`
///      with `USING FUSION(strategy='maximum')` vs `strategy='rrf'`
/// THEN the anchored path honors the requested strategy (scores/order differ),
///      while the predicate-filtered result SET is identical (bug #6, anchored).
#[test]
fn test_anchored_hybrid_honors_fusion_strategy() {
    let (_dir, db) = create_test_db();
    setup_anchored_hybrid_notes(&db);

    let q = "SELECT * FROM notes AS n \
             WHERE vector NEAR [1.0, 0.0] AND MATCH (n)-[:CITES]->(h) \
             AND content MATCH 'learning' LIMIT 10";
    let rrf = execute_sql(&db, &format!("{q} USING FUSION(strategy = 'rrf', k = 60)"))
        .expect("anchored rrf hybrid must execute");
    let maximum = execute_sql(&db, &format!("{q} USING FUSION(strategy = 'maximum')"))
        .expect("anchored maximum hybrid must execute");

    // The predicate-filtered SET is identical: only nodes citing the hub AND
    // matching the text (ids 1 and 2; the hub 99 never matches 'learning').
    assert_eq!(
        result_ids(&rrf),
        [1u64, 2].into_iter().collect(),
        "rrf anchored set must be the citing text-matching notes"
    );
    assert_eq!(
        result_ids(&maximum),
        result_ids(&rrf),
        "strategy must not change the anchored predicate-filtered SET"
    );

    // The fusion strategy must change the ranking/scores (it was ignored before).
    let rrf_scored: Vec<(u64, f32)> = rrf.iter().map(|r| (r.point.id, r.score)).collect();
    let max_scored: Vec<(u64, f32)> = maximum.iter().map(|r| (r.point.id, r.score)).collect();
    assert_ne!(
        rrf_scored, max_scored,
        "anchored hybrid must honor strategy='maximum' (different ranking than rrf)"
    );
}

// =========================================================================
// C. Flagship: implicit anchor binding (V011 relaxation, guards G1/G2/G3)
// =========================================================================

/// Creates an `agent_memory` collection for the flagship query.
///
/// Vectors (4-dim, cosine), query is `[1, 0, 0, 0]`; similarity strictly
/// decreases with the id.
///
/// | ids    | session_id | outgoing RELATES_TO |
/// |--------|------------|---------------------|
/// | 1..=12 | sess-42    | yes (-> 20)         |
/// | 13     | sess-42    | no                  |
/// | 14     | other      | yes (-> 20)         |
/// | 20     | other      | no (target only)    |
fn setup_agent_memory(db: &Database) {
    db.create_vector_collection("agent_memory", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create agent_memory");
    let vc = db
        .get_vector_collection("agent_memory")
        .expect("test: get agent_memory");

    let session = |id: u16| {
        if id == 14 || id == 20 {
            "other"
        } else {
            "sess-42"
        }
    };
    let points: Vec<Point> = (1..=14u16)
        .chain(std::iter::once(20))
        .map(|id| {
            Point::new(
                u64::from(id),
                vec![1.0, 0.05 * f32::from(id), 0.0, 0.0],
                Some(json!({"session_id": session(id)})),
            )
        })
        .collect();
    vc.upsert(points).expect("test: upsert agent_memory");

    for (edge_id, source) in (500u64..).zip((1u64..=12).chain(std::iter::once(14))) {
        let edge = GraphEdge::new(edge_id, source, 20, "RELATES_TO").expect("test: create edge");
        vc.add_edge(edge).expect("test: add RELATES_TO edge");
    }
}

/// GIVEN an `agent_memory` collection with embeddings, session payloads, and
///      RELATES_TO edges
/// WHEN running the flagship agent-memory query VERBATIM (implicit anchor:
///      no pattern alias matches the FROM alias `memory`, so `ctx` binds to
///      the FROM rows)
/// THEN it returns the memories with an outgoing RELATES_TO edge AND the
///      matching session_id, ordered by similarity DESC, capped at LIMIT 10.
#[test]
fn test_flagship_agent_memory_query_runs_verbatim() {
    let (_dir, db) = create_test_db();
    setup_agent_memory(&db);

    let sql = "SELECT memory.*, similarity() FROM agent_memory AS memory \
               WHERE vector NEAR $embedding AND MATCH (ctx)-[:RELATES_TO]->(fact) \
               AND session_id = $current_session \
               ORDER BY similarity() DESC LIMIT 10";
    let mut params = std::collections::HashMap::new();
    params.insert(
        "embedding".to_string(),
        json!([1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32]),
    );
    params.insert("current_session".to_string(), json!("sess-42"));

    let results = execute_sql_with_params(&db, sql, &params)
        .expect("flagship agent-memory query must run verbatim");

    // Candidates: ids 1..=12 (outgoing edge + session match; 13 has no edge,
    // 14/20 are in another session). LIMIT 10 keeps the 10 most similar.
    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        (1..=10).collect::<Vec<u64>>(),
        "top-10 relating session memories, similarity DESC order"
    );
    for pair in results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "ORDER BY similarity() DESC must hold"
        );
    }
}

/// GIVEN the same agent-memory corpus
/// WHEN excluding the pattern with NOT MATCH (implicit anchor, exact dual of
///      the positive case)
/// THEN only the memories WITHOUT an outgoing RELATES_TO edge are returned.
#[test]
fn test_not_match_implicit_anchor_excludes_relating_memories() {
    let (_dir, db) = create_test_db();
    setup_agent_memory(&db);

    let results = execute_sql(
        &db,
        "SELECT * FROM agent_memory AS memory \
         WHERE NOT MATCH (ctx)-[:RELATES_TO]->(fact) LIMIT 20",
    )
    .expect("NOT MATCH with implicit anchor must execute");

    assert_eq!(
        result_ids(&results),
        [13u64, 20].into_iter().collect(),
        "only memories without an outgoing RELATES_TO edge may remain"
    );
}

/// GIVEN the same agent-memory corpus
/// WHEN the FROM alias `memory` appears in a NON-anchor position of the
///      pattern (guard G1: inverted direction)
/// THEN the query is rejected with a clear, actionable error naming both the
///      anchor and the declared alias BEFORE any execution.
#[test]
fn test_g1_anchor_inversion_is_clear_error() {
    let (_dir, db) = create_test_db();
    setup_agent_memory(&db);

    let sql = "SELECT memory.*, similarity() FROM agent_memory AS memory \
               WHERE vector NEAR $v AND MATCH (ctx)-[:RELATES_TO]->(memory) \
               ORDER BY similarity() DESC LIMIT 10";
    let err = execute_sql_with_params(&db, sql, &vector_param(&[1.0, 0.0, 0.0, 0.0]))
        .expect_err("FROM alias 'memory' in non-anchor position must be rejected (G1)");

    let msg = err.to_string();
    assert!(
        msg.contains("ctx"),
        "error must name the mismatched anchor alias, got: {msg}"
    );
    assert!(
        msg.contains("MATCH (memory)-[:RELATES_TO]->(ctx)"),
        "error must suggest the user's pattern re-anchored on 'memory', got: {msg}"
    );
}
