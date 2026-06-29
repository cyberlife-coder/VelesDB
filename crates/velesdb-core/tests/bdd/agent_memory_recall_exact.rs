//! BDD: pin AGENT-MEMORY recall to EXACT results (not just "non-empty").
//!
//! These tests fix the precise recall contract of the `AgentMemory` typed API
//! (`semantic()`, `episodic()`, `procedural()`), which previously was only
//! asserted loosely. Every scenario states its hand-computed ground truth.
//!
//! ## Cosine ground truth (used by the semantic scenarios)
//!
//! All semantic facts have the shape `[1.0, off, 0.0, 0.0]` and the query is
//! `[1, 0, 0, 0]`. Cosine similarity is `dot / (|a||b|) = 1 / sqrt(1 + off^2)`,
//! which is **strictly decreasing** in `off`. With the well-separated offsets
//! `{0.0, 0.3, 0.7, 1.2, 3.0}` the exact similarities are:
//!   off=0.0 -> 1.0000   off=0.3 -> 0.9578   off=0.7 -> 0.8189
//!   off=1.2 -> 0.6402   off=3.0 -> 0.3162
//! so the recall order by descending similarity is unambiguously the order of
//! increasing `off`. The gaps (>= 0.06) are large enough that HNSW reproduces
//! the exact order.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use tempfile::TempDir;
use velesdb_core::agent::AgentMemory;
use velesdb_core::{velesql::Parser, Database, SearchResult};

// ============================================================================
// Helpers
// ============================================================================

/// Create a `Database` + `AgentMemory` with dimension 4 for test isolation.
fn setup() -> (TempDir, Arc<Database>, AgentMemory) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");
    (dir, db, memory)
}

/// The canonical query vector for all cosine scenarios.
const Q: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Store the five well-separated cosine facts (see module ground truth).
/// Returns ids in DECREASING-similarity order so callers can assert against it.
fn store_five_facts(memory: &AgentMemory) -> [u64; 5] {
    let facts = [
        (10_u64, 0.0_f32),
        (20, 0.3),
        (30, 0.7),
        (40, 1.2),
        (50, 3.0),
    ];
    for (id, off) in facts {
        memory
            .semantic()
            .store(id, "fact", &[1.0, off, 0.0, 0.0])
            .expect("store fact");
    }
    [10, 20, 30, 40, 50]
}

/// Recalled ids in returned (similarity-descending) order.
fn recalled_ids(rows: &[(u64, f32, String)]) -> Vec<u64> {
    rows.iter().map(|r| r.0).collect()
}

/// Execute a `VelesQL` query through `Database::execute_query` (no params).
fn db_sql(db: &Database, sql: &str) -> Vec<SearchResult> {
    let query = Parser::parse(sql)
        .map_err(|e| velesdb_core::Error::Query(e.to_string()))
        .expect("parse SQL");
    db.execute_query(&query, &std::collections::HashMap::new())
        .expect("execute SQL")
}

// ============================================================================
// (1) semantic().query -> exact ordered ids + strictly decreasing scores
// ============================================================================

/// GIVEN five facts at offsets {0.0,0.3,0.7,1.2,3.0} (sims 1.0>0.958>0.819>
/// 0.640>0.316). WHEN `query([1,0,0,0], 5)`. THEN ids come back in exactly the
/// decreasing-similarity order [10,20,30,40,50] and scores strictly decrease,
/// because cosine `1/sqrt(1+off^2)` is monotonic in `off`.
#[test]
fn test_semantic_query_exact_order_and_monotonic_scores() {
    let (_dir, _db, memory) = setup();
    let expected = store_five_facts(&memory);

    let rows = memory.semantic().query(&Q, 5).expect("query");

    assert_eq!(recalled_ids(&rows), expected.to_vec(), "exact recall order");
    let scores: Vec<f32> = rows.iter().map(|r| r.1).collect();
    assert!(
        scores.windows(2).all(|w| w[0] > w[1]),
        "scores must strictly decrease: {scores:?}"
    );
}

