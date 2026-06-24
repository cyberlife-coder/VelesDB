//! BDD conformance tests for BM25 full-text ranking via `VelesQL` `MATCH`.
//!
//! Surface under test: the SQL path `SELECT * FROM <coll> WHERE <col> MATCH
//! '<query>' LIMIT k`. A pure-`MATCH` query (no `vector NEAR`, no metadata
//! filter) routes through `Collection::text_search` (see
//! `collection/search/query/execution_paths.rs:264`), which sets each
//! `SearchResult.score` to the **raw BM25 score** and tags
//! `component_scores = [("bm25_score", score)]` (`collection/search/text.rs`
//! lines 100-117). We therefore read the BM25 score directly off
//! `result.score`.
//!
//! BM25 formula (verified against `index/bm25/bm25.rs`,
//! `score_document_fast` + `build_idf_cache`):
//! ```text
//!   idf(t)   = ln( (N - df + 0.5) / (df + 0.5) + 1.0 )
//!   len_norm = 1 - b + b * |D| / avgdl
//!   term     = idf * tf*(k1+1) / (tf + k1*len_norm)
//!   score(D) = Σ term over query terms
//! ```
//! with `k1 = 1.2`, `b = 0.75` (`Bm25Params::default`).
//!
//! Tokenizer (verified, `Bm25Index::tokenize`): lowercases, splits on every
//! non-alphanumeric char, and DROPS tokens of length ≤ 1.
//!
//! Whole-payload indexing (verified, `collection/text_utils.rs::extract_text`):
//! every **string leaf** of the JSON payload is concatenated (space-joined)
//! and indexed; the `<col>` named in `MATCH` is IGNORED at runtime — only the
//! query text drives scoring. To keep hand-computed scores exact, the
//! single-field scenarios use payloads whose only string leaf is `content`.

use serde_json::json;
use velesdb_core::{Database, DistanceMetric, Point};

use super::helpers::{approx_eq, create_test_db, execute_sql, result_ids};

/// BM25 score epsilon — single-term f32 accumulation, 1e-4 is comfortable.
const BM25_EPS: f32 = 1e-4;

/// Create a 2-dim cosine vector collection and return its handle.
fn make_collection(db: &Database, name: &str) {
    db.create_vector_collection(name, 2, DistanceMetric::Cosine)
        .expect("test: create vector collection");
}

/// Upsert `(id, content)` points whose ONLY string leaf is `content`.
///
/// Vectors are irrelevant to a pure-`MATCH` query but must satisfy the
/// declared dimension (2); they are fixed and identical so they cannot
/// perturb the BM25-only ranking.
fn upsert_contents(db: &Database, coll: &str, docs: &[(u64, &str)]) {
    let vc = db
        .get_vector_collection(coll)
        .expect("test: collection must exist");
    let points: Vec<Point> = docs
        .iter()
        .map(|(id, content)| Point::new(*id, vec![1.0, 0.0], Some(json!({ "content": content }))))
        .collect();
    vc.upsert(points).expect("test: upsert corpus");
}

/// Fetch a result's raw BM25 score by id (pure-MATCH ⇒ `score` IS the BM25 score).
fn score_of(results: &[velesdb_core::SearchResult], id: u64) -> f32 {
    results
        .iter()
        .find(|r| r.point.id == id)
        .map(|r| r.score)
        .expect("test: expected id present in results")
}

// =========================================================================
// Scenario 1 — equal-length docs: term frequency drives ranking
// =========================================================================

