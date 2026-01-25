//! Tests for AgentMemory (EPIC-010/US-001)

use super::*;
use crate::Database;
use tempfile::tempdir;

/// Test: AgentMemory can be created from a Database
#[test]
fn test_agent_memory_new() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let memory = AgentMemory::new(&db);
    assert!(memory.is_ok(), "AgentMemory::new should succeed");
}

/// Test: AgentMemory provides access to SemanticMemory
#[test]
fn test_agent_memory_semantic_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let semantic = memory.semantic();
    assert!(semantic.collection_name().starts_with("_semantic"));
}

/// Test: AgentMemory provides access to EpisodicMemory
#[test]
fn test_agent_memory_episodic_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let episodic = memory.episodic();
    assert!(episodic.collection_name().starts_with("_episodic"));
}

/// Test: AgentMemory provides access to ProceduralMemory
#[test]
fn test_agent_memory_procedural_access() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();
    let memory = AgentMemory::new(&db).unwrap();

    let procedural = memory.procedural();
    assert!(procedural.collection_name().starts_with("_procedural"));
}

/// Test: Multiple AgentMemory instances share the same collections
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