/// GIVEN the same five facts. WHEN `query(q, 3)` (top-k truncation). THEN only
/// the three most similar survive, in exact order [10,20,30] — the k=3 cut
/// drops offsets 1.2 and 3.0, the two least similar.
#[test]
fn test_semantic_query_topk_truncates_to_exact_prefix() {
    let (_dir, _db, memory) = setup();
    store_five_facts(&memory);

    let rows = memory.semantic().query(&Q, 3).expect("query k=3");

    assert_eq!(rows.len(), 3, "k=3 returns exactly 3 rows");
    assert_eq!(recalled_ids(&rows), vec![10, 20, 30], "top-3 exact prefix");
}

// ============================================================================
// (2) query_filtered: exact ids under a metadata filter + offset pagination
// ============================================================================

/// GIVEN five facts where the three closest (ids 10,20,30) carry
/// `team="red"` and the two farthest (40,50) carry `team="blue"`. WHEN
/// `query_filtered(q, 10, {team:red}, 0)`. THEN exactly [10,20,30] in
/// similarity order — the blue rows are filtered out, the red rows keep their
/// cosine ranking.
#[test]
fn test_query_filtered_honors_metadata_filter_exact() {
    let (_dir, _db, memory) = setup();
    let teams = [
        (10_u64, "red"),
        (20, "red"),
        (30, "red"),
        (40, "blue"),
        (50, "blue"),
    ];
    let offs = [0.0_f32, 0.3, 0.7, 1.2, 3.0];
    for ((id, team), off) in teams.into_iter().zip(offs) {
        let mut meta = Map::new();
        meta.insert("team".to_string(), Value::String(team.to_string()));
        memory
            .semantic()
            .store_with_metadata(id, "fact", &[1.0, off, 0.0, 0.0], &meta)
            .expect("store with metadata");
    }
    let mut filter = Map::new();
    filter.insert("team".to_string(), json!("red"));

    let rows = memory
        .semantic()
        .query_filtered(&Q, 10, &filter, 0)
        .expect("query_filtered");

    assert_eq!(recalled_ids(&rows), vec![10, 20, 30], "only red, ranked");
}

/// GIVEN the same five facts (10,20,30 = red; 40,50 = blue). WHEN
/// `query_excluding(q, 10, {team:blue})`. THEN exactly [10,20,30] in similarity
/// order — the negative counterpart of `query_filtered`: blue rows are dropped,
/// the rest keep their cosine ranking. AND an empty exclude drops nothing.
#[test]
fn test_query_excluding_drops_matching_payload() {
    let (_dir, _db, memory) = setup();
    let teams = [
        (10_u64, "red"),
        (20, "red"),
        (30, "red"),
        (40, "blue"),
        (50, "blue"),
    ];
    let offs = [0.0_f32, 0.3, 0.7, 1.2, 3.0];
    for ((id, team), off) in teams.into_iter().zip(offs) {
        let mut meta = Map::new();
        meta.insert("team".to_string(), Value::String(team.to_string()));
        memory
            .semantic()
            .store_with_metadata(id, "fact", &[1.0, off, 0.0, 0.0], &meta)
            .expect("store with metadata");
    }
    let mut exclude = Map::new();
    exclude.insert("team".to_string(), json!("blue"));

    let rows = memory
        .semantic()
        .query_excluding(&Q, 10, &exclude)
        .expect("query_excluding");
    assert_eq!(
        recalled_ids(&rows),
        vec![10, 20, 30],
        "blue dropped, ranked"
    );

    let all = memory
        .semantic()
        .query_excluding(&Q, 10, &Map::new())
        .expect("query_excluding empty");
    assert_eq!(
        recalled_ids(&all),
        vec![10, 20, 30, 40, 50],
        "empty exclude drops nothing"
    );
}

