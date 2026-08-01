//! BDD integration tests for Agent Memory `VelesQL` queryability.
//!
//! Proves that agent memory collections (`_semantic_memory`, `_episodic_memory`,
//! `_procedural_memory`) created via `AgentMemory` are queryable through the
//! standard `Database::execute_query` pipeline (not just `Collection::execute_query_str`).

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::agent::AgentMemory;
use velesdb_core::{velesql::Parser, Database, SearchResult, EXPIRES_AT_KEY};

// ============================================================================
// Helpers
// ============================================================================

/// Create a `Database` + `AgentMemory` with dimension 4 for test isolation.
fn setup_agent_memory() -> (TempDir, Arc<Database>, AgentMemory) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");
    (dir, db, memory)
}

/// Execute a `VelesQL` query through `Database::execute_query`.
fn db_execute(
    db: &Database,
    sql: &str,
    params: &HashMap<String, serde_json::Value>,
) -> velesdb_core::Result<Vec<SearchResult>> {
    let query = Parser::parse(sql).map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    db.execute_query(&query, params)
}

/// Build a params map with a single 4-dim vector parameter named `$v`.
fn vector_param(v: [f32; 4]) -> HashMap<String, serde_json::Value> {
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!(v));
    params
}

// ============================================================================
// Semantic Memory — via Database::execute_query
// ============================================================================

#[test]
fn test_semantic_memory_velesql_query() {
    let (_dir, db, memory) = setup_agent_memory();

    memory
        .semantic()
        .store(1, "Paris is the capital of France", &[1.0, 0.0, 0.0, 0.0])
        .expect("store semantic fact 1");
    memory
        .semantic()
        .store(2, "Berlin is the capital of Germany", &[0.0, 1.0, 0.0, 0.0])
        .expect("store semantic fact 2");
    memory
        .semantic()
        .store(3, "Rome is the capital of Italy", &[0.0, 0.0, 1.0, 0.0])
        .expect("store semantic fact 3");

    let params = vector_param([1.0, 0.0, 0.0, 0.0]);
    let results = db_execute(
        &db,
        "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
        &params,
    )
    .expect("semantic vector search via Database::execute_query should succeed");

    assert!(!results.is_empty(), "should return stored facts");
    assert_eq!(
        results[0].point.id, 1,
        "closest to [1,0,0,0] should be point 1"
    );
}

#[test]
fn test_semantic_memory_with_filter() {
    let (_dir, db, memory) = setup_agent_memory();

    memory
        .semantic()
        .store(1, "Paris fact", &[1.0, 0.0, 0.0, 0.0])
        .expect("store 1");
    memory
        .semantic()
        .store(2, "Berlin fact", &[0.0, 1.0, 0.0, 0.0])
        .expect("store 2");

    let params = HashMap::new();
    let results = db_execute(
        &db,
        "SELECT * FROM _semantic_memory WHERE content = 'Paris fact' LIMIT 10",
        &params,
    )
    .expect("payload filter on semantic memory should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
}

// ============================================================================
// Episodic Memory — via Database::execute_query
// ============================================================================

#[test]
fn test_episodic_memory_velesql_query() {
    let (_dir, db, memory) = setup_agent_memory();

    memory
        .episodic()
        .record(
            1,
            "User asked about weather",
            1_700_000_000,
            Some(&[1.0, 0.0, 0.0, 0.0]),
        )
        .expect("record 1");
    memory
        .episodic()
        .record(
            2,
            "User asked about code",
            1_700_000_100,
            Some(&[0.0, 1.0, 0.0, 0.0]),
        )
        .expect("record 2");

    let params = HashMap::new();
    let results = db_execute(&db, "SELECT * FROM _episodic_memory LIMIT 10", &params)
        .expect("scan query on episodic memory should succeed");

    assert_eq!(results.len(), 2, "should return 2 recorded events");
}

#[test]
fn test_episodic_memory_recent_via_sql() {
    let (_dir, db, memory) = setup_agent_memory();

    memory
        .episodic()
        .record(1, "early", 1_000_000, Some(&[1.0, 0.0, 0.0, 0.0]))
        .expect("record 1");
    memory
        .episodic()
        .record(2, "mid", 2_000_000, Some(&[0.0, 1.0, 0.0, 0.0]))
        .expect("record 2");
    memory
        .episodic()
        .record(3, "late", 3_000_000, Some(&[0.0, 0.0, 1.0, 0.0]))
        .expect("record 3");

    let params = HashMap::new();
    let results = db_execute(
        &db,
        "SELECT * FROM _episodic_memory ORDER BY timestamp DESC LIMIT 5",
        &params,
    )
    .expect("ORDER BY timestamp DESC should succeed");

    assert_eq!(results.len(), 3);
    // Check descending order by extracting timestamps.
    let timestamps: Vec<i64> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("timestamp"))
                .and_then(serde_json::Value::as_i64)
        })
        .collect();
    assert!(
        timestamps.windows(2).all(|w| w[0] >= w[1]),
        "timestamps should be descending: {timestamps:?}"
    );
}

