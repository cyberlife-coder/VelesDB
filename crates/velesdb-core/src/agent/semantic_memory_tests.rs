//! Unit tests for SemanticMemory (EPIC-010/US-002).

#[cfg(test)]
mod tests {
    use super::super::semantic_memory::SemanticMemory;
    use super::super::ttl::{MemoryKind, MemoryTtl};
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

    // ── #1040: expired top-k point must not shrink results below k ──────────────

    #[test]
    fn test_expired_topk_point_freed_slot_filled_by_live_point() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        // Shared TTL so we can mark an already-persisted live point as expired
        // without physically deleting it (the bug shape: expired-but-present).
        let ttl = Arc::new(MemoryTtl::new());
        let sm = SemanticMemory::new(Arc::clone(&db), 4, Arc::clone(&ttl)).unwrap();

        let q = vec![1.0_f32, 0.0, 0.0, 0.0];
        let near = vec![0.99_f32, 0.14, 0.0, 0.0];
        // id=1 is the absolute best match and physically present, but expired.
        sm.store(1, "expired best match", &q).unwrap();
        // Keyed by the semantic namespace so the subsystem observes the expiry.
        ttl.set_ttl(MemoryKind::Semantic, 1, 0); // expires immediately, point still persisted
        sm.store(2, "live runner up", &near).unwrap();

