//! Tests for AgentMemory (EPIC-010/US-001, US-002, US-003, US-004)

use super::*;
use crate::Database;
use std::sync::Arc;
use tempfile::tempdir;

// ============================================================================
// US-001: Basic API tests
// ============================================================================

/// Test: AgentMemory can be created from a Database
#[test]
fn test_agent_memory_new() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory = AgentMemory::new(Arc::clone(&db));
    assert!(memory.is_ok(), "AgentMemory::new should succeed");
}

/// Test: AgentMemory provides access to SemanticMemory
#[test]
fn test_agent_memory_semantic_access() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::new(Arc::clone(&db)).unwrap();

    let semantic = memory.semantic();
    assert!(semantic.collection_name().starts_with("_semantic"));
}

/// Test: AgentMemory provides access to EpisodicMemory
#[test]
fn test_agent_memory_episodic_access() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::new(Arc::clone(&db)).unwrap();

    let episodic = memory.episodic();
    assert!(episodic.collection_name().starts_with("_episodic"));
}

/// Test: AgentMemory provides access to ProceduralMemory
#[test]
fn test_agent_memory_procedural_access() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::new(Arc::clone(&db)).unwrap();

    let procedural = memory.procedural();
    assert!(procedural.collection_name().starts_with("_procedural"));
}

/// Test: Multiple AgentMemory instances share the same collections
#[test]
fn test_agent_memory_shared_collections() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory1 = AgentMemory::new(Arc::clone(&db)).unwrap();
    let memory2 = AgentMemory::new(Arc::clone(&db)).unwrap();

    assert_eq!(
        memory1.semantic().collection_name(),
        memory2.semantic().collection_name()
    );
}

// ============================================================================
// US-002: SemanticMemory tests
// ============================================================================

/// Test: SemanticMemory can store and query facts
#[test]
fn test_semantic_store_and_query() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Store a fact
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory
        .semantic()
        .store(1, "The sky is blue", &embedding)
        .unwrap();

    // Query should find it
    let results = memory.semantic().query(&embedding, 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1); // ID
    assert!(results[0].2.contains("blue")); // Content
}

/// Test: SemanticMemory dimension validation
#[test]
fn test_semantic_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Wrong dimension should fail
    let bad_embedding = vec![1.0, 0.0]; // Only 2 dims
    let result = memory.semantic().store(1, "test", &bad_embedding);
    assert!(result.is_err());
}

/// Test: AgentMemory rejects mismatched dimension when collection exists (PR #93 bug fix)
#[test]
fn test_dimension_mismatch_on_existing_collection() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    // Create memory with dimension 4
    let memory1 = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();
    assert_eq!(memory1.semantic().dimension(), 4);

    // Store something to ensure collection is created
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory1.semantic().store(1, "test", &embedding).unwrap();

    // Try to create memory with different dimension - should fail
    let result = AgentMemory::with_dimension(Arc::clone(&db), 8);
    assert!(result.is_err());

    // Creating with same dimension should succeed
    let memory2 = AgentMemory::with_dimension(Arc::clone(&db), 4);
    assert!(memory2.is_ok());
}

// ============================================================================
// US-003: EpisodicMemory tests
// ============================================================================

/// Test: EpisodicMemory can record and retrieve events
#[test]
fn test_episodic_record_and_recent() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Record events with timestamps
    memory.episodic().record(1, "Event A", 1000, None).unwrap();
    memory.episodic().record(2, "Event B", 2000, None).unwrap();
    memory.episodic().record(3, "Event C", 3000, None).unwrap();

    // Get recent events (should be ordered by timestamp desc)
    let events = memory.episodic().recent(2, None).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, 3); // Most recent first (Event C)
    assert_eq!(events[1].0, 2); // Then Event B
}

/// Test: EpisodicMemory similarity search
#[test]
fn test_episodic_recall_similar() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Record events with embeddings
    let emb1 = vec![1.0, 0.0, 0.0, 0.0];
    let emb2 = vec![0.0, 1.0, 0.0, 0.0];
    memory
        .episodic()
        .record(1, "Similar to query", 1000, Some(&emb1))
        .unwrap();
    memory
        .episodic()
        .record(2, "Different event", 2000, Some(&emb2))
        .unwrap();

    // Query with similar embedding
    let results = memory.episodic().recall_similar(&emb1, 2).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].0, 1); // Most similar should be first
}

// ============================================================================
// US-004: ProceduralMemory tests
// ============================================================================