/// GIVEN three equal-length (4-token) docs differing only in tf('rust'):
///       id1 tf=3, id2 tf=2, id3 tf=1, query 'rust'
/// WHEN  `WHERE content MATCH 'rust'`
/// THEN  order is [1,2,3] with scores {1:0.2098, 2:0.1836, 3:0.1335}.
///
/// Ground truth (N=3, df=3, avgdl=4, k1=1.2, b=0.75): idf=ln(0.5/3.5+1)≈0.1335;
/// len_norm=1; term=idf*tf*2.2/(tf+1.2) ⇒ 0.209835 / 0.183606 / 0.133531.
/// All three scores are strictly distinct ⇒ id order is deterministic.
#[test]
fn test_bm25_equal_length_ranks_by_term_frequency() {
    let (_dir, db) = create_test_db();
    make_collection(&db, "tf_docs");
    upsert_contents(
        &db,
        "tf_docs",
        &[
            (1, "rust rust rust filler"),
            (2, "rust rust foo bar"),
            (3, "rust alpha beta gamma"),
        ],
    );

    let results = execute_sql(
        &db,
        "SELECT * FROM tf_docs WHERE content MATCH 'rust' LIMIT 10",
    )
    .expect("test: BM25 MATCH 'rust'");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2, 3], "higher tf ⇒ higher BM25, strict order");

    assert!(
        approx_eq(score_of(&results, 1), 0.209_835, BM25_EPS),
        "id1 (tf=3) BM25 = 0.209835, got {}",
        score_of(&results, 1)
    );
    assert!(
        approx_eq(score_of(&results, 2), 0.183_606, BM25_EPS),
        "id2 (tf=2) BM25 = 0.183606, got {}",
        score_of(&results, 2)
    );
    assert!(
        approx_eq(score_of(&results, 3), 0.133_531, BM25_EPS),
        "id3 (tf=1) BM25 = 0.133531, got {}",
        score_of(&results, 3)
    );
}

// =========================================================================
// Scenario 2 — length normalization: shorter doc with same tf wins
// =========================================================================

/// GIVEN two docs each containing 'rust' exactly once but of different length:
///       id1 length=2, id2 length=9, query 'rust'
/// WHEN  `WHERE content MATCH 'rust'`
/// THEN  order is [1,2] with scores {1:0.2465, 2:0.1447}.
///
/// Ground truth (N=2, df=2, avgdl=(2+9)/2=5.5): idf=ln(0.5/2.5+1)≈0.1823;
/// id1 len_norm=1-0.75+0.75*2/5.5≈0.5227 ⇒ 0.246491;
/// id2 len_norm=1-0.75+0.75*9/5.5≈1.4773 ⇒ 0.144662. Strictly distinct.
#[test]
fn test_bm25_length_normalization_favours_shorter_doc() {
    let (_dir, db) = create_test_db();
    make_collection(&db, "len_docs");
    upsert_contents(
        &db,
        "len_docs",
        &[
            (1, "rust filler"),
            (2, "rust alpha beta gamma delta epsilon zeta eta theta"),
        ],
    );

    let results = execute_sql(
        &db,
        "SELECT * FROM len_docs WHERE content MATCH 'rust' LIMIT 10",
    )
    .expect("test: BM25 length-norm MATCH 'rust'");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2], "shorter doc (same tf) outranks longer one");

    assert!(
        approx_eq(score_of(&results, 1), 0.246_491, BM25_EPS),
        "id1 (len=2) BM25 = 0.246491, got {}",
        score_of(&results, 1)
    );
    assert!(
        approx_eq(score_of(&results, 2), 0.144_662, BM25_EPS),
        "id2 (len=9) BM25 = 0.144662, got {}",
        score_of(&results, 2)
    );
}

// =========================================================================
// Scenario 3 — multi-term query: dual-match outranks single-match
// =========================================================================

/// GIVEN three 3-token docs, query 'rust systems':
///       id1 'rust systems rust' (rust tf=2 + systems tf=1, dual match),
///       id2 'rust alpha beta'    (rust tf=1, single match),
///       id3 'systems alpha beta' (systems tf=1, single match)
/// WHEN  `WHERE content MATCH 'rust systems'`
/// THEN  all three returned ({1,2,3}); id1 ranks first; the two single-match
///       docs (id2, id3) carry EQUAL scores ⇒ their mutual order is
///       non-deterministic, so we assert score equality, not id order.
///
/// Ground truth (N=3, avgdl=3, df_rust=df_systems=2 ⇒ idf≈0.5108, len_norm=1):
/// single-term tf=1 score = 0.5108*2.2/2.2 = 0.510826 *... → both = 0.470004;
/// id1 = rust-term(tf2)+systems-term(tf1) = 0.646255+0.470004 = 1.116259.
#[test]
fn test_bm25_multiterm_dual_match_outranks_single_match() {
    let (_dir, db) = create_test_db();
    make_collection(&db, "multi_docs");
    upsert_contents(
        &db,
        "multi_docs",
        &[
            (1, "rust systems rust"),
            (2, "rust alpha beta"),
            (3, "systems alpha beta"),
        ],
    );

    let results = execute_sql(
        &db,
        "SELECT * FROM multi_docs WHERE content MATCH 'rust systems' LIMIT 10",
    )
    .expect("test: BM25 multi-term MATCH");

    assert_eq!(
        result_ids(&results),
        std::collections::HashSet::from([1, 2, 3]),
        "all three docs match at least one query term"
    );
    assert_eq!(
        results[0].point.id, 1,
        "dual-match doc id=1 must rank first (highest BM25 sum)"
    );

    let (s1, s2, s3) = (
        score_of(&results, 1),
        score_of(&results, 2),
        score_of(&results, 3),
    );
    assert!(
        approx_eq(s1, 1.116_259, BM25_EPS),
        "id1 (rust tf2 + systems tf1) BM25 = 1.116259, got {s1}"
    );
    // id2 and id3 are symmetric single matches (same tf=1, len=3, df=2).
    assert!(
        approx_eq(s2, 0.470_004, BM25_EPS) && approx_eq(s3, 0.470_004, BM25_EPS),
        "single-match docs id2/id3 each BM25 = 0.470004, got {s2} / {s3}"
    );
    assert!(
        s1 > s2 && s1 > s3,
        "dual match must strictly outrank single matches: {s1} vs {s2}/{s3}"
    );
}

