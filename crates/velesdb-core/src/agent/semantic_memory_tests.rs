//! Unit tests for SemanticMemory (EPIC-010/US-002).

#[cfg(test)]
mod tests {
    use super::super::error::AgentMemoryError;
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
    fn test_get_metadata_returns_payload_excluding_none_for_unknown() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta = serde_json::Map::new();
        meta.insert("tag".to_string(), serde_json::json!("science"));
        sm.store_with_metadata(1, "Photosynthesis", &emb, &meta)
            .unwrap();

        let payload = sm.get_metadata(1).unwrap().expect("payload present");
        assert_eq!(payload.get("tag"), Some(&serde_json::json!("science")));
        assert!(sm.get_metadata(404).unwrap().is_none());
    }

    #[test]
    fn test_get_metadata_bare_store_has_no_extra_fields() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        sm.store(1, "no metadata here", &[1.0, 0.0, 0.0, 0.0])
            .unwrap();

        // `store()` still writes a payload (the reserved `content` key), so the
        // map is Some, just without any caller-supplied field.
        let payload = sm.get_metadata(1).unwrap().expect("payload present");
        assert!(!payload.contains_key("tag"));
    }

    #[test]
    fn test_get_metadata_batch_matches_individual_calls_order_and_length() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let mut tagged = serde_json::Map::new();
        tagged.insert("tag".to_string(), serde_json::json!("science"));
        sm.store_with_metadata(1, "photosynthesis", &[1.0, 0.0, 0.0, 0.0], &tagged)
            .unwrap();
        sm.store(2, "no metadata here", &[0.0, 1.0, 0.0, 0.0])
            .unwrap();

        let batch = sm.get_metadata_batch(&[1, 2, 404]).unwrap();
        assert_eq!(batch.len(), 3, "one result per input id, in order");
        assert_eq!(
            batch[0].as_ref().and_then(|m| m.get("tag")),
            Some(&serde_json::json!("science"))
        );
        assert!(!batch[1].as_ref().unwrap().contains_key("tag"));
        assert!(batch[2].is_none(), "unknown id maps to None, not an error");
    }

    #[test]
    fn test_get_metadata_batch_handles_a_gap_among_present_ids() {
        // `store_with_ttl(id, .., 0)` deletes on the spot (see
        // `test_store_with_ttl_zero_does_not_persist_point`), so id 1 here is
        // absent from storage entirely by the time the batch runs — the same
        // "missing id in the middle of the batch" shape a real expired-TTL
        // gap would produce, without needing a real-time sleep to test it.
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        sm.store_with_ttl(1, "never actually persisted", &[1.0, 0.0, 0.0, 0.0], 0)
            .unwrap();
        sm.store(2, "live", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        let batch = sm.get_metadata_batch(&[1, 2]).unwrap();
        assert!(batch[0].is_none());
        assert!(batch[1].is_some());
    }

    #[test]
    fn test_get_metadata_batch_excludes_a_durably_expired_id() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        sm.store(1, "will expire", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        sm.set_ttl_durable(1, 0).unwrap();
        sm.store(2, "live", &[0.0, 1.0, 0.0, 0.0]).unwrap();

        let batch = sm.get_metadata_batch(&[1, 2]).unwrap();
        assert!(
            batch[0].is_none(),
            "durably-expired id must not surface metadata"
        );
        assert!(batch[1].is_some());
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

    // ── #1044: store_with_metadata / update_metadata / query_filtered ───────────

    #[test]
    fn test_store_with_metadata_persists_extra_fields() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta = serde_json::Map::new();
        meta.insert("tag".to_string(), serde_json::json!("science"));
        sm.store_with_metadata(1, "Photosynthesis", &emb, &meta)
            .unwrap();

        let results = sm.query(&emb, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
        // content field still set correctly
        assert!(results[0].2.contains("Photosynthesis"));
    }

    #[test]
    fn test_store_with_metadata_content_wins_on_collision() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta = serde_json::Map::new();
        // caller tries to inject a different content via metadata
        meta.insert(
            "content".to_string(),
            serde_json::json!("should be overwritten"),
        );
        sm.store_with_metadata(1, "canonical content", &emb, &meta)
            .unwrap();

        let results = sm.query(&emb, 1).unwrap();
        assert_eq!(results[0].2, "canonical content", "content param must win");
    }

    #[test]
    fn test_update_metadata_merges_fields() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "original", &emb).unwrap();

        let mut updates = serde_json::Map::new();
        updates.insert("tag".to_string(), serde_json::json!("updated"));
        sm.update_metadata(1, &updates).unwrap();

        // content field must still be present after update
        let results = sm.query(&emb, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].2.contains("original"));
    }

    #[test]
    fn test_update_metadata_unknown_id_errors() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let updates = serde_json::Map::new();
        assert!(
            sm.update_metadata(9999, &updates).is_err(),
            "unknown id must return NotFound"
        );
    }

    #[test]
    fn test_update_metadata_expired_id_errors() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ttl = Arc::new(super::super::ttl::MemoryTtl::new());
        let sm = SemanticMemory::new(Arc::clone(&db), 4, Arc::clone(&ttl)).expect("init failed");

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "fact", &emb).unwrap();
        ttl.set_ttl(MemoryKind::Semantic, 1, 0); // immediate expiry

        let updates = serde_json::Map::new();
        assert!(
            sm.update_metadata(1, &updates).is_err(),
            "expired id must return NotFound"
        );
    }

    #[test]
    fn test_query_filtered_matches_tag() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta_a = serde_json::Map::new();
        meta_a.insert("category".to_string(), serde_json::json!("physics"));
        let mut meta_b = serde_json::Map::new();
        meta_b.insert("category".to_string(), serde_json::json!("biology"));

        sm.store_with_metadata(1, "gravity", &emb, &meta_a).unwrap();
        sm.store_with_metadata(2, "photosynthesis", &emb, &meta_b)
            .unwrap();

        let mut filter = serde_json::Map::new();
        filter.insert("category".to_string(), serde_json::json!("physics"));

        let results = sm.query_filtered(&emb, 5, &filter, 0).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_ensure_index_creates_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta = serde_json::Map::new();
        meta.insert("project".to_string(), serde_json::json!("veles"));
        sm.store_with_metadata(1, "auth bug", &emb, &meta).unwrap();

        let collection = db
            .get_vector_collection(sm.collection_name())
            .expect("semantic collection exists")
            .inner;
        assert!(
            !collection.has_secondary_index("project"),
            "no secondary index before ensure_index — recall would post-filter O(n)"
        );

        sm.ensure_index("project").expect("ensure_index");
        assert!(
            collection.has_secondary_index("project"),
            "ensure_index activates the bitmap prefilter index on the field"
        );

        // Idempotent: a second call is a cheap no-op that still succeeds.
        sm.ensure_index("project")
            .expect("ensure_index is idempotent");
        assert!(collection.has_secondary_index("project"));
    }

    #[test]
    fn test_query_filtered_skips_expired() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let ttl = Arc::new(super::super::ttl::MemoryTtl::new());
        let sm = SemanticMemory::new(Arc::clone(&db), 4, Arc::clone(&ttl)).expect("init failed");

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let mut meta = serde_json::Map::new();
        meta.insert("kind".to_string(), serde_json::json!("test"));

        sm.store_with_metadata(1, "live fact", &emb, &meta).unwrap();
        sm.store_with_metadata(2, "expired fact", &emb, &meta)
            .unwrap();
        ttl.set_ttl(MemoryKind::Semantic, 2, 0); // expire id=2

        let mut filter = serde_json::Map::new();
        filter.insert("kind".to_string(), serde_json::json!("test"));

        let results = sm.query_filtered(&emb, 5, &filter, 0).unwrap();
        assert_eq!(results.len(), 1, "expired point must be excluded");
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_query_filtered_with_offset() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let e1 = vec![1.0_f32, 0.0, 0.0, 0.0];
        let e2 = vec![0.99_f32, 0.14, 0.0, 0.0];
        let e3 = vec![0.98_f32, 0.20, 0.0, 0.0];

        let mut meta = serde_json::Map::new();
        meta.insert("grp".to_string(), serde_json::json!("x"));

        sm.store_with_metadata(1, "best", &e1, &meta).unwrap();
        sm.store_with_metadata(2, "second", &e2, &meta).unwrap();
        sm.store_with_metadata(3, "third", &e3, &meta).unwrap();

        let mut filter = serde_json::Map::new();
        filter.insert("grp".to_string(), serde_json::json!("x"));

        // offset=1 skips the top result, so second-best appears first
        let paged = sm.query_filtered(&e1, 2, &filter, 1).unwrap();
        assert_eq!(paged.len(), 2, "should get the 2nd and 3rd results");
        assert!(
            paged.iter().all(|r| r.0 != 1),
            "top result must be skipped by offset"
        );
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

    // ── Reserved durable-TTL key: user "expires_at" metadata is business data ──

    /// Reopens the database at `path` into a fresh `SemanticMemory` with its
    /// own TTL map, mimicking a process restart (payload-driven TTL rebuild).
    fn reopen_semantic(path: &std::path::Path) -> (Arc<MemoryTtl>, SemanticMemory) {
        let db = Arc::new(Database::open(path).unwrap());
        let ttl = Arc::new(MemoryTtl::new());
        let sm = SemanticMemory::new(db, 4, Arc::clone(&ttl)).unwrap();
        (ttl, sm)
    }

    /// Builds a one-entry metadata map.
    fn meta_one(key: &str, value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(key.to_string(), value);
        map
    }

    /// `set_ttl_durable` on an existing fact persists the expiry: after a
    /// reopen the TTL map is rebuilt from the payload and the fact expires.
    #[test]
    fn test_set_ttl_durable_survives_restart() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];

        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            sm.store(1, "post-hoc expiring fact", &emb).unwrap();
            // Post-hoc durable TTL: already elapsed (0 seconds).
            sm.set_ttl_durable(1, 0).unwrap();
        }

        let (ttl, sm) = reopen_semantic(dir.path());

        assert!(
            ttl.get(MemoryKind::Semantic, 1).is_some(),
            "durable post-hoc TTL must be rebuilt into the TTL map on reopen"
        );
        assert!(
            ttl.is_expired(MemoryKind::Semantic, 1),
            "a 0-second TTL must be expired after reopen"
        );
        assert!(
            sm.get(1).unwrap().is_none(),
            "expired fact must be invisible after reopen"
        );
    }

    /// `set_ttl_durable` keeps the fact's existing metadata intact and only
    /// adds the reserved expiry key.
    #[test]
    fn test_set_ttl_durable_preserves_existing_metadata() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];

        let meta = meta_one("source", serde_json::json!("chat"));
        sm.store_with_metadata(1, "fact with metadata", &emb, &meta)
            .unwrap();
        sm.set_ttl_durable(1, 3600).unwrap();

        let fact = sm.get(1).unwrap().expect("fact still alive (1h TTL)");
        assert_eq!(fact.0, "fact with metadata", "content preserved");
        let results = sm.query(&emb, 5).unwrap();
        assert!(results.iter().any(|r| r.0 == 1), "fact stays queryable");
    }

    /// `set_ttl_durable` on an expired-but-not-yet-swept id must surface
    /// `NotFound` instead of resurrecting the dead fact with a fresh TTL
    /// (expired entries are invisible on every read AND write surface).
    #[test]
    fn test_set_ttl_durable_expired_id_is_not_found() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];

        sm.store(1, "fact to expire", &emb).unwrap();
        sm.set_ttl_durable(1, 0).unwrap(); // expires immediately
        assert!(sm.get(1).unwrap().is_none(), "fact invisible once expired");

        let err = sm.set_ttl_durable(1, 3600).unwrap_err();
        assert!(
            matches!(err, AgentMemoryError::NotFound(_)),
            "refreshing an expired id must not resurrect it, got: {err:?}"
        );
        assert!(sm.get(1).unwrap().is_none(), "fact must stay invisible");
    }

    /// `set_ttl_durable` on a missing id surfaces a `NotFound` error instead
    /// of silently arming a TTL for a nonexistent fact.
    #[test]
    fn test_set_ttl_durable_missing_id_is_not_found() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));

        let err = sm.set_ttl_durable(999, 60).unwrap_err();
        assert!(
            matches!(err, AgentMemoryError::NotFound(_)),
            "missing id must yield NotFound, got: {err:?}"
        );
    }

    // ── Graph dimension: relate / relations / unrelate ─────────────────────

    /// relate() creates a typed edge between two live facts; relations()
    /// exposes it; unrelate() removes it.
    #[test]
    fn test_relate_relations_unrelate_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "context", &emb).unwrap();
        sm.store(2, "fact", &emb).unwrap();

        let props = meta_one("weight", serde_json::json!(0.9));
        let edge_id = sm.relate(1, 2, "RELATES_TO", Some(&props)).unwrap();

        let rels = sm.relations(1).unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].id(), edge_id);
        assert_eq!(rels[0].target(), 2);
        assert_eq!(rels[0].label(), "RELATES_TO");
        assert_eq!(rels[0].property("weight"), Some(&serde_json::json!(0.9)));

        assert!(sm.unrelate(edge_id).unwrap(), "edge must be removed");
        assert!(sm.relations(1).unwrap().is_empty());
    }

    /// relate() refuses missing and expired endpoints (write surfaces must
    /// not resurrect or dangle).
    #[test]
    fn test_relate_rejects_missing_and_expired_endpoints() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "context", &emb).unwrap();

        let err = sm.relate(1, 999, "RELATES_TO", None).unwrap_err();
        assert!(matches!(err, AgentMemoryError::NotFound(_)));

        sm.store(2, "ephemeral", &emb).unwrap();
        sm.set_ttl_durable(2, 0).unwrap(); // expires immediately
        let err = sm.relate(1, 2, "RELATES_TO", None).unwrap_err();
        assert!(matches!(err, AgentMemoryError::NotFound(_)));
    }

    /// Deleting a memory cascades to its relation edges (no dangling edges).
    #[test]
    fn test_delete_memory_cascades_relations() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "context", &emb).unwrap();
        sm.store(2, "fact", &emb).unwrap();
        sm.relate(1, 2, "RELATES_TO", None).unwrap();

        sm.delete(2).unwrap();
        assert!(
            sm.relations(1).unwrap().is_empty(),
            "deleting the target memory must cascade away the edge"
        );
    }

    /// Relations survive a restart (edge WAL) and the edge-id allocator
    /// reseeds past persisted edges.
    #[test]
    fn test_relations_survive_restart_without_id_collision() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let first_edge;
        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            sm.store(1, "context", &emb).unwrap();
            sm.store(2, "fact", &emb).unwrap();
            first_edge = sm.relate(1, 2, "RELATES_TO", None).unwrap();
        }

        let (_ttl, sm) = reopen_semantic(dir.path());
        let rels = sm.relations(1).unwrap();
        assert_eq!(rels.len(), 1, "edge must survive the restart (edge WAL)");
        assert_eq!(rels[0].id(), first_edge);

        sm.store(3, "another fact", &emb).unwrap();
        let second_edge = sm.relate(1, 3, "SUPPORTS", None).unwrap();
        assert_ne!(
            second_edge, first_edge,
            "reseeded allocator must not collide with persisted edges"
        );
        assert_eq!(sm.relations(1).unwrap().len(), 2);
    }

    /// Snapshot round-trip preserves relations: serialize captures the edges
    /// between snapshotted memories and restore re-adds them (review
    /// 2026-06-11: restore previously wiped every relation via the cascade).
    #[test]
    fn test_snapshot_roundtrip_preserves_relations() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "ctx", &emb).unwrap();
        sm.store(2, "fact", &emb).unwrap();
        let edge_id = sm.relate(1, 2, "RELATES_TO", None).unwrap();

        let snapshot = sm.serialize().unwrap();
        // Mutate after the snapshot: unrelate + add a new relation.
        sm.unrelate(edge_id).unwrap();
        sm.store(3, "other", &emb).unwrap();
        sm.relate(1, 3, "SUPPORTS", None).unwrap();

        sm.deserialize(&snapshot).unwrap();

        let rels = sm.relations(1).unwrap();
        assert_eq!(rels.len(), 1, "restore must bring back the snapshot edge");
        assert_eq!(rels[0].target(), 2);
        assert_eq!(rels[0].label(), "RELATES_TO");
    }

    /// Pre-graph snapshots (bare point arrays) still load — without edges.
    #[test]
    fn test_legacy_bare_array_snapshot_still_loads() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "ctx", &emb).unwrap();

        // Simulate an old snapshot: a bare JSON array of points.
        let points: Vec<crate::Point> = vec![crate::Point::new(
            7,
            emb.clone(),
            Some(serde_json::json!({"content": "legacy"})),
        )];
        let legacy = serde_json::to_vec(&points).unwrap();

        sm.deserialize(&legacy).unwrap();
        assert!(sm.get(7).unwrap().is_some(), "legacy snapshot points load");
        assert!(sm.relations(7).unwrap().is_empty());
    }

    /// flush() compacts the edge WAL into the snapshot for memory (vector)
    /// collections too, and edges survive the reopen via the snapshot
    /// (review 2026-06-11: the WAL previously grew forever and a torn tail
    /// permanently broke edge durability).
    #[test]
    fn test_flush_compacts_edge_wal_and_edges_survive_reopen() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            sm.store(1, "ctx", &emb).unwrap();
            sm.store(2, "fact", &emb).unwrap();
            sm.relate(1, 2, "RELATES_TO", None).unwrap();
            db.flush_all();
        }

        let collection_dir = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("_semantic"))
            .expect("semantic collection dir")
            .path();
        assert!(
            collection_dir.join("edge_store.bin").exists(),
            "flush must snapshot the edge store for memory collections"
        );
        let wal_len = std::fs::metadata(collection_dir.join("edges.wal")).map_or(0, |m| m.len());
        assert_eq!(wal_len, 0, "flush must truncate the compacted edge WAL");

        let (_ttl, sm) = reopen_semantic(dir.path());
        assert_eq!(
            sm.relations(1).unwrap().len(),
            1,
            "edges must survive reopen via the snapshot"
        );
    }

    /// relations() hides edges whose endpoint has expired (read invisibility
    /// extends to the graph surface).
    #[test]
    fn test_relations_hide_expired_endpoints() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let sm = make_semantic(Arc::clone(&db));
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        sm.store(1, "ctx", &emb).unwrap();
        sm.store(2, "ephemeral fact", &emb).unwrap();
        sm.relate(1, 2, "RELATES_TO", None).unwrap();

        sm.set_ttl_durable(2, 0).unwrap(); // expires immediately
        assert!(
            sm.relations(1).unwrap().is_empty(),
            "edges to expired endpoints must be invisible"
        );
    }

    /// THE mission query: vector NEAR + graph MATCH + scalar metadata over
    /// agent memory, end-to-end through the VelesQL bridge.
    #[test]
    fn test_mission_query_near_match_scalar_over_memory() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let memory = crate::agent::AgentMemory::with_dimension(Arc::clone(&db), 4)
            .expect("test: AgentMemory::with_dimension");
        let sm = memory.semantic();

        let close = vec![1.0_f32, 0.0, 0.0, 0.0];
        let far = vec![0.0_f32, 1.0, 0.0, 0.0];
        let tech = meta_one("category", serde_json::json!("tech"));
        let bio = meta_one("category", serde_json::json!("bio"));

        // ctx(1) relates to fact(2); ctx(3) has no relations; ctx(4) wrong category.
        sm.store_with_metadata(1, "ctx about rust", &close, &tech)
            .unwrap();
        sm.store_with_metadata(2, "fact: rust is fast", &close, &tech)
            .unwrap();
        sm.store_with_metadata(3, "ctx unrelated", &close, &tech)
            .unwrap();
        sm.store_with_metadata(4, "ctx other domain", &far, &bio)
            .unwrap();
        sm.relate(1, 2, "RELATES_TO", None).unwrap();
        sm.relate(4, 2, "RELATES_TO", None).unwrap();

        let mut params = std::collections::HashMap::new();
        params.insert("q".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]));
        let results = memory
            .query_semantic(
                "SELECT * FROM memory AS m \
                 WHERE vector NEAR $q AND category = 'tech' \
                 AND MATCH (m)-[:RELATES_TO]->(f) LIMIT 5",
                &params,
            )
            .unwrap();

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert_eq!(
            ids,
            vec![1],
            "only ctx 1 is tech AND relates to a fact; got {ids:?}"
        );
    }

    /// A user business field named `expires_at` (subscription, offer, token…)
    /// stored via `store_with_metadata` must stay plain metadata: visible in
    /// session AND after a reopen, never rebuilt into the durable TTL map.
    #[test]
    fn test_user_expires_at_metadata_survives_restart() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let past_epoch = 1_000_000_u64; // long-gone epoch seconds

        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            let meta = meta_one("expires_at", serde_json::json!(past_epoch));
            sm.store_with_metadata(1, "offer expired yesterday", &emb, &meta)
                .unwrap();
            assert!(sm.get(1).unwrap().is_some(), "fact visible in session");
        }

        let (ttl, sm) = reopen_semantic(dir.path());

        assert!(
            ttl.get(MemoryKind::Semantic, 1).is_none(),
            "user expires_at metadata must not be rebuilt into the TTL map"
        );
        assert!(
            sm.get(1).unwrap().is_some(),
            "fact with user expires_at metadata must stay alive after reopen"
        );
        let results = sm.query(&emb, 5).unwrap();
        assert!(results.iter().any(|r| r.0 == 1), "fact must stay queryable");

        // The business field itself is preserved and filterable.
        let filter = meta_one("expires_at", serde_json::json!(past_epoch));
        let filtered = sm.query_filtered(&emb, 5, &filter, 0).unwrap();
        assert_eq!(
            filtered.len(),
            1,
            "user expires_at field must be preserved as metadata"
        );
    }

    /// Same collision via `update_metadata`: merging a user `expires_at` into
    /// an existing fact must not arm a durable TTL at the next reopen.
    #[test]
    fn test_update_metadata_user_expires_at_survives_restart() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];

        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            sm.store(1, "subscription fact", &emb).unwrap();
            let updates = meta_one("expires_at", serde_json::json!(1_000_000_u64));
            sm.update_metadata(1, &updates).unwrap();
        }

        let (ttl, sm) = reopen_semantic(dir.path());

        assert!(
            ttl.get(MemoryKind::Semantic, 1).is_none(),
            "user expires_at update must not be rebuilt into the TTL map"
        );
        assert!(
            sm.get(1).unwrap().is_some(),
            "fact must stay alive after reopen"
        );
    }

    /// The reserved durable-expiry key (`_veles_expires_at`) is stripped from
    /// user metadata, mirroring how the `content` parameter owns `content`.
    #[test]
    fn test_reserved_expiry_key_stripped_from_store_metadata() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];

        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let sm = make_semantic(Arc::clone(&db));
            let meta = meta_one("_veles_expires_at", serde_json::json!(1_u64));
            sm.store_with_metadata(1, "spoof attempt", &emb, &meta)
                .unwrap();
        }

        let (ttl, sm) = reopen_semantic(dir.path());

        assert!(
            ttl.get(MemoryKind::Semantic, 1).is_none(),
            "reserved key must be stripped at store time"
        );
        assert!(sm.get(1).unwrap().is_some());
    }

    /// `update_metadata` must neither inject nor clobber the reserved durable
    /// expiry: a legitimate `store_with_ttl` expiry survives a metadata update
    /// and is rebuilt identically at reopen.
    #[test]
    fn test_update_metadata_preserves_legit_durable_ttl() {
        let dir = tempdir().unwrap();
        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        let original_expiry;

        {
            let db = Arc::new(Database::open(dir.path()).unwrap());
            let ttl = Arc::new(MemoryTtl::new());
            let sm = SemanticMemory::new(Arc::clone(&db), 4, Arc::clone(&ttl)).unwrap();
            sm.store_with_ttl(1, "mortal fact", &emb, 9_999).unwrap();
            original_expiry = ttl
                .get(MemoryKind::Semantic, 1)
                .expect("TTL tracked at store time")
                .expires_at;

            let mut updates = meta_one("tag", serde_json::json!("updated"));
            updates.insert("_veles_expires_at".to_string(), serde_json::json!(1_u64));
            sm.update_metadata(1, &updates).unwrap();
        }

        let (ttl, _sm) = reopen_semantic(dir.path());

        let entry = ttl
            .get(MemoryKind::Semantic, 1)
            .expect("durable TTL must survive a metadata update");
        assert_eq!(
            entry.expires_at, original_expiry,
            "reserved key in updates must not clobber the durable expiry"
        );
    }
}
