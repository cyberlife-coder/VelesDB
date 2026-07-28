//! Unit tests for `NativeStore`'s `MemoryStore` implementation.

use super::*;
use crate::model::ColumnOp;

fn store() -> (tempfile::TempDir, NativeStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = NativeStore::open(dir.path(), 4).expect("open store");
    (dir, store)
}

#[test]
fn test_store_and_get_roundtrip() {
    let (_dir, store) = store();
    store.store(1, "hello", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    let (content, embedding) = store.get(1).unwrap().expect("present");
    assert_eq!(content, "hello");
    assert_eq!(embedding, vec![1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn test_store_with_metadata_round_trips_via_get_metadata() {
    let (_dir, store) = store();
    let mut meta = Metadata::new();
    meta.insert("tag".to_string(), Value::String("science".to_string()));
    store
        .store_with_metadata(1, "photosynthesis", &[1.0, 0.0, 0.0, 0.0], &meta)
        .unwrap();
    let payload = store.get_metadata(1).unwrap().expect("metadata present");
    assert_eq!(
        payload.get("tag"),
        Some(&Value::String("science".to_string()))
    );
}

#[test]
fn test_delete_removes_the_fact() {
    let (_dir, store) = store();
    store.store(1, "ephemeral", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.delete(1).unwrap();
    assert!(store.get(1).unwrap().is_none());
}

#[test]
fn test_relate_and_relations_round_trip() {
    let (_dir, store) = store();
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.relate(1, 2, "decided_in").unwrap();
    let edges = store.relations(1).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from, 1);
    assert_eq!(edges[0].to, 2);
    assert_eq!(edges[0].relation, "decided_in");
}

#[test]
fn test_query_filtered_matches_exact_metadata() {
    let (_dir, store) = store();
    let mut meta = Metadata::new();
    meta.insert("project".to_string(), Value::String("veles".to_string()));
    store
        .store_with_metadata(1, "auth bug", &[1.0, 0.0, 0.0, 0.0], &meta)
        .unwrap();
    store.store(2, "unrelated", &[0.0, 1.0, 0.0, 0.0]).unwrap();

    let hits = store
        .query_filtered(&[1.0, 0.0, 0.0, 0.0], 5, &meta, 0)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 1);
}

#[test]
fn test_query_excluding_drops_matching_metadata() {
    let (_dir, store) = store();
    let mut hub_meta = Metadata::new();
    hub_meta.insert("_veles_hub".to_string(), Value::Bool(true));
    store
        .store_with_metadata(1, "Entity: rust", &[1.0, 0.0, 0.0, 0.0], &hub_meta)
        .unwrap();
    store
        .store(2, "a real fact", &[1.0, 0.0, 0.0, 0.0])
        .unwrap();

    let hits = store
        .query_excluding(&[1.0, 0.0, 0.0, 0.0], 5, &hub_meta)
        .unwrap();
    assert!(hits.iter().all(|h| h.0 != 1), "hub must be excluded");
    assert!(hits.iter().any(|h| h.0 == 2));
}

#[test]
fn test_query_columnar_applies_range_predicate() {
    let (_dir, store) = store();
    let mut early = Metadata::new();
    early.insert("year".to_string(), Value::from(2003));
    store
        .store_with_metadata(1, "alice was CEO", &[1.0, 0.0, 0.0, 0.0], &early)
        .unwrap();
    let mut late = Metadata::new();
    late.insert("year".to_string(), Value::from(2020));
    store
        .store_with_metadata(2, "bob was CEO", &[1.0, 0.0, 0.0, 0.0], &late)
        .unwrap();

    let filters = vec![ColumnFilter {
        field: "year".to_string(),
        op: ColumnOp::Le,
        value: Value::from(2010),
    }];
    let hits = store
        .query_columnar(&[1.0, 0.0, 0.0, 0.0], 5, &filters)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, 1);
}

#[test]
fn test_query_columnar_rejects_invalid_field() {
    let (_dir, store) = store();
    let filters = vec![ColumnFilter {
        field: "content".to_string(),
        op: ColumnOp::Eq,
        value: Value::from(1),
    }];
    let err = store
        .query_columnar(&[1.0, 0.0, 0.0, 0.0], 5, &filters)
        .expect_err("reserved field must be rejected");
    assert!(matches!(err, MemoryError::InvalidFilter(_)));
}

#[test]
fn test_count_reflects_live_facts() {
    let (_dir, store) = store();
    assert_eq!(store.count(), 0);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    assert_eq!(store.count(), 2);
}

/// A TTL'd write must carry BOTH the metadata and the durable expiry, and
/// must not be able to expire while it is being written.
///
/// The historical order — `store_with_ttl` then `update_metadata` — left the
/// fact live and already expiring between the two calls: a short TTL could
/// lapse in the gap, and the metadata write then failed with
/// `NotFound(... is expired ...)` on a fact that was valid when the caller
/// asked for it. Observed in the wild with a 1 s TTL under load.
///
/// The order is now reversed — the fact is stored with its metadata and NO
/// expiry, then the expiry is applied — so there is no window in which it can
/// expire mid-write.
#[test]
fn test_store_with_metadata_and_ttl_survives_a_short_expiry() {
    let (_dir, store) = store();
    let mut meta = Metadata::new();
    meta.insert("project".to_string(), Value::from("veles"));

    store
        .store_with_metadata_and_ttl(1, "short-lived", &[1.0, 0.0, 0.0, 0.0], &meta, 60)
        .expect("a metadata+ttl write must succeed");

    let stored = store
        .get_metadata(1)
        .expect("get_metadata")
        .expect("the fact must exist");

    assert_eq!(
        stored.get("project"),
        Some(&Value::from("veles")),
        "the caller metadata must survive the write that also set the expiry"
    );
    assert!(
        stored.keys().any(|k| k.contains("expires")),
        "the durable expiry must be persisted by the same write; keys: {:?}",
        stored.keys().collect::<Vec<_>>()
    );
}

/// A zero TTL deletes, exactly like the TTL-only path — the combined write
/// must not quietly turn "expire now" into "store forever with metadata".
#[test]
fn test_store_with_metadata_and_ttl_zero_deletes() {
    let (_dir, store) = store();
    let meta = Metadata::new();
    store.store(1, "doomed", &[1.0, 0.0, 0.0, 0.0]).unwrap();

    store
        .store_with_metadata_and_ttl(1, "doomed", &[1.0, 0.0, 0.0, 0.0], &meta, 0)
        .expect("a zero ttl must not error");

    assert!(
        store.get(1).expect("get").is_none(),
        "ttl_seconds == 0 must delete, matching store_with_ttl"
    );
}
