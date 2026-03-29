//! VelesQL bridge tests for Agent Memory subsystems (VelesQL v1.10 Phase 4).
//!
//! Proves that the three agent memory collections (`_semantic_memory`,
//! `_episodic_memory`, `_procedural_memory`) are fully queryable via VelesQL
//! `execute_query_str`, and that the convenience wrappers on `AgentMemory`
//! correctly delegate to the underlying collection.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::float_cmp
)]

#[cfg(all(test, feature = "persistence"))]
mod tests {
    use crate::agent::AgentMemory;
    #[allow(deprecated)]
    use crate::Collection;
    use crate::Database;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ========================================================================
    // Helpers
    // ========================================================================

    /// Create a Database + AgentMemory with dimension 4 for test isolation.
    fn setup_agent_memory() -> (TempDir, Arc<Database>, AgentMemory) {
        let dir = TempDir::new().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();
        (dir, db, memory)
    }

    /// Retrieve the legacy `Collection` by name from the database.
    #[allow(deprecated)]
    fn get_collection(db: &Database, name: &str) -> Collection {
        db.get_collection(name)
            .unwrap_or_else(|| panic!("collection {name} should exist"))
    }

    /// Build a basic params map with a 4-dim vector parameter `$v`.
    fn params_with_vector(v: [f32; 4]) -> HashMap<String, serde_json::Value> {
        let mut params = HashMap::new();
        params.insert("v".to_string(), serde_json::json!(v));
        params
    }

    // ========================================================================
    // A. Semantic Memory — VelesQL integration
    // ========================================================================