// =========================================================================
// Scenario 4 — tokenizer + column-name semantics
// =========================================================================

/// GIVEN id1 has TWO string leaves {title:'unused', content:'a I rust'} and
///       id2 has only {content:'rust rust'}; the MATCH column is `title`.
/// WHEN  `WHERE title MATCH 'rust'`
/// THEN  the column name is IGNORED (whole payload indexed): BOTH docs are
///       returned, and id2 (tf=2) outranks id1 (tf=1).
///
/// Ground truth: extract_text concatenates ALL string leaves ⇒ id1 indexed
/// text = "unused" + "a I rust"; tokenizer drops len≤1 ('a','i') ⇒ id1 tokens
/// {unused, rust} (len=2, tf_rust=1); id2 {rust,rust} (len=2, tf_rust=2).
/// N=2, df_rust=2, avgdl=2 ⇒ id1=0.182322, id2=0.250692 ⇒ id2 first.
#[test]
fn test_bm25_match_ignores_column_and_indexes_whole_payload() {
    let (_dir, db) = create_test_db();
    make_collection(&db, "col_docs");

    let vc = db
        .get_vector_collection("col_docs")
        .expect("test: col_docs must exist");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({ "title": "unused", "content": "a I rust" })),
        ),
        Point::new(2, vec![1.0, 0.0], Some(json!({ "content": "rust rust" }))),
    ])
    .expect("test: upsert column corpus");

    // Column 'title' is named but ignored at runtime — whole payload is indexed.
    let results = execute_sql(
        &db,
        "SELECT * FROM col_docs WHERE title MATCH 'rust' LIMIT 10",
    )
    .expect("test: BM25 MATCH with arbitrary column name");

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert_eq!(
        ids,
        vec![2, 1],
        "column ignored: both docs returned; id2 (tf=2) outranks id1 (tf=1)"
    );
    assert!(
        approx_eq(score_of(&results, 2), 0.250_692, BM25_EPS),
        "id2 (tf=2, len=2) BM25 = 0.250692, got {}",
        score_of(&results, 2)
    );
    assert!(
        approx_eq(score_of(&results, 1), 0.182_322, BM25_EPS),
        "id1 (tf=1, len=2) BM25 = 0.182322, got {}",
        score_of(&results, 1)
    );
}

/// GIVEN the same two-doc corpus
/// WHEN  the query is a single-character token `MATCH 'a'`
/// THEN  the tokenizer drops the len≤1 token ⇒ empty query ⇒ zero results.
///
/// Ground truth: `Bm25Index::tokenize` filters `s.len() > 1`; an all-dropped
/// query yields `query_terms.is_empty()` ⇒ `search` returns an empty Vec.
#[test]
fn test_bm25_single_char_token_is_dropped_yielding_no_results() {
    let (_dir, db) = create_test_db();
    make_collection(&db, "drop_docs");
    upsert_contents(&db, "drop_docs", &[(1, "a I rust"), (2, "rust rust")]);

    let results = execute_sql(
        &db,
        "SELECT * FROM drop_docs WHERE content MATCH 'a' LIMIT 10",
    )
    .expect("test: BM25 MATCH single-char token");

    assert!(
        results.is_empty(),
        "single-char query token 'a' is dropped ⇒ empty query ⇒ no results, got {}",
        results.len()
    );
}