// ============================================================================
// Procedural Memory — via Database::execute_query
// ============================================================================

#[test]
fn test_procedural_memory_velesql_query() {
    let (_dir, db, memory) = setup_agent_memory();

    memory
        .procedural()
        .learn(
            1,
            "greet_user",
            &["say hello".to_string(), "ask name".to_string()],
            Some(&[1.0, 0.0, 0.0, 0.0]),
            0.9,
        )
        .expect("learn 1");
    memory
        .procedural()
        .learn(
            2,
            "search_docs",
            &["open search".to_string()],
            Some(&[0.0, 1.0, 0.0, 0.0]),
            0.5,
        )
        .expect("learn 2");

    let params = HashMap::new();
    let results = db_execute(&db, "SELECT * FROM _procedural_memory LIMIT 10", &params)
        .expect("scan query on procedural memory should succeed");

    assert_eq!(results.len(), 2);
}

// ============================================================================
// SHOW COLLECTIONS — includes agent memory collections
// ============================================================================

#[test]
fn test_agent_memory_show_collections_includes_internal() {
    let (_dir, db, _memory) = setup_agent_memory();

    let params = HashMap::new();
    let results =
        db_execute(&db, "SHOW COLLECTIONS", &params).expect("SHOW COLLECTIONS should succeed");

    let names: Vec<&str> = results
        .iter()
        .filter_map(|r| {
            r.point
                .payload
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(serde_json::Value::as_str)
        })
        .collect();

    assert!(
        names.contains(&"_semantic_memory"),
        "SHOW COLLECTIONS should include _semantic_memory, got: {names:?}"
    );
    assert!(
        names.contains(&"_episodic_memory"),
        "SHOW COLLECTIONS should include _episodic_memory, got: {names:?}"
    );
    assert!(
        names.contains(&"_procedural_memory"),
        "SHOW COLLECTIONS should include _procedural_memory, got: {names:?}"
    );
}

// ============================================================================
// Durable TTL key — user "expires_at" metadata is business data, not a TTL
// ============================================================================

