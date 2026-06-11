//! BDD integration tests for Agent Memory `VelesQL` queryability.
//!
//! Proves that agent memory collections (`_semantic_memory`, `_episodic_memory`,
//! `_procedural_memory`) created via `AgentMemory` are queryable through the
//! standard `Database::execute_query` pipeline (not just `Collection::execute_query_str`).

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::agent::AgentMemory;
use velesdb_core::{velesql::Parser, Database, SearchResult};

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
