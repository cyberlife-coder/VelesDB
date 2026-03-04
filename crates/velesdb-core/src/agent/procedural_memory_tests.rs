//! Unit tests for ProceduralMemory (EPIC-010/US-004).

#[cfg(test)]
mod tests {
    use super::super::procedural_memory::ProceduralMemory;
    use super::super::reinforcement::FixedRate;
    use super::super::ttl::MemoryTtl;
    use crate::Database;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn steps(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("step {i}")).collect()
    }

    fn make_procedural(db: Arc<Database>) -> ProceduralMemory {
        ProceduralMemory::new(db, 4, Arc::new(MemoryTtl::new()))
            .expect("ProceduralMemory::new failed")
    }

    // ── Basic API ──────────────────────────────────────────────────────────────

    #[test]
    fn test_collection_name_prefixed() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));
        assert!(pm.collection_name().starts_with("_procedural"));
    }

    // ── learn() / recall() ────────────────────────────────────────────────────

    #[test]
    fn test_learn_and_recall() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "greet_user", &steps(3), Some(&emb), 0.8)
            .unwrap();

        let results = pm.recall(&emb, 1, 0.0).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 1);
        assert_eq!(results[0].name, "greet_user");
        assert_eq!(results[0].steps.len(), 3);
        assert!((results[0].confidence - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_recall_without_embedding_uses_zero_vector() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        pm.learn(1, "no-vec procedure", &steps(2), None, 0.6)
            .unwrap();

        let zero = vec![0.0_f32; 4];
        let results = pm.recall(&zero, 1, 0.0).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_recall_min_confidence_filters_below_threshold() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "low-conf", &steps(1), Some(&emb), 0.2).unwrap();

        let results_high = pm.recall(&emb, 5, 0.5).unwrap();
        assert!(
            results_high.iter().all(|r| r.id != 1),
            "procedure below min_confidence must not appear"
        );

        let results_low = pm.recall(&emb, 5, 0.1).unwrap();
        assert!(
            results_low.iter().any(|r| r.id == 1),
            "procedure above min_confidence must appear"
        );
    }

    #[test]
    fn test_recall_ranks_most_similar_first() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb_target = vec![1.0_f32, 0.0, 0.0, 0.0];
        let emb_other = vec![0.0_f32, 1.0, 0.0, 0.0];
        pm.learn(1, "target", &steps(1), Some(&emb_target), 0.9)
            .unwrap();
        pm.learn(2, "other", &steps(1), Some(&emb_other), 0.9)
            .unwrap();

        let results = pm.recall(&emb_target, 2, 0.0).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, 1, "most similar procedure must rank first");
    }

    // ── reinforce() ───────────────────────────────────────────────────────────

    #[test]
    fn test_reinforce_success_raises_confidence() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "trainable", &steps(1), Some(&emb), 0.5)
            .unwrap();
        pm.reinforce(1, true).unwrap();

        let results = pm.recall(&emb, 1, 0.0).unwrap();
        assert!(
            results[0].confidence > 0.5,
            "confidence should increase after positive reinforcement"
        );
    }

    #[test]
    fn test_reinforce_failure_lowers_confidence() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "punishable", &steps(1), Some(&emb), 0.8)
            .unwrap();
        pm.reinforce(1, false).unwrap();

        let results = pm.recall(&emb, 1, 0.0).unwrap();
        assert!(
            results[0].confidence < 0.8,
            "confidence should decrease after negative reinforcement"
        );
    }

    #[test]
    fn test_reinforce_with_custom_strategy() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "custom-strategy", &steps(1), Some(&emb), 0.5)
            .unwrap();

        // FixedRate with delta = 0.2 should increase confidence by 0.2 on success.
        let strategy = FixedRate::new(0.2, 0.1);
        pm.reinforce_with_strategy(1, true, &strategy).unwrap();

        let results = pm.recall(&emb, 1, 0.0).unwrap();
        assert!(
            (results[0].confidence - 0.7).abs() < 0.01,
            "FixedRate(0.2) on confidence 0.5 should produce 0.7"
        );
    }

    // ── list_all() ────────────────────────────────────────────────────────────

    #[test]
    fn test_list_all_returns_all_stored_procedures() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        for i in 1u64..=4 {
            pm.learn(i, &format!("proc_{i}"), &steps(1), Some(&emb), 0.5)
                .unwrap();
        }

        let all = pm.list_all().unwrap();
        assert_eq!(all.len(), 4);
    }

    // ── delete() ──────────────────────────────────────────────────────────────

    #[test]
    fn test_delete_removes_procedure() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "to delete", &steps(1), Some(&emb), 0.7)
            .unwrap();
        pm.delete(1).unwrap();

        let all = pm.list_all().unwrap();
        assert!(all.iter().all(|p| p.id != 1));
    }

    // ── Dimension validation ───────────────────────────────────────────────────

    #[test]
    fn test_learn_dimension_mismatch_rejected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db)); // dim = 4

        let bad_emb = vec![1.0_f32, 0.0]; // dim = 2
        let result = pm.learn(1, "bad", &steps(1), Some(&bad_emb), 0.5);
        assert!(result.is_err());
    }

    #[test]
    fn test_recall_dimension_mismatch_rejected() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db)); // dim = 4

        let bad_query = vec![0.5_f32]; // dim = 1
        let result = pm.recall(&bad_query, 1, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_new_detects_dimension_mismatch_on_existing_collection() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());

        let _pm = ProceduralMemory::new_from_db(Arc::clone(&db), 4).unwrap();

        let result = ProceduralMemory::new_from_db(Arc::clone(&db), 8);
        assert!(result.is_err());
    }

    // ── TTL ───────────────────────────────────────────────────────────────────

    #[test]
    fn test_learn_with_ttl_zero_expires_immediately() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn_with_ttl(77, "ephemeral", &steps(1), Some(&emb), 0.8, 0)
            .unwrap();

        let all = pm.list_all().unwrap();
        assert!(
            all.iter().all(|p| p.id != 77),
            "TTL-0 procedure must not appear in list_all()"
        );
    }

    #[test]
    fn test_learn_with_positive_ttl_still_visible() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn_with_ttl(8, "long-lived", &steps(1), Some(&emb), 0.6, 9_999)
            .unwrap();

        let all = pm.list_all().unwrap();
        assert!(all.iter().any(|p| p.id == 8));
    }

    // ── Serialize / Deserialize ────────────────────────────────────────────────

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let dir1 = tempdir().unwrap();
        let db1 = Arc::new(Database::open(dir1.path()).unwrap());
        let pm1 = make_procedural(Arc::clone(&db1));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm1.learn(1, "proc_a", &steps(2), Some(&emb), 0.7).unwrap();
        pm1.learn(2, "proc_b", &steps(3), Some(&emb), 0.9).unwrap();
        let bytes = pm1.serialize().unwrap();

        let dir2 = tempdir().unwrap();
        let db2 = Arc::new(Database::open(dir2.path()).unwrap());
        let pm2 = make_procedural(Arc::clone(&db2));
        pm2.deserialize(&bytes).unwrap();

        let all = pm2.list_all().unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<u64> = all.iter().map(|p| p.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }

    #[test]
    fn test_deserialize_empty_bytes_is_noop() {
        let dir = tempdir().unwrap();
        let db = Arc::new(Database::open(dir.path()).unwrap());
        let pm = make_procedural(Arc::clone(&db));

        let emb = vec![1.0_f32, 0.0, 0.0, 0.0];
        pm.learn(1, "existing", &steps(1), Some(&emb), 0.5).unwrap();

        pm.deserialize(&[]).unwrap();

        let all = pm.list_all().unwrap();
        assert_eq!(all.len(), 1);
    }
}