/// GIVEN 12 EXCLUDED points ranked nearer the query than 3 wanted facts (a
/// dense band of hubs, the realistic auto-extraction shape). WHEN
/// `query_excluding(q, 3, {block:yes})`. THEN all 3 facts come back — the
/// growing fetch must not let the excluded band starve the result below k (a
/// fixed `max(2k, k+8)` window would fetch only 11 ≈ all-excluded and return 0).
#[test]
fn test_query_excluding_grows_fetch_past_a_dense_excluded_band() {
    let (_dir, _db, memory) = setup();
    // 12 excluded points at the nearest offsets, then 3 wanted facts farther out.
    for i in 0..12u64 {
        let mut meta = Map::new();
        meta.insert("block".to_string(), json!("yes"));
        let off = i as f32 * 0.1; // 0.0 .. 1.1, all nearer than the facts
        memory
            .semantic()
            .store_with_metadata(100 + i, "hub", &[1.0, off, 0.0, 0.0], &meta)
            .expect("store excluded");
    }
    let want = [200_u64, 201, 202];
    for (j, id) in want.into_iter().enumerate() {
        let off = 2.0 + j as f32 * 0.1; // farther than every excluded point
        memory
            .semantic()
            .store(id, "fact", &[1.0, off, 0.0, 0.0])
            .expect("store fact");
    }
    let mut exclude = Map::new();
    exclude.insert("block".to_string(), json!("yes"));

    let rows = memory
        .semantic()
        .query_excluding(&Q, 3, &exclude)
        .expect("query_excluding");
    assert_eq!(
        recalled_ids(&rows),
        want.to_vec(),
        "all 3 facts survive the dense excluded band"
    );
}

/// GIVEN the five facts (no filter). WHEN paginating with k=2: page0 =
/// offset 0, page1 = offset 2, page2 = offset 4. THEN the pages are exact,
/// contiguous, non-overlapping slices of the global similarity order
/// [10,20,30,40,50]: [10,20], [30,40], [50].
#[test]
fn test_query_filtered_offset_pagination_exact() {
    let (_dir, _db, memory) = setup();
    store_five_facts(&memory);
    let empty = Map::new();

    let page0 = memory
        .semantic()
        .query_filtered(&Q, 2, &empty, 0)
        .expect("p0");
    let page1 = memory
        .semantic()
        .query_filtered(&Q, 2, &empty, 2)
        .expect("p1");
    let page2 = memory
        .semantic()
        .query_filtered(&Q, 2, &empty, 4)
        .expect("p2");

    assert_eq!(recalled_ids(&page0), vec![10, 20], "page 0");
    assert_eq!(recalled_ids(&page1), vec![30, 40], "page 1");
    assert_eq!(recalled_ids(&page2), vec![50], "page 2 (tail, 1 row)");
}

// ============================================================================
// (3) delete() removes a fact from subsequent recall (recomputed exact set)
// ============================================================================

/// GIVEN the five facts. WHEN id 20 (the 2nd closest) is deleted and we
/// re-query top-5. THEN recall is exactly [10,30,40,50]: the surviving four in
/// unchanged cosine order, with 20 absent — deletion is reflected immediately.
#[test]
fn test_delete_removes_fact_from_recall_exact() {
    let (_dir, _db, memory) = setup();
    store_five_facts(&memory);

    memory.semantic().delete(20).expect("delete id 20");
    let rows = memory.semantic().query(&Q, 5).expect("re-query");

    assert_eq!(
        recalled_ids(&rows),
        vec![10, 30, 40, 50],
        "deleted id 20 must be gone, rest keep order"
    );
}

// ============================================================================
// (4) episodic: increasing timestamps -> SQL ORDER BY timestamp DESC exact ids
// ============================================================================

/// Records three episodic events (ids 1,2,3) at strictly increasing timestamps.
fn record_three_episodes(memory: &AgentMemory) {
    memory
        .episodic()
        .record(1, "old", 1000, Some(&Q))
        .expect("rec 1");
    memory
        .episodic()
        .record(2, "mid", 2000, Some(&Q))
        .expect("rec 2");
    memory
        .episodic()
        .record(3, "new", 3000, Some(&Q))
        .expect("rec 3");
}

/// GIVEN three events recorded with strictly increasing timestamps
/// (1000<2000<3000) under ids 1,2,3. WHEN SQL `ORDER BY timestamp DESC` over
/// `_episodic_memory`. THEN ids come back newest-first: exactly [3,2,1].
#[test]
fn test_episodic_order_by_timestamp_desc_exact_ids() {
    let (_dir, db, memory) = setup();
    record_three_episodes(&memory);

    let rows = db_sql(
        &db,
        "SELECT * FROM _episodic_memory ORDER BY timestamp DESC LIMIT 10",
    );

    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![3, 2, 1], "newest timestamp first");
}

