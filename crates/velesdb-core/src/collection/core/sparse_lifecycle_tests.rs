use super::*;
use crate::index::sparse::{persistence::compact_named, SparseVector};

#[test]
fn manifested_named_index_is_discovered_without_inactive_alias() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let index = SparseInvertedIndex::new();
    index.insert(7, &SparseVector::new(vec![(3, 1.5)]));
    compact_named(dir.path(), "title", &index).expect("test: compact");

    let loaded = load_named_sparse_indexes(dir.path());
    assert_eq!(loaded.get("title").expect("title index").doc_count(), 1);
    assert!(!loaded.contains_key("title.next"));
}

#[test]
fn hidden_orphan_slot_is_not_discovered() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    std::fs::write(dir.path().join(".sparse-orphan.next.meta"), b"partial")
        .expect("test: write orphan");
    assert!(discover_named_indexes(dir.path()).is_empty());
}