        // Asking for k=1 must still return the live point, not an empty result.
        let results = sm.query(&q, 1).unwrap();
        assert_eq!(
            results.len(),
            1,
            "live point must fill the slot freed by the expired top-k point"
        );
        assert_eq!(results[0].0, 2);
    }

    // ── #1043(a): TTL-bearing serialize roundtrip ───────────────────────────────

    #[test]
    fn test_serialize_omits_ttl_facts_survive_roundtrip() {
        let dir1 = tempdir().unwrap();
        let db1 = Arc::new(Database::open(dir1.path()).unwrap());
        let ttl = Arc::new(MemoryTtl::new());
        let sm1 = SemanticMemory::new(Arc::clone(&db1), 4, Arc::clone(&ttl)).unwrap();

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm1.store_with_ttl(10, "fact with ttl", &emb, 9_999)
            .unwrap();
        assert!(
            ttl.get(MemoryKind::Semantic, 10).is_some(),
            "TTL entry tracked before serialize"
        );

        let bytes = sm1.serialize().unwrap();

        // Restore into a fresh subsystem with an independent TTL map.
        let dir2 = tempdir().unwrap();
        let db2 = Arc::new(Database::open(dir2.path()).unwrap());
        let ttl2 = Arc::new(MemoryTtl::new());
        let sm2 = SemanticMemory::new(Arc::clone(&db2), 4, Arc::clone(&ttl2)).unwrap();
        sm2.deserialize(&bytes).unwrap();

        // The fact survives the per-subsystem roundtrip.
        let results = sm2.query(&emb, 5).unwrap();
        assert!(results.iter().any(|r| r.0 == 10));
        // Documented limitation: TTL is NOT carried by per-subsystem serialize.
        assert!(
            ttl2.get(MemoryKind::Semantic, 10).is_none(),
            "per-subsystem serialize intentionally omits TTL state"
        );
    }

    // ── #1043(b): ttl=0 physical removal ────────────────────────────────────────

    #[test]
    fn test_store_with_ttl_zero_does_not_persist_point() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store_with_ttl(7, "ephemeral", &emb, 0).unwrap();

        // Not tracked and not physically present.
        assert_eq!(sm.count(), 0);
        assert!(sm.get(7).unwrap().is_none());
    }

    #[test]
    fn test_store_with_ttl_zero_evicts_preexisting_point() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(7, "live", &emb).unwrap();
        assert_eq!(sm.count(), 1);

        // ttl=0 over an existing id removes it physically.
        sm.store_with_ttl(7, "replace-then-expire", &emb, 0)
            .unwrap();
        assert_eq!(sm.count(), 0);
        assert!(sm.get(7).unwrap().is_none());
    }

    #[test]
    fn test_store_with_ttl_zero_dimension_mismatch_rejected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let bad = vec![1.0_f32]; // dim = 1, expected 4
        assert!(sm.store_with_ttl(1, "bad", &bad, 0).is_err());
    }

    // ── #1044: list_all / get / count / is_empty / clear / store_batch ──────────

    #[test]
    fn test_count_and_is_empty() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        assert!(sm.is_empty());
        assert_eq!(sm.count(), 0);

        sm.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        assert!(!sm.is_empty());
        assert_eq!(sm.count(), 1);
    }

    #[test]
    fn test_get_returns_content_and_embedding() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![0.0_f32, 1.0, 0.0, 0.0];
        sm.store(3, "hello", &emb).unwrap();

        let (content, vector) = sm.get(3).unwrap().expect("fact present");
        assert_eq!(content, "hello");
        assert_eq!(vector, emb);
        assert!(sm.get(404).unwrap().is_none());
    }

    #[test]
    fn test_list_all_returns_live_facts() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        sm.store(1, "first", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        sm.store(2, "second", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        let mut listed = sm.list_all().unwrap();
        listed.sort_by_key(|(id, _)| *id);
        assert_eq!(
            listed,
            vec![(1, "first".to_string()), (2, "second".to_string())]
        );
    }

    #[test]
    fn test_clear_removes_all_facts() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        sm.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        sm.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
        sm.clear().unwrap();

        assert!(sm.is_empty());
        assert!(sm.list_all().unwrap().is_empty());
    }

    #[test]
    fn test_store_batch_inserts_all() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let e1 = vec![1.0_f32, 0.0, 0.0, 0.0];
        let e2 = vec![0.0_f32, 1.0, 0.0, 0.0];
        let facts: Vec<(u64, &str, &[f32])> =
            vec![(1, "one", e1.as_slice()), (2, "two", e2.as_slice())];
        sm.store_batch(&facts).unwrap();

        assert_eq!(sm.count(), 2);
        assert_eq!(sm.get(1).unwrap().unwrap().0, "one");
        assert_eq!(sm.get(2).unwrap().unwrap().0, "two");
    }

    #[test]
    fn test_store_batch_rejects_dimension_mismatch() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let good = vec![1.0_f32, 0.0, 0.0, 0.0];
        let bad = vec![1.0_f32]; // wrong dim
        let facts: Vec<(u64, &str, &[f32])> =
            vec![(1, "ok", good.as_slice()), (2, "bad", bad.as_slice())];
        assert!(sm.store_batch(&facts).is_err());
    }

    // ── #1049: edge-case / robustness tests ─────────────────────────────────────

    #[test]
    fn test_delete_unknown_id_is_ok() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        assert!(sm.delete(12345).is_ok());
    }

    #[test]
    fn test_deserialize_malformed_bytes_errors() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        // Non-empty, not valid JSON for Vec<Point>.
        let garbage = vec![0xFF_u8, 0x00, 0x42, 0x13];
        assert!(sm.deserialize(&garbage).is_err());
    }

    #[test]
    fn test_deserialize_replaces_not_merges() {
        let dir1 = tempdir().unwrap();
        let db1 = Arc::new(Database::open(dir1.path()).unwrap());
        let sm1 = make_semantic(Arc::clone(&db1));
        sm1.store(10, "snapshot fact", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();
        let bytes = sm1.serialize().unwrap();

        let dir2 = tempdir().unwrap();
        let db2 = Arc::new(Database::open(dir2.path()).unwrap());
        let sm2 = make_semantic(Arc::clone(&db2));
        // Pre-existing fact that must NOT survive the deserialize.
        sm2.store(99, "preexisting fact", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        sm2.deserialize(&bytes).unwrap();

        let ids: Vec<u64> = sm2
            .list_all()
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec![10], "deserialize must replace, not merge");
    }

    #[test]
    fn test_concurrent_store_query_delete() {
        use std::thread;

        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = Arc::new(make_semantic(Arc::clone(&db)));

        let mut handles = Vec::new();
        for t in 0..4u64 {
            let sm = Arc::clone(&sm);
            handles.push(thread::spawn(move || {
                let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
                for i in 0..25u64 {
                    let id = t * 100 + i;
                    sm.store(id, "c", &emb).unwrap();
                    let _ = sm.query(&emb, 3).unwrap();
                    if i % 2 == 0 {
                        sm.delete(id).unwrap();
                    }
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }

        // Half of each thread's writes were deleted: 4 threads * 12 survivors.
        assert_eq!(sm.count(), 4 * 12);
    }
}