    #[test]
    fn test_semantic_memory_queryable_via_velesql() {
        let (_dir, db, memory) = setup_agent_memory();

        // Store 3 semantic facts with distinct embeddings.
        memory
            .semantic()
            .store(1, "Paris is capital of France", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        memory
            .semantic()
            .store(2, "Berlin is capital of Germany", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();
        memory
            .semantic()
            .store(3, "Rome is capital of Italy", &[0.0, 0.0, 1.0, 0.0])
            .unwrap();

        // Query via VelesQL on the underlying collection.
        let col = get_collection(&db, "_semantic_memory");
        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = col
            .execute_query_str(
                "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("VelesQL on semantic memory should succeed");

        assert!(!results.is_empty(), "should return stored facts");
        // The closest vector to [1,0,0,0] should be point 1.
        assert_eq!(results[0].point.id, 1);
        let content = results[0]
            .point
            .payload
            .as_ref()
            .and_then(|p: &serde_json::Value| p.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert!(content.contains("Paris"));
    }

    #[test]
    fn test_semantic_memory_with_mode() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .semantic()
            .store(1, "fact one", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        memory
            .semantic()
            .store(2, "fact two", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        let col = get_collection(&db, "_semantic_memory");
        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = col
            .execute_query_str(
                "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5 WITH (mode='accurate')",
                &params,
            )
            .expect("WITH (mode='accurate') on semantic memory should work");

        assert!(!results.is_empty());
    }

    #[test]
    fn test_semantic_memory_empty_returns_empty() {
        let (_dir, db, _memory) = setup_agent_memory();

        let col = get_collection(&db, "_semantic_memory");
        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = col
            .execute_query_str(
                "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("query on empty collection should succeed");

        assert!(
            results.is_empty(),
            "empty collection should return no results"
        );
    }

    // ========================================================================
    // B. Episodic Memory — VelesQL integration
    // ========================================================================

    #[test]
    fn test_episodic_memory_queryable_via_velesql() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .episodic()
            .record(
                1,
                "User asked about weather",
                1_700_000_000,
                Some(&[1.0, 0.0, 0.0, 0.0]),
            )
            .unwrap();
        memory
            .episodic()
            .record(
                2,
                "User asked about code",
                1_700_000_100,
                Some(&[0.0, 1.0, 0.0, 0.0]),
            )
            .unwrap();
        memory
            .episodic()
            .record(
                3,
                "User asked about history",
                1_700_000_200,
                Some(&[0.0, 0.0, 1.0, 0.0]),
            )
            .unwrap();

        // Payload filter: timestamp > 1700000050 should exclude event 1.
        let col = get_collection(&db, "_episodic_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _episodic_memory WHERE timestamp > 1700000050 LIMIT 10",
                &params,
            )
            .expect("temporal payload filter should succeed");

        // Events 2 and 3 have timestamp > 1700000050.
        assert!(
            results.len() >= 2,
            "should return events with timestamp > cutoff, got {} results",
            results.len()
        );
        // Event 1 (timestamp=1700000000) should not be present.
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(!ids.contains(&1), "event 1 should be filtered out");
    }

    #[test]
    fn test_episodic_memory_similarity_search() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .episodic()
            .record(
                1,
                "weather event",
                1_700_000_000,
                Some(&[1.0, 0.0, 0.0, 0.0]),
            )
            .unwrap();
        memory
            .episodic()
            .record(2, "code event", 1_700_000_100, Some(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();

        let col = get_collection(&db, "_episodic_memory");
        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = col
            .execute_query_str(
                "SELECT * FROM _episodic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("similarity search on episodic memory should succeed");

        assert!(!results.is_empty());
        // Event 1 should be closest to [1,0,0,0].
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_episodic_memory_order_by_timestamp() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .episodic()
            .record(1, "early event", 1_000_000, Some(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        memory
            .episodic()
            .record(2, "mid event", 2_000_000, Some(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        memory
            .episodic()
            .record(3, "late event", 3_000_000, Some(&[0.0, 0.0, 1.0, 0.0]))
            .unwrap();

        let col = get_collection(&db, "_episodic_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _episodic_memory ORDER BY timestamp DESC LIMIT 10",
                &params,
            )
            .expect("ORDER BY timestamp DESC should succeed");

        assert_eq!(results.len(), 3);
        // Descending order: 3, 2, 1.
        let timestamps: Vec<i64> = results
            .iter()
            .filter_map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .and_then(|p: &serde_json::Value| p.get("timestamp"))
                    .and_then(serde_json::Value::as_i64)
            })
            .collect();
        assert!(
            timestamps.windows(2).all(|w| w[0] >= w[1]),
            "timestamps should be descending: {timestamps:?}"
        );
    }

    // ========================================================================
    // C. Procedural Memory — VelesQL integration
    // ========================================================================

    #[test]
    fn test_procedural_memory_queryable_via_velesql() {
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
            .unwrap();
        memory
            .procedural()
            .learn(
                2,
                "search_docs",
                &["open search".to_string(), "type query".to_string()],
                Some(&[0.0, 1.0, 0.0, 0.0]),
                0.5,
            )
            .unwrap();

        let col = get_collection(&db, "_procedural_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str("SELECT * FROM _procedural_memory LIMIT 10", &params)
            .expect("scan query on procedural memory should succeed");

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_procedural_memory_confidence_filter() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .procedural()
            .learn(
                1,
                "high_conf",
                &["step1".into()],
                Some(&[1.0, 0.0, 0.0, 0.0]),
                0.9,
            )
            .unwrap();
        memory
            .procedural()
            .learn(
                2,
                "low_conf",
                &["step1".into()],
                Some(&[0.0, 1.0, 0.0, 0.0]),
                0.3,
            )
            .unwrap();
        memory
            .procedural()
            .learn(
                3,
                "mid_conf",
                &["step1".into()],
                Some(&[0.0, 0.0, 1.0, 0.0]),
                0.7,
            )
            .unwrap();

        let col = get_collection(&db, "_procedural_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _procedural_memory WHERE confidence > 0.7 LIMIT 10",
                &params,
            )
            .expect("confidence filter should succeed");

        // Only point 1 has confidence 0.9 (> 0.7). Point 3 has exactly 0.7, not > 0.7.
        assert!(
            !results.is_empty(),
            "should return high-confidence procedures"
        );
        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(ids.contains(&1), "high-conf procedure should be returned");
        assert!(
            !ids.contains(&2),
            "low-conf procedure should be filtered out"
        );
    }

    #[test]
    fn test_procedural_memory_order_by_confidence() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .procedural()
            .learn(1, "low", &["s".into()], Some(&[1.0, 0.0, 0.0, 0.0]), 0.2)
            .unwrap();
        memory
            .procedural()
            .learn(2, "high", &["s".into()], Some(&[0.0, 1.0, 0.0, 0.0]), 0.9)
            .unwrap();
        memory
            .procedural()
            .learn(3, "mid", &["s".into()], Some(&[0.0, 0.0, 1.0, 0.0]), 0.5)
            .unwrap();

        let col = get_collection(&db, "_procedural_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _procedural_memory ORDER BY confidence DESC LIMIT 10",
                &params,
            )
            .expect("ORDER BY confidence DESC should succeed");

        assert_eq!(results.len(), 3);
        let confidences: Vec<f64> = results
            .iter()
            .filter_map(|r| {
                r.point
                    .payload
                    .as_ref()
                    .and_then(|p: &serde_json::Value| p.get("confidence"))
                    .and_then(serde_json::Value::as_f64)
            })
            .collect();
        assert!(
            confidences.windows(2).all(|w| w[0] >= w[1]),
            "confidences should be descending: {confidences:?}"
        );
    }

    // ========================================================================
    // D. Convenience API — AgentMemory.query_*()
    // ========================================================================

    #[test]
    fn test_agent_memory_query_semantic() {
        let (_dir, _db, memory) = setup_agent_memory();

        memory
            .semantic()
            .store(1, "the sky is blue", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        memory
            .semantic()
            .store(2, "grass is green", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = memory
            .query_semantic(
                "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("query_semantic should succeed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_agent_memory_query_episodic() {
        let (_dir, _db, memory) = setup_agent_memory();

        memory
            .episodic()
            .record(1, "event one", 1_000_000, Some(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        memory
            .episodic()
            .record(2, "event two", 2_000_000, Some(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();

        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = memory
            .query_episodic(
                "SELECT * FROM _episodic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("query_episodic should succeed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
    }

    #[test]
    fn test_agent_memory_query_procedural() {
        let (_dir, _db, memory) = setup_agent_memory();

        memory
            .procedural()
            .learn(1, "proc1", &["s1".into()], Some(&[1.0, 0.0, 0.0, 0.0]), 0.8)
            .unwrap();

        let params = HashMap::new();
        let results = memory
            .query_procedural("SELECT * FROM _procedural_memory LIMIT 10", &params)
            .expect("query_procedural should succeed");

        assert_eq!(results.len(), 1);
        let name = results[0]
            .point
            .payload
            .as_ref()
            .and_then(|p: &serde_json::Value| p.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert_eq!(name, "proc1");
    }

    // ========================================================================
    // E. Edge cases
    // ========================================================================

    #[test]
    fn test_agent_memory_query_wrong_collection_returns_no_payload_match() {
        let (_dir, db, memory) = setup_agent_memory();

        // Store semantic data (payload: {"content": "..."}).
        memory
            .semantic()
            .store(1, "a fact", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();

        // Query _semantic_memory but filter on an episodic field: "description".
        // Since semantic payloads have "content" not "description", the filter
        // should yield no matches.
        let col = get_collection(&db, "_semantic_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _semantic_memory WHERE description = 'something' LIMIT 10",
                &params,
            )
            .expect("query should succeed even with non-existent field");

        assert!(
            results.is_empty(),
            "querying wrong payload field should return empty"
        );
    }

    #[test]
    fn test_agent_memory_query_invalid_sql_returns_error() {
        let (_dir, _db, memory) = setup_agent_memory();

        let params = HashMap::new();
        let result = memory.query_semantic("THIS IS NOT SQL", &params);

        assert!(result.is_err(), "invalid SQL should return error");
    }

    #[test]
    fn test_agent_memory_query_semantic_empty_collection() {
        let (_dir, _db, memory) = setup_agent_memory();

        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = memory
            .query_semantic(
                "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("query on empty semantic should succeed");

        assert!(results.is_empty());
    }

    #[test]
    fn test_agent_memory_query_episodic_empty_collection() {
        let (_dir, _db, memory) = setup_agent_memory();

        let params = params_with_vector([1.0, 0.0, 0.0, 0.0]);
        let results = memory
            .query_episodic(
                "SELECT * FROM _episodic_memory WHERE vector NEAR $v LIMIT 5",
                &params,
            )
            .expect("query on empty episodic should succeed");

        assert!(results.is_empty());
    }

    #[test]
    fn test_agent_memory_query_procedural_empty_collection() {
        let (_dir, _db, memory) = setup_agent_memory();

        let params = HashMap::new();
        let results = memory
            .query_procedural("SELECT * FROM _procedural_memory LIMIT 10", &params)
            .expect("query on empty procedural should succeed");

        assert!(results.is_empty());
    }

    #[test]
    fn test_agent_memory_query_semantic_with_scan_no_vector() {
        let (_dir, _db, memory) = setup_agent_memory();

        memory
            .semantic()
            .store(1, "fact alpha", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        memory
            .semantic()
            .store(2, "fact beta", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        // Pure scan query (no NEAR clause).
        let params = HashMap::new();
        let results = memory
            .query_semantic("SELECT * FROM _semantic_memory LIMIT 10", &params)
            .expect("scan query should succeed");

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_episodic_memory_timestamp_range_filter() {
        let (_dir, db, memory) = setup_agent_memory();

        memory
            .episodic()
            .record(1, "old event", 1_000, Some(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        memory
            .episodic()
            .record(2, "recent event", 5_000, Some(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        memory
            .episodic()
            .record(3, "very recent event", 9_000, Some(&[0.0, 0.0, 1.0, 0.0]))
            .unwrap();

        // Range: 2000 < timestamp < 8000 => only event 2.
        let col = get_collection(&db, "_episodic_memory");
        let params = HashMap::new();
        let results = col
            .execute_query_str(
                "SELECT * FROM _episodic_memory WHERE timestamp > 2000 AND timestamp < 8000 LIMIT 10",
                &params,
            )
            .expect("timestamp range filter should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].point.id, 2);
    }
}
