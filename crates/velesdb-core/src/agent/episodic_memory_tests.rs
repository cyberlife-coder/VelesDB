//! Unit tests for EpisodicMemory (EPIC-010/US-003).

#[cfg(test)]
mod tests {
    use super::super::episodic_memory::EpisodicMemory;
    use super::super::temporal_index::TemporalIndex;
    use super::super::ttl::MemoryTtl;
    use crate::Database;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_episodic(db: Arc<Database>) -> EpisodicMemory {
        EpisodicMemory::new(
            db,
            4,
            Arc::new(MemoryTtl::new()),
            Arc::new(TemporalIndex::new()),
        )
        .expect("EpisodicMemory::new failed")
    }

    // ── Basic API ──────────────────────────────────────────────────────────────

    #[test]
    fn test_collection_name_prefixed() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));
        assert!(ep.collection_name().starts_with("_episodic"));
    }

    #[test]
    fn test_record_stores_event() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        ep.record(1, "login event", 1_000, None).unwrap();

        let events = ep.recent(10, None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 1);
        assert!(events[0].1.contains("login"));
        assert_eq!(events[0].2, 1_000);
    }

    #[test]
    fn test_record_with_embedding() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        ep.record(1, "event with vector", 2_000, Some(&emb))
            .unwrap();

        let result = ep.get_with_embedding(1).unwrap();
        assert!(result.is_some());
        let (desc, ts, vec) = result.unwrap();
        assert!(desc.contains("vector"));
        assert_eq!(ts, 2_000);
        assert_eq!(vec.len(), 4);
    }

    // ── recent() ──────────────────────────────────────────────────────────────

    #[test]
    fn test_recent_limit_respected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        for i in 0u64..5 {
            #[allow(clippy::cast_possible_wrap)] // Reason: i < 5; u64→i64 cannot wrap here.
            ep.record(i, "ev", i as i64 * 1_000, None).unwrap();
        }

        let events = ep.recent(3, None).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_recent_with_since_timestamp_filters_older() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        ep.record(1, "old", 500, None).unwrap();
        ep.record(2, "new", 2_000, None).unwrap();

        let events = ep.recent(10, Some(1_000)).unwrap();
        let ids: Vec<u64> = events.iter().map(|e| e.0).collect();
        assert!(
            ids.contains(&2),
            "event after since_timestamp must be included"
        );
        assert!(
            !ids.contains(&1),
            "event before since_timestamp must be excluded"
        );
    }

    // ── older_than() ──────────────────────────────────────────────────────────

    #[test]
    fn test_older_than_returns_events_below_threshold() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        ep.record(1, "old event", 100, None).unwrap();
        ep.record(2, "recent event", 9_000, None).unwrap();

        let results = ep.older_than(5_000, 10).unwrap();
        let ids: Vec<u64> = results.iter().map(|e| e.0).collect();
        assert!(ids.contains(&1), "event before threshold must appear");
        assert!(!ids.contains(&2), "event after threshold must not appear");
    }

    // ── recall_similar() ──────────────────────────────────────────────────────

    #[test]
    fn test_recall_similar_finds_closest() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        let emb_a = vec![1.0_f32, 0.0, 0.0, 0.0];
        let emb_b = vec![0.0_f32, 1.0, 0.0, 0.0];
        ep.record(1, "A", 1_000, Some(&emb_a)).unwrap();
        ep.record(2, "B", 2_000, Some(&emb_b)).unwrap();

        let results = ep.recall_similar(&emb_a, 2).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, 1, "most similar event must rank first");
    }

    // ── get_with_embedding() ──────────────────────────────────────────────────

    #[test]
    fn test_get_with_embedding_missing_returns_none() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        let result = ep.get_with_embedding(999).unwrap();
        assert!(result.is_none());
    }

    // ── delete() ──────────────────────────────────────────────────────────────

    #[test]
    fn test_delete_removes_event() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        ep.record(1, "to delete", 1_000, None).unwrap();
        ep.delete(1).unwrap();

        let events = ep.recent(10, None).unwrap();
        assert!(events.iter().all(|e| e.0 != 1));
    }

    // ── Dimension validation ───────────────────────────────────────────────────

    #[test]
    fn test_dimension_mismatch_rejected_on_record() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db)); // dim = 4

        let bad_emb = vec![1.0_f32, 0.0]; // only 2 dims
        let result = ep.record(1, "bad", 1_000, Some(&bad_emb));
        assert!(result.is_err());
    }

    #[test]
    fn test_dimension_mismatch_rejected_on_recall_similar() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db)); // dim = 4

        let bad_query = vec![1.0_f32, 0.0]; // only 2 dims
        let result = ep.recall_similar(&bad_query, 1);
        assert!(result.is_err());
    }

    // ── TTL ───────────────────────────────────────────────────────────────────

    #[test]
    fn test_record_with_ttl_zero_expires_immediately() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        // TTL = 0 seconds: expired as soon as it's set.
        ep.record_with_ttl(42, "ephemeral", 1_000, None, 0).unwrap();

        let events = ep.recent(10, None).unwrap();
        assert!(
            events.iter().all(|e| e.0 != 42),
            "TTL-0 event must not appear in recent()"
        );
    }

    #[test]
    fn test_record_with_positive_ttl_not_yet_expired() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));

        // TTL = 9999 seconds: not expired yet.
        ep.record_with_ttl(7, "long-lived", 1_000, None, 9_999)
            .unwrap();

        let events = ep.recent(10, None).unwrap();
        assert!(
            events.iter().any(|e| e.0 == 7),
            "event with future TTL must appear"
        );
    }

    // ── Serialize / Deserialize ────────────────────────────────────────────────

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let dir1 = tempdir().unwrap();
        let db1 = Arc::new(Database::open(dir1.path()).unwrap());
        let ep1 = make_episodic(Arc::clone(&db1));
        ep1.record(1, "event one", 1_000, None).unwrap();
        ep1.record(2, "event two", 2_000, None).unwrap();
        let bytes = ep1.serialize().unwrap();

        // Restore into a fresh collection.
        let dir2 = tempdir().unwrap();
        let db2 = Arc::new(Database::open(dir2.path()).unwrap());
        let ep2 = make_episodic(Arc::clone(&db2));
        ep2.deserialize(&bytes).unwrap();

        let events = ep2.recent(10, None).unwrap();
        assert_eq!(events.len(), 2);
        let ids: Vec<u64> = events.iter().map(|e| e.0).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }

    #[test]
    fn test_deserialize_empty_bytes_is_noop() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ep = make_episodic(Arc::clone(&db));
        ep.record(1, "existing", 1_000, None).unwrap();

        ep.deserialize(&[]).unwrap(); // must not error or wipe data

        let events = ep.recent(10, None).unwrap();
        assert_eq!(events.len(), 1);
    }

    // ── Dimension mismatch on new ──────────────────────────────────────────────

    #[test]
    fn test_new_from_db_detects_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());

        let _ep = EpisodicMemory::new_from_db(Arc::clone(&db), 4).unwrap();

        // Second instantiation with wrong dimension must fail.
        let result = EpisodicMemory::new_from_db(Arc::clone(&db), 8);
        assert!(result.is_err());
    }
}
