use super::*;
use tempfile::TempDir;

fn create_test_db() -> (TempDir, std::sync::Arc<VelesDatabase>) {
    let dir = TempDir::new().expect("test: create temp dir");
    let db =
        VelesDatabase::open(dir.path().to_string_lossy().to_string()).expect("test: open database");
    (dir, db)
}

#[test]
fn test_semantic_memory_new() {
    let (_dir, db) = create_test_db();
    let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");
    assert_eq!(memory.dimension(), 4);
    assert!(memory.is_empty().expect("test: is_empty on fresh memory"));
}

#[test]
fn test_semantic_memory_store_and_query() {
    let (_dir, db) = create_test_db();
    let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

    memory
        .store(1, "Test content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
        .expect("test: store knowledge fact");

    assert_eq!(memory.len().expect("test: read len"), 1);
    assert!(!memory.is_empty().expect("test: is_empty after store"));

    let results = memory
        .query(vec![0.1, 0.2, 0.3, 0.4], 5)
        .expect("test: query semantic memory");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
    assert_eq!(results[0].content, "Test content");
}

#[test]
fn test_semantic_memory_delete() {
    let (_dir, db) = create_test_db();
    let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

    memory
        .store(1, "Content".to_string(), vec![0.1, 0.2, 0.3, 0.4])
        .expect("test: store knowledge fact");
    assert_eq!(memory.len().expect("test: read len"), 1);

    memory.delete(1).expect("test: delete knowledge fact");
    assert!(memory.is_empty().expect("test: is_empty after delete"));
}

#[test]
fn test_semantic_memory_clear() {
    let (_dir, db) = create_test_db();
    let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");

    memory
        .store(1, "First".to_string(), vec![0.1, 0.2, 0.3, 0.4])
        .expect("test: store first fact");
    memory
        .store(2, "Second".to_string(), vec![0.5, 0.6, 0.7, 0.8])
        .expect("test: store second fact");
    assert_eq!(memory.len().expect("test: read len"), 2);

    memory.clear().expect("test: clear knowledge facts");
    assert!(memory.is_empty().expect("test: is_empty after clear"));
}

#[test]
fn test_semantic_memory_content_survives_reload() {
    let dir = TempDir::new().expect("test: create temp dir");
    let path = dir.path().to_string_lossy().to_string();

    // Store a fact, then drop the database handle entirely.
    {
        let db = VelesDatabase::open(path.clone()).expect("test: open database");
        let memory = VelesSemanticMemory::new(&db, 4).expect("test: construct semantic memory");
        memory
            .store(
                7,
                "Paris is the capital of France".to_string(),
                vec![0.1, 0.2, 0.3, 0.4],
            )
            .expect("test: store knowledge fact");
    }

    // Re-open the database from disk and recover the content text.
    let db = VelesDatabase::open(path).expect("test: re-open database");
    let memory = VelesSemanticMemory::new(&db, 4).expect("test: re-construct semantic memory");

    let results = memory
        .query(vec![0.1, 0.2, 0.3, 0.4], 5)
        .expect("test: query after reload");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 7);
    assert_eq!(results[0].content, "Paris is the capital of France");
}
