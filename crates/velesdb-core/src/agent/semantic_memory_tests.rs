//! Unit tests for SemanticMemory (EPIC-010/US-002).

#[cfg(test)]
mod tests {
    use super::super::semantic_memory::SemanticMemory;
    use super::super::ttl::MemoryTtl;
    use crate::Database;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_semantic(db: Arc<Database>) -> SemanticMemory {
        SemanticMemory::new(db, 4, Arc::new(MemoryTtl::new())).expect("SemanticMemory::new failed")
    }

    // ── Basic API ──────────────────────────────────────────────────────────────

    #[test]
    fn test_collection_name_prefixed() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        assert!(sm.collection_name().starts_with("_semantic"));
    }

    #[test]
    fn test_dimension_accessor() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        assert_eq!(sm.dimension(), 4);
    }

    // ── store() / query() ─────────────────────────────────────────────────────

    #[test]
    fn test_store_and_query_returns_fact() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "Paris is the capital of France", &emb).unwrap();

        let results = sm.query(&emb, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
        assert!(results[0].2.contains("Paris"));
    }

    #[test]
    fn test_query_ranks_similar_first() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb_target = vec![1.0_f32, 0.0, 0.0, 0.0];
        let emb_other = vec![0.0_f32, 1.0, 0.0, 0.0];
        sm.store(1, "target fact", &emb_target).unwrap();
        sm.store(2, "unrelated fact", &emb_other).unwrap();

        let results = sm.query(&emb_target, 2).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, 1, "most similar fact must rank first");
    }

    #[test]
    fn test_store_upserts_existing_id() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "original content", &emb).unwrap();
        sm.store(1, "updated content", &emb).unwrap();

        let results = sm.query(&emb, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].2.contains("updated"));
    }

    // ── delete() ──────────────────────────────────────────────────────────────

    #[test]
    fn test_delete_removes_fact() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "to delete", &emb).unwrap();
        sm.delete(1).unwrap();

        let results = sm.query(&emb, 5).unwrap();
        assert!(results.iter().all(|r| r.0 != 1));
    }

    // ── Dimension validation ───────────────────────────────────────────────────

    #[test]
    fn test_store_dimension_mismatch_rejected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db)); // dim = 4

        let bad_emb = vec![1.0_f32, 0.0]; // dim = 2
        let result = sm.store(1, "bad", &bad_emb);
        assert!(result.is_err());
    }

    #[test]
    fn test_query_dimension_mismatch_rejected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db)); // dim = 4

        let bad_query = vec![0.5_f32]; // dim = 1
        let result = sm.query(&bad_query, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_new_detects_dimension_mismatch_on_existing_collection() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());

        let _sm = SemanticMemory::new_from_db(Arc::clone(&db), 4).unwrap();

        let result = SemanticMemory::new_from_db(Arc::clone(&db), 8);
        assert!(result.is_err());
    }

    // ── TTL ───────────────────────────────────────────────────────────────────

    #[test]
    fn test_ttl_zero_expires_immediately() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store_with_ttl(99, "short-lived fact", &emb, 0).unwrap();

        let results = sm.query(&emb, 5).unwrap();
        assert!(
            results.iter().all(|r| r.0 != 99),
            "TTL-0 fact must not appear in query results"
        );
    }

    #[test]
    fn test_store_with_positive_ttl_still_visible() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store_with_ttl(5, "long-lived fact", &emb, 9_999)
            .unwrap();

        let results = sm.query(&emb, 5).unwrap();
        assert!(
            results.iter().any(|r| r.0 == 5),
            "fact with future TTL must appear in query results"
        );
    }

    // ── Serialize / Deserialize ────────────────────────────────────────────────

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let dir1 = tempdir().unwrap();
        let db1 = Arc::new(Database::open(dir1.path()).unwrap());
        let sm1 = make_semantic(Arc::clone(&db1));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm1.store(10, "fact to persist", &emb).unwrap();
        sm1.store(11, "another fact", &emb).unwrap();
        let bytes = sm1.serialize().unwrap();

        // Restore into a fresh collection on a different database.
        let dir2 = tempdir().unwrap();
        let db2 = Arc::new(Database::open(dir2.path()).unwrap());
        let sm2 = make_semantic(Arc::clone(&db2));
        sm2.deserialize(&bytes).unwrap();

        let results = sm2.query(&emb, 5).unwrap();
        assert_eq!(results.len(), 2);
        let ids: Vec<u64> = results.iter().map(|r| r.0).collect();
        assert!(ids.contains(&10));
        assert!(ids.contains(&11));
    }

    #[test]
    fn test_deserialize_empty_bytes_is_noop() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "existing fact", &emb).unwrap();

        sm.deserialize(&[]).unwrap(); // must not error or wipe data

        let results = sm.query(&emb, 5).unwrap();
        assert_eq!(results.len(), 1);
    }
}