/// Test: ProceduralMemory can learn and recall procedures
#[test]
fn test_procedural_learn_and_recall() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Learn a procedure
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["Step 1".to_string(), "Step 2".to_string()];
    memory
        .procedural()
        .learn(1, "Test Procedure", &steps, Some(&embedding), 0.8)
        .unwrap();

    // Recall should find it
    let results = memory.procedural().recall(&embedding, 1, 0.0).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
    assert_eq!(results[0].name, "Test Procedure");
    assert_eq!(results[0].steps.len(), 2);
    assert!((results[0].confidence - 0.8).abs() < 0.01);
}

/// Test: ProceduralMemory reinforcement
#[test]
fn test_procedural_reinforce() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    // Learn a procedure with initial confidence
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["Step 1".to_string()];
    memory
        .procedural()
        .learn(1, "Reinforce Test", &steps, Some(&embedding), 0.5)
        .unwrap();

    // Reinforce positively
    memory.procedural().reinforce(1, true).unwrap();

    // Check confidence increased
    let results = memory.procedural().recall(&embedding, 1, 0.0).unwrap();
    assert!((results[0].confidence - 0.6).abs() < 0.01); // 0.5 + 0.1 = 0.6
}

/// Test: ProceduralMemory min_confidence filter
#[test]
fn test_procedural_min_confidence_filter() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["Step".to_string()];

    // Learn procedure with low confidence
    memory
        .procedural()
        .learn(1, "Low Confidence", &steps, Some(&embedding), 0.3)
        .unwrap();

    // Query with high min_confidence should return empty
    let results = memory.procedural().recall(&embedding, 1, 0.5).unwrap();
    assert!(results.is_empty());

    // Query with low min_confidence should return the procedure
    let results = memory.procedural().recall(&embedding, 1, 0.2).unwrap();
    assert_eq!(results.len(), 1);
}

// ============================================================================
// US-005: Eviction, TTL, Snapshots, Consolidation
// ============================================================================

/// Test: with_eviction_config sets the eviction configuration
#[test]
fn test_with_eviction_config() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let config = EvictionConfig {
        consolidation_age_threshold: 3600,
        min_confidence_threshold: 0.5,
        max_entries_per_cycle: 100,
    };

    // Builder pattern must not fail
    let memory = AgentMemory::new(Arc::clone(&db))
        .unwrap()
        .with_eviction_config(config);

    // Verify the memory is usable after configuration
    memory.semantic().store(1, "fact", &vec![0.0; 384]).unwrap();
    assert!(memory.auto_expire().is_ok());
}

/// Test: with_snapshots enables the snapshot manager
#[test]
fn test_with_snapshots() {
    let dir = tempdir().unwrap();
    let snap_dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory = AgentMemory::new(Arc::clone(&db))
        .unwrap()
        .with_snapshots(snap_dir.path().to_str().unwrap(), 5);

    // Snapshot operations should now succeed (manager is configured)
    let version = memory.snapshot().unwrap();
    assert_eq!(version, 1);
}

/// Test: set_semantic_ttl / set_episodic_ttl / set_procedural_ttl register TTLs
#[test]
fn test_set_ttl_functions() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];

    memory.semantic().store(1, "sem fact", &embedding).unwrap();
    memory
        .episodic()
        .record(2, "epi event", 1000, Some(&embedding))
        .unwrap();
    let steps = vec!["s1".to_string()];
    memory
        .procedural()
        .learn(3, "proc", &steps, Some(&embedding), 0.9)
        .unwrap();

    // Setting TTLs should not panic
    memory.set_semantic_ttl(1, 3600);
    memory.set_episodic_ttl(2, 7200);
    memory.set_procedural_ttl(3, 1800);

    // Non-expired entries should still be queryable
    let results = memory.semantic().query(&embedding, 1).unwrap();
    assert_eq!(results.len(), 1);
}

/// Test: auto_expire removes entries whose TTL has elapsed
#[test]
fn test_auto_expire_removes_expired() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory
        .semantic()
        .store(1, "will expire", &embedding)
        .unwrap();

    // Set a TTL of 0 seconds — expires immediately
    memory.set_semantic_ttl(1, 0);

    let result = memory.auto_expire().unwrap();
    // #1041: each expired key is namespaced by subsystem, so only the semantic
    // counter increments — episodic/procedural are not touched by a semantic TTL.
    assert_eq!(result.semantic_expired, 1);
    assert_eq!(result.episodic_expired, 0);
    assert_eq!(result.procedural_expired, 0);

    // Verify the entry is actually gone from semantic memory
    let results = memory.semantic().query(&embedding, 1).unwrap();
    assert!(results.is_empty());
}