/// GIVEN the same three events. WHEN SQL `ORDER BY timestamp ASC LIMIT 2`.
/// THEN exactly the two oldest in ascending order: [1,2].
#[test]
fn test_episodic_order_by_timestamp_asc_limit_exact() {
    let (_dir, db, memory) = setup();
    record_three_episodes(&memory);

    let rows = db_sql(
        &db,
        "SELECT * FROM _episodic_memory ORDER BY timestamp ASC LIMIT 2",
    );

    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2], "two oldest, ascending");
}

// ============================================================================
// (5) procedural: learn several -> exact count + confidence-ordered recall
// ============================================================================

/// GIVEN three procedures learned with confidences {0.9, 0.5, 0.3} at the same
/// query vector. WHEN SQL `ORDER BY confidence DESC` over `_procedural_memory`.
/// THEN exactly 3 rows in confidence order [1,2,3] (the ids were assigned in
/// decreasing-confidence order), proving the stored `confidence` payload is the
/// sort key.
#[test]
fn test_procedural_count_and_confidence_order_exact() {
    let (_dir, db, memory) = setup();
    memory
        .procedural()
        .learn(1, "high", &["a".to_string()], Some(&Q), 0.9)
        .expect("learn 1");
    memory
        .procedural()
        .learn(2, "mid", &["b".to_string()], Some(&Q), 0.5)
        .expect("learn 2");
    memory
        .procedural()
        .learn(3, "low", &["c".to_string()], Some(&Q), 0.3)
        .expect("learn 3");

    let rows = db_sql(
        &db,
        "SELECT * FROM _procedural_memory ORDER BY confidence DESC LIMIT 10",
    );

    assert_eq!(rows.len(), 3, "exactly 3 procedures stored");
    let ids: Vec<u64> = rows.iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 2, 3], "highest confidence first");
}

/// GIVEN two procedures, only id 7 has confidence >= 0.8. WHEN `recall(q, 10,
/// 0.8)` (min-confidence floor). THEN exactly one match, id 7 — the 0.4-conf
/// procedure is filtered below the floor.
#[test]
fn test_procedural_recall_min_confidence_floor_exact() {
    let (_dir, _db, memory) = setup();
    memory
        .procedural()
        .learn(7, "strong", &["x".to_string()], Some(&Q), 0.9)
        .expect("learn strong");
    memory
        .procedural()
        .learn(8, "weak", &["y".to_string()], Some(&Q), 0.4)
        .expect("learn weak");

    let matches = memory.procedural().recall(&Q, 10, 0.8).expect("recall");

    assert_eq!(matches.len(), 1, "only the >=0.8 procedure survives");
    assert_eq!(matches[0].id, 7, "id 7 is the strong procedure");
}

// ============================================================================
// (6) set_ttl_durable(id,0) + auto_expire -> exact expired count, gone from recall
// ============================================================================

/// GIVEN five facts; ids 10 and 40 get an immediate durable TTL
/// (`set_ttl_durable(id, 0)` persists `_veles_expires_at = now`, it does NOT
/// delete the point). WHEN `auto_expire()` runs after a reopen. THEN
/// `semantic_expired == 2` (exactly the two TTL'd facts) and recall returns the
/// surviving three in cosine order [20,30,50].
#[test]
fn test_ttl_zero_then_auto_expire_exact_count_and_recall() {
    let dir = TempDir::new().expect("test: create temp dir");
    {
        let db = Arc::new(Database::open(dir.path()).expect("open db"));
        let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("create AgentMemory");
        store_five_facts(&memory);
        memory.semantic().set_ttl_durable(10, 0).expect("ttl 10");
        memory.semantic().set_ttl_durable(40, 0).expect("ttl 40");
    }

    let db = Arc::new(Database::open(dir.path()).expect("reopen db"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("reopen AgentMemory");
    let stats = memory.auto_expire().expect("auto_expire");

    assert_eq!(stats.semantic_expired, 2, "exactly ids 10 and 40 expired");
    let rows = memory.semantic().query(&Q, 5).expect("query survivors");
    assert_eq!(
        recalled_ids(&rows),
        vec![20, 30, 50],
        "expired 10 & 40 gone, rest keep cosine order"
    );
}