/// A semantic fact carrying a user metadata field named `expires_at` (a common
/// business field: subscription, offer, token…) must never be interpreted as a
/// durable TTL — only the reserved `_veles_expires_at` system key is.
#[test]
fn test_user_expires_at_metadata_is_not_interpreted_as_ttl() {
    // GIVEN an agent memory holding a fact whose metadata carries a past epoch
    // under the user key "expires_at"
    let dir = TempDir::new().expect("test: create temp dir");
    {
        let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
        let memory =
            AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");
        let mut meta = serde_json::Map::new();
        meta.insert("expires_at".to_string(), serde_json::json!(1_000_000_u64));
        memory
            .semantic()
            .store_with_metadata(1, "offer expired yesterday", &[1.0, 0.0, 0.0, 0.0], &meta)
            .expect("store fact with business expires_at metadata");
    }

    // WHEN the database is reopened and expired entries are purged
    let db = Arc::new(Database::open(dir.path()).expect("test: reopen database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: reopen AgentMemory");
    let stats = memory.auto_expire().expect("auto_expire should succeed");

    // THEN the fact is untouched: not expired, still queryable
    assert_eq!(
        stats.semantic_expired, 0,
        "user expires_at metadata must not expire the fact"
    );
    let results = memory
        .semantic()
        .query(&[1.0, 0.0, 0.0, 0.0], 5)
        .expect("query semantic memory");
    assert!(
        results.iter().any(|r| r.0 == 1),
        "fact must survive reopen + auto_expire"
    );
}

// ============================================================================
// Durable TTL — expired-but-unswept entries survive restart as reclaimable
// ============================================================================

/// Anti-leak guard (AM-1): an entry whose durable TTL elapsed before a restart
/// must still be reclaimed by `auto_expire` after the reopen. The TTL cache is
/// rebuilt from raw payload reads (`get_raw`); if the rebuild used the
/// TTL-filtered `get`, the expired point would become invisible AND
/// undeletable — a permanent storage leak.
#[test]
fn test_expired_ttl_is_reclaimed_by_auto_expire_after_reopen() {
    // GIVEN a fact whose durable TTL expires immediately (refresh with ttl=0
    // persists `_veles_expires_at = now`, it does not delete the point)
    let dir = TempDir::new().expect("test: create temp dir");
    {
        let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
        let memory =
            AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");
        memory
            .semantic()
            .store(1, "ephemeral fact", &[1.0, 0.0, 0.0, 0.0])
            .expect("store fact");
        memory
            .semantic()
            .set_ttl_durable(1, 0)
            .expect("persist immediate expiry");
    }

    // WHEN the database is reopened
    let db = Arc::new(Database::open(dir.path()).expect("test: reopen database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: reopen AgentMemory");

    // THEN the entry is invisible on read surfaces but auto_expire reclaims it
    assert!(
        memory
            .semantic()
            .get(1)
            .expect("get should succeed")
            .is_none(),
        "expired fact must be invisible after reopen"
    );
    let stats = memory.auto_expire().expect("auto_expire should succeed");
    assert_eq!(
        stats.semantic_expired, 1,
        "auto_expire must reclaim the expired-but-unswept fact (storage leak guard)"
    );
    let collection = db
        .get_vector_collection("_semantic_memory")
        .expect("semantic collection exists");
    assert_eq!(
        collection.len(),
        0,
        "the expired point's storage must be physically reclaimed"
    );
}

// ============================================================================
// A re-store applies only what the CURRENT call supplies
// ============================================================================
//
// The rule, stated once for the three tests below: on a re-store, only the
// properties the current call supplies are applied. A historical expiry is
// never inherited implicitly.
//
// An RL confidence and an entity tag are state the SYSTEM learned, and
// `carry_forward_reserved_keys` rightly preserves them. An expiry is an intent
// the CALLER expressed, and it must not outlive the call that expressed it.
//
// Carrying it forward left no published way to promote a TTL'd fact back to
// permanent: the very call five binding surfaces document as "omit it for a
// permanent memory" quietly reinstated the old expiry, and the only escape was
// delete-and-recreate, which mints a new id and breaks every edge pointing at
// it.
//
// These assert on the reserved `_veles_expires_at` payload key rather than
// waiting for a TTL to lapse: the claim is about what the write PERSISTS, and
// a sleeping test would pin the clock instead of the contract.

/// Case 1 — existing TTL + re-store WITHOUT a TTL must become permanent.
/// This is the one that used to fail: the expiry was carried forward.
#[test]
fn restoring_without_a_ttl_clears_an_existing_expiry() {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");

    // GIVEN a fact stored with a durable TTL
    memory
        .semantic()
        .store_with_ttl(1, "temporary at first", &[1.0, 0.0, 0.0, 0.0], 3600)
        .expect("store with ttl");
    let before = memory
        .semantic()
        .get_metadata(1)
        .expect("read metadata")
        .expect("the fact exists");
    assert!(
        before.contains_key(EXPIRES_AT_KEY),
        "precondition: the fact must actually carry an expiry, else this test proves nothing"
    );

    // WHEN it is re-stored without any TTL
    memory
        .semantic()
        .store(1, "temporary at first", &[1.0, 0.0, 0.0, 0.0])
        .expect("re-store without ttl");

    // THEN it is permanent
    let after = memory
        .semantic()
        .get_metadata(1)
        .expect("read metadata")
        .expect("the fact still exists");
    assert!(
        !after.contains_key(EXPIRES_AT_KEY),
        "a re-store without a TTL must clear the expiry — five binding surfaces \
         document this exact call as storing a permanent memory, and inheriting \
         the old expiry silently overrides the caller's intent"
    );
}

/// Case 2 — existing TTL + re-store WITH a TTL must take the NEW one.
/// Guards the other direction: clearing must not have become unconditional.
#[test]
fn restoring_with_a_ttl_replaces_the_previous_expiry() {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");

    memory
        .semantic()
        .store_with_ttl(1, "short lived", &[1.0, 0.0, 0.0, 0.0], 60)
        .expect("store with a short ttl");
    let first = memory
        .semantic()
        .get_metadata(1)
        .expect("read metadata")
        .expect("the fact exists");
    let first_expiry = first
        .get(EXPIRES_AT_KEY)
        .and_then(serde_json::Value::as_u64)
        .expect("the first write persisted an expiry");

    memory
        .semantic()
        .store_with_ttl(1, "short lived", &[1.0, 0.0, 0.0, 0.0], 86_400)
        .expect("re-store with a longer ttl");

    let second = memory
        .semantic()
        .get_metadata(1)
        .expect("read metadata")
        .expect("the fact still exists");
    let second_expiry = second
        .get(EXPIRES_AT_KEY)
        .and_then(serde_json::Value::as_u64)
        .expect("an explicit TTL must still persist an expiry");
    assert!(
        second_expiry > first_expiry,
        "the NEW ttl must win: expected an expiry later than {first_expiry}, got {second_expiry}"
    );
}

/// Case 3 — already permanent + re-store WITHOUT a TTL stays permanent.
/// The unremarkable case, pinned so a future "helpful" default cannot make a
/// plain re-store start expiring things.
#[test]
fn restoring_a_permanent_fact_without_a_ttl_keeps_it_permanent() {
    let dir = TempDir::new().expect("test: create temp dir");
    let db = Arc::new(Database::open(dir.path()).expect("test: open database"));
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).expect("test: create AgentMemory");

    memory
        .semantic()
        .store(1, "permanent from the start", &[1.0, 0.0, 0.0, 0.0])
        .expect("store permanently");
    memory
        .semantic()
        .store(1, "permanent from the start", &[1.0, 0.0, 0.0, 0.0])
        .expect("re-store permanently");

    let after = memory
        .semantic()
        .get_metadata(1)
        .expect("read metadata")
        .expect("the fact exists");
    assert!(
        !after.contains_key(EXPIRES_AT_KEY),
        "a permanent fact re-stored without a TTL must stay permanent"
    );
}
