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
    memory.episodic().record(2, "epi event", 1000, Some(&embedding)).unwrap();
    let steps = vec!["s1".to_string()];
    memory.procedural().learn(3, "proc", &steps, Some(&embedding), 0.9).unwrap();

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
    memory.semantic().store(1, "will expire", &embedding).unwrap();

    // Set a TTL of 0 seconds — expires immediately
    memory.set_semantic_ttl(1, 0);

    let result = memory.auto_expire().unwrap();
    // auto_expire tries delete on all three subsystems per expired ID;
    // delete succeeds even for missing IDs, so all three counters increment.
    assert_eq!(result.semantic_expired, 1);
    assert!(result.episodic_expired <= 1);
    assert!(result.procedural_expired <= 1);

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
    memory.procedural().learn(1, "weak", &steps, Some(&embedding), 0.1).unwrap();
    // High confidence — should survive
    memory.procedural().learn(2, "strong", &steps, Some(&embedding), 0.9).unwrap();

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

    memory.procedural().learn(1, "good", &steps, Some(&embedding), 0.8).unwrap();
    memory.procedural().learn(2, "also good", &steps, Some(&embedding), 0.7).unwrap();

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
    memory.episodic().record(2, "event one", 1000, Some(&embedding)).unwrap();
    let steps = vec!["step1".to_string()];
    memory.procedural().learn(3, "proc one", &steps, Some(&embedding), 0.8).unwrap();

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
