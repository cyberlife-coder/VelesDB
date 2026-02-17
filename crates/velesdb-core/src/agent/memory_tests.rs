//! Tests for `AgentMemory` (EPIC-010/US-001, US-002, US-003, US-004)

use super::*;
use crate::Database;
use tempfile::tempdir;

// ============================================================================
// US-001: Basic API tests
// ============================================================================

/// Test: `AgentMemory` can be created from a Database
#[test]
fn test_agent_memory_new() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let memory = AgentMemory::new(&db);
    assert!(memory.is_ok(), "AgentMemory::new should succeed");
}

/// Test: `AgentMemory` provides access to `SemanticMemory`
#[test]
fn test_agent_memory_semantic_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let semantic = memory.semantic();
    assert!(semantic.collection_name().starts_with("_semantic"));
}

/// Test: `AgentMemory` provides access to `EpisodicMemory`
#[test]
fn test_agent_memory_episodic_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let episodic = memory.episodic();
    assert!(episodic.collection_name().starts_with("_episodic"));
}

/// Test: `AgentMemory` provides access to `ProceduralMemory`
#[test]
fn test_agent_memory_procedural_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let procedural = memory.procedural();
    assert!(procedural.collection_name().starts_with("_procedural"));
}

/// Test: Multiple `AgentMemory` instances share the same collections
#[test]
fn test_agent_memory_shared_collections() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let memory1 = AgentMemory::new(&db).unwrap();
    let memory2 = AgentMemory::new(&db).unwrap();

    assert_eq!(
        memory1.semantic().collection_name(),
        memory2.semantic().collection_name()
    );
}

// ============================================================================
// US-002: SemanticMemory tests
// ============================================================================

/// Test: `SemanticMemory` can store and query facts
#[test]
fn test_semantic_store_and_query() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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

/// Test: `SemanticMemory` dimension validation
#[test]
fn test_semantic_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

    // Wrong dimension should fail
    let bad_embedding = vec![1.0, 0.0]; // Only 2 dims
    let result = memory.semantic().store(1, "test", &bad_embedding);
    assert!(result.is_err());
}

/// Test: `AgentMemory` rejects mismatched dimension when collection exists (PR #93 bug fix)
#[test]
fn test_dimension_mismatch_on_existing_collection() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    // Create memory with dimension 4
    let memory1 = AgentMemory::with_dimension(&db, 4).unwrap();
    assert_eq!(memory1.semantic().dimension(), 4);

    // Store something to ensure collection is created
    let embedding = vec![1.0, 0.0, 0.0, 0.0];
    memory1.semantic().store(1, "test", &embedding).unwrap();

    // Try to create memory with different dimension - should fail
    let result = AgentMemory::with_dimension(&db, 8);
    assert!(result.is_err());

    // Creating with same dimension should succeed
    let memory2 = AgentMemory::with_dimension(&db, 4);
    assert!(memory2.is_ok());
}

// ============================================================================
// US-003: EpisodicMemory tests
// ============================================================================

/// Test: `EpisodicMemory` can record and retrieve events
#[test]
fn test_episodic_record_and_recent() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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

/// Test: `EpisodicMemory` similarity search
#[test]
fn test_episodic_recall_similar() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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

/// Test: `ProceduralMemory` can learn and recall procedures
#[test]
fn test_procedural_learn_and_recall() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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

/// Test: `ProceduralMemory` reinforcement
#[test]
fn test_procedural_reinforce() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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

/// Test: `ProceduralMemory` `min_confidence` filter
#[test]
fn test_procedural_min_confidence_filter() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::with_dimension(&db, 4).unwrap();

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
