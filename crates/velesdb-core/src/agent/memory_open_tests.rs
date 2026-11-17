//! Tests for `AgentMemory::open_existing`: opening the memory a read needs
//! without creating it.

use super::*;
use crate::Database;
use std::sync::Arc;
use tempfile::tempdir;

fn fresh_db() -> (tempfile::TempDir, Arc<Database>) {
    let dir = tempdir().unwrap();
    let db = Arc::new(Database::open(dir.path()).unwrap());
    (dir, db)
}

/// A database that never held memory has nothing to open, and opening it
/// creates nothing: a read must not write.
#[test]
fn open_existing_on_a_fresh_database_is_none_and_creates_nothing() {
    let (_dir, db) = fresh_db();

    let memory = AgentMemory::open_existing(Arc::clone(&db), 4).unwrap();

    assert!(memory.is_none());
    assert!(
        db.list_collections().is_empty(),
        "{:?}",
        db.list_collections()
    );
}

/// Once the memory exists, `open_existing` opens it with what it holds.
#[test]
fn open_existing_opens_the_memory_a_write_created() {
    let (_dir, db) = fresh_db();
    let embedding = [0.1_f32, 0.2, 0.3, 0.4];
    AgentMemory::with_dimension(Arc::clone(&db), 4)
        .unwrap()
        .semantic()
        .store(1, "fact", &embedding)
        .unwrap();

    let memory = AgentMemory::open_existing(Arc::clone(&db), 4)
        .unwrap()
        .expect("the memory exists");

    let hits = memory.semantic().query(&embedding, 1).unwrap();
    assert_eq!(hits[0].0, 1);
}

/// Part of the memory missing is refused, and the missing part is not
/// created behind the read's back.
#[test]
fn open_existing_refuses_an_incomplete_memory_without_completing_it() {
    let (_dir, db) = fresh_db();
    SemanticMemory::new_from_db(Arc::clone(&db), 4).unwrap();

    let err = AgentMemory::open_existing(Arc::clone(&db), 4)
        .err()
        .expect("an incomplete memory must be refused");

    assert!(matches!(err, AgentMemoryError::NotFound(_)), "{err:?}");
    assert_eq!(
        db.list_collections().len(),
        1,
        "{:?}",
        db.list_collections()
    );
}

/// The existing collections' dimension still has to match.
#[test]
fn open_existing_checks_the_dimension() {
    let (_dir, db) = fresh_db();
    AgentMemory::with_dimension(Arc::clone(&db), 4).unwrap();

    let err = AgentMemory::open_existing(Arc::clone(&db), 8)
        .err()
        .expect("a dimension mismatch must be refused");

    assert!(
        matches!(err, AgentMemoryError::DimensionMismatch { .. }),
        "{err:?}"
    );
}