/// Test: auto_expire returns zero counts when nothing is expired
#[test]
fn test_auto_expire_nothing_expired() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory.semantic().store(1, "keep me", &embedding).unwrap();

    // TTL far in the future — should not expire
    memory.set_semantic_ttl(1, 999_999);

    let result = memory.auto_expire().unwrap();
    assert_eq!(result.semantic_expired, 0);
    assert_eq!(result.episodic_expired, 0);
    assert_eq!(result.procedural_expired, 0);
    assert_eq!(result.episodic_consolidated, 0);
}

/// Test: evict_low_confidence_procedures removes low-confidence entries
#[test]
fn test_evict_low_confidence_procedures() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["step".to_string()];

    // Low confidence — should be evicted
    memory
        .procedural()
        .learn(1, "weak", &steps, Some(&embedding), 0.1)
        .unwrap();
    // High confidence — should survive
    memory
        .procedural()
        .learn(2, "strong", &steps, Some(&embedding), 0.9)
        .unwrap();

    let evicted = memory.evict_low_confidence_procedures(0.5).unwrap();
    assert_eq!(evicted, 1);

    // Only the high-confidence procedure should remain
    let remaining = memory.procedural().list_all().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, 2);
}

/// Test: evict_low_confidence_procedures returns 0 when all above threshold
#[test]
fn test_evict_low_confidence_none_evicted() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["step".to_string()];

    memory
        .procedural()
        .learn(1, "good", &steps, Some(&embedding), 0.8)
        .unwrap();
    memory
        .procedural()
        .learn(2, "also good", &steps, Some(&embedding), 0.7)
        .unwrap();

    let evicted = memory.evict_low_confidence_procedures(0.5).unwrap();
    assert_eq!(evicted, 0);
}

/// Test: snapshot and load_latest_snapshot round-trip
#[test]
fn test_snapshot_round_trip() {
    let dir = tempdir().unwrap();
    let snap_dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_snapshots(snap_dir.path().to_str().unwrap(), 5);

    // Store data across all subsystems
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory.semantic().store(1, "fact one", &embedding).unwrap();
    memory
        .episodic()
        .record(2, "event one", 1000, Some(&embedding))
        .unwrap();
    let steps = vec!["step1".to_string()];
    memory
        .procedural()
        .learn(3, "proc one", &steps, Some(&embedding), 0.8)
        .unwrap();

    // Create snapshot
    let version = memory.snapshot().unwrap();
    assert!(version >= 1);

    // Load it back
    let loaded_version = memory.load_latest_snapshot().unwrap();
    assert_eq!(loaded_version, version);
}

/// Test: snapshot without manager returns an error
#[test]
fn test_snapshot_without_manager_errors() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::new(Arc::clone(&db)).unwrap();

    // No snapshot manager configured — should fail
    assert!(memory.snapshot().is_err());
    assert!(memory.load_latest_snapshot().is_err());
    assert!(memory.list_snapshot_versions().is_err());
}

/// Test: list_snapshot_versions returns created versions
#[test]
fn test_list_snapshot_versions() {
    let dir = tempdir().unwrap();
    let snap_dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_snapshots(snap_dir.path().to_str().unwrap(), 10);

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory.semantic().store(1, "fact", &embedding).unwrap();

    // Create two snapshots
    let v1 = memory.snapshot().unwrap();
    let v2 = memory.snapshot().unwrap();

    let versions = memory.list_snapshot_versions().unwrap();
    assert!(versions.contains(&v1));
    assert!(versions.contains(&v2));
    assert_eq!(versions.len(), 2);
}

/// Test: consolidate_old_episodes migrates old events to semantic memory
#[test]
fn test_consolidate_old_episodes_via_auto_expire() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    // Use a very small consolidation threshold (1 second) so past events qualify
    let config = EvictionConfig {
        consolidation_age_threshold: 1,
        min_confidence_threshold: 0.1,
        max_entries_per_cycle: 1000,
    };
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_eviction_config(config);

    // Record an episode with a timestamp far in the past
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory
        .episodic()
        .record(1, "ancient event", 100, Some(&embedding))
        .unwrap();

    // auto_expire should consolidate the old episode into semantic memory
    let result = memory.auto_expire().unwrap();
    assert_eq!(result.episodic_consolidated, 1);

    // The event should now be findable in semantic memory
    let sem_results = memory.semantic().query(&embedding, 1).unwrap();
    assert_eq!(sem_results.len(), 1);
    assert!(sem_results[0].2.contains("ancient event"));
}

