use super::*;

#[test]
fn test_semantic_memory_new() {
    let memory = SemanticMemory::new(384).unwrap();
    assert_eq!(memory.dimension(), 384);
    assert!(memory.is_empty());
}

#[test]
fn test_semantic_memory_store_and_len() {
    let mut memory = SemanticMemory::new(4).unwrap();
    let embedding = vec![0.1, 0.2, 0.3, 0.4];

    memory.store(1, "Test content", &embedding).unwrap();

    assert_eq!(memory.len(), 1);
    assert!(!memory.is_empty());
}

#[test]
fn test_semantic_memory_content_in_payload() {
    let mut memory = SemanticMemory::new(4).unwrap();
    let embedding = vec![0.1, 0.2, 0.3, 0.4];

    memory.store(1, "Paris is the capital", &embedding).unwrap();

    assert_eq!(memory.content_for(1), "Paris is the capital");
}

#[test]
fn test_semantic_memory_delete() {
    let mut memory = SemanticMemory::new(4).unwrap();
    let embedding = vec![0.1, 0.2, 0.3, 0.4];

    memory.store(1, "Test content", &embedding).unwrap();
    assert_eq!(memory.len(), 1);

    let removed = memory.delete(1);
    assert!(removed);
    assert!(memory.is_empty());
}

#[test]
fn test_semantic_memory_clear() {
    let mut memory = SemanticMemory::new(4).unwrap();
    let embedding = vec![0.1, 0.2, 0.3, 0.4];

    memory.store(1, "Content 1", &embedding).unwrap();
    memory.store(2, "Content 2", &embedding).unwrap();
    assert_eq!(memory.len(), 2);

    memory.clear();
    assert!(memory.is_empty());
}