// ============================================================================
// #1041..#1043: P0 data-loss / deadlock regression coverage
// ============================================================================

/// #1041: a TTL on semantic id=5 must not expire a live episodic id=5.
/// `auto_expire` after expiry deletes only the semantic row; the episodic
/// event survives and the counters are exact.
#[test]
fn test_auto_expire_no_cross_subsystem_expiry() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    // Disable consolidation so this test isolates TTL cross-expiry only.
    let config = EvictionConfig {
        consolidation_age_threshold: 0,
        ..EvictionConfig::default()
    };
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_eviction_config(config);

    let embedding = vec![1.0, 0.0, 0.0, 0.0];

    // Same numeric id (5) in two subsystems.
    memory
        .semantic()
        .store(5, "semantic fact", &embedding)
        .unwrap();
    memory
        .episodic()
        .record(5, "live episodic event", 1_000, Some(&embedding))
        .unwrap();

    // Expire ONLY the semantic id 5.
    memory.set_semantic_ttl(5, 0);

    let result = memory.auto_expire().unwrap();
    assert_eq!(result.semantic_expired, 1, "semantic id 5 must expire");
    assert_eq!(
        result.episodic_expired, 0,
        "episodic id 5 must NOT be counted as expired"
    );
    assert_eq!(result.procedural_expired, 0);

    // The episodic event must SURVIVE.
    let survivor = memory.episodic().get_with_embedding(5).unwrap();
    assert!(
        survivor.is_some(),
        "episodic id 5 must survive a semantic-only TTL expiry"
    );
    assert_eq!(survivor.unwrap().0, "live episodic event");

    // The semantic fact is gone.
    assert!(memory.semantic().get(5).unwrap().is_none());
}

/// #1042: consolidation must not clobber an existing semantic fact that shares
/// the consolidated episode's numeric id. The original fact stays intact and
/// the consolidated event lands under a fresh semantic id.
#[test]
fn test_consolidation_preserves_existing_semantic_fact() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let config = EvictionConfig {
        consolidation_age_threshold: 1,
        min_confidence_threshold: 0.1,
        max_entries_per_cycle: 1000,
    };
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_eviction_config(config);

    let sem_emb = vec![1.0, 0.0, 0.0, 0.0];
    let epi_emb = vec![0.0, 1.0, 0.0, 0.0];

    // Pre-existing semantic fact at id=1.
    memory
        .semantic()
        .store(1, "irreplaceable fact", &sem_emb)
        .unwrap();
    // Old episodic event ALSO at id=1 — will be consolidated.
    memory
        .episodic()
        .record(1, "old episode", 100, Some(&epi_emb))
        .unwrap();

    let result = memory.auto_expire().unwrap();
    assert_eq!(result.episodic_consolidated, 1);

    // The original semantic fact at id=1 must still be intact (NOT overwritten).
    let original = memory.semantic().get(1).unwrap();
    assert!(original.is_some(), "original semantic fact must survive");
    assert_eq!(original.unwrap().0, "irreplaceable fact");

    // The consolidated episode is now retrievable in semantic memory under a
    // fresh id.
    let found = memory.semantic().query(&epi_emb, 5).unwrap();
    assert!(
        found.iter().any(|r| r.2.contains("old episode")),
        "consolidated episode must be stored under a fresh semantic id"
    );
}

/// #1042: exceeding the per-cycle consolidation cap surfaces a "truncated"
/// signal so the caller knows to run `auto_expire` again.
#[test]
fn test_consolidation_truncation_signal() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let config = EvictionConfig {
        consolidation_age_threshold: 1,
        min_confidence_threshold: 0.1,
        max_entries_per_cycle: 2, // tiny cap to force truncation
    };
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_eviction_config(config);

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    // Record more old episodes than the cap.
    for id in 1u64..=5 {
        let ts = 100 + i64::try_from(id).unwrap();
        memory
            .episodic()
            .record(id, "ancient", ts, Some(&embedding))
            .unwrap();
    }

    let result = memory.auto_expire().unwrap();
    assert_eq!(
        result.episodic_consolidated, 2,
        "cap limits this cycle to 2"
    );
    assert!(
        result.consolidation_truncated,
        "truncated signal must be set when more old episodes remain"
    );

    // A second pass drains more and (with 3 left, still > cap=2) stays truncated.
    let result2 = memory.auto_expire().unwrap();
    assert_eq!(result2.episodic_consolidated, 2);
    assert!(result2.consolidation_truncated);

    // Final pass: 1 left, fits under the cap, no longer truncated.
    let result3 = memory.auto_expire().unwrap();
    assert_eq!(result3.episodic_consolidated, 1);
    assert!(!result3.consolidation_truncated);
}

/// #1043: snapshot round-trip must restore DATA and TTL state, not just the
/// version number. Restore into a fresh AgentMemory and assert both.
#[test]
fn test_snapshot_round_trip_restores_data_and_ttl() {
    let dir = tempdir().unwrap();
    let snap_dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_snapshots(snap_dir.path().to_str().unwrap(), 5);

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory
        .semantic()
        .store(1, "persisted fact", &embedding)
        .unwrap();
    memory
        .episodic()
        .record(2, "persisted event", 1_000, Some(&embedding))
        .unwrap();
    // A long-lived TTL on the semantic fact so it survives but is tracked.
    memory.set_semantic_ttl(1, 9_999);

    let version = memory.snapshot().unwrap();

    // Restore into a SEPARATE database/memory to prove the snapshot carries
    // state, not just the in-memory handle.
    let dir2 = tempdir().unwrap();
    let db2 = Arc::new(Database::open(dir2.path()).unwrap());
    let memory2 = AgentMemory::with_dimension(Arc::clone(&db2), 4)
        .unwrap()
        .with_snapshots(snap_dir.path().to_str().unwrap(), 5);

    let loaded = memory2.load_snapshot_version(version);
    assert!(loaded.is_ok());

    // DATA restored.
    let fact = memory2.semantic().get(1).unwrap();
    assert_eq!(fact.unwrap().0, "persisted fact");
    let event = memory2.episodic().get_with_embedding(2).unwrap();
    assert_eq!(event.unwrap().0, "persisted event");

    // TTL state restored under the SEMANTIC namespace (still live, not expired).
    let after = memory2.auto_expire().unwrap();
    assert_eq!(
        after.semantic_expired, 0,
        "restored TTL is far-future, fact must not expire"
    );
    assert!(
        memory2.semantic().get(1).unwrap().is_some(),
        "fact with restored future TTL must survive auto_expire"
    );
}

/// #1045: `auto_expire` now populates `procedural_evicted` using the configured
/// `min_confidence_threshold` (previously a dead field on both ends).
#[test]
fn test_auto_expire_populates_procedural_evicted() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());

    let config = EvictionConfig {
        consolidation_age_threshold: 0, // isolate eviction
        min_confidence_threshold: 0.5,
        max_entries_per_cycle: 1000,
    };
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .with_eviction_config(config);

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["s".to_string()];
    // Below threshold -> evicted.
    memory
        .procedural()
        .learn(1, "weak", &steps, Some(&embedding), 0.2)
        .unwrap();
    // Above threshold -> kept.
    memory
        .procedural()
        .learn(2, "strong", &steps, Some(&embedding), 0.9)
        .unwrap();

    let result = memory.auto_expire().unwrap();
    assert_eq!(
        result.procedural_evicted, 1,
        "one low-confidence procedure evicted"
    );

    let remaining = memory.procedural().list_all().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, 2);
}

/// Reinforcing a procedure by an id obtained from `recall` must update that
/// same procedure (no id drift between recall and reinforce).
#[test]
fn test_reinforce_by_recalled_id_updates_same_procedure() {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    let memory = AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    let steps = vec!["step".to_string()];
    memory
        .procedural()
        .learn(99, "recall me", &steps, Some(&embedding), 0.5)
        .unwrap();

    // Obtain the id via recall, then reinforce by that id.
    let recalled = memory.procedural().recall(&embedding, 1, 0.0).unwrap();
    assert_eq!(recalled.len(), 1);
    let id = recalled[0].id;
    assert_eq!(id, 99);

    memory.procedural().reinforce(id, true).unwrap();

    let after = memory.procedural().recall(&embedding, 1, 0.0).unwrap();
    assert_eq!(after[0].id, 99);
    assert!(
        (after[0].confidence - 0.6).abs() < 0.01,
        "reinforce(recalled_id) must raise the same procedure's confidence"
    );
}
