//! Unit tests for `WasmStore`'s `MemoryStore` implementation. Runs natively
//! (`cargo test -p velesdb-wasm`) — no `wasm32` target or JS host required,
//! since `now_ms()` falls back to `SystemTime` off `wasm32`.

use super::*;
use velesdb_memory::ColumnOp;

fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

#[test]
fn test_store_and_get_roundtrip() {
    let store = WasmStore::new(4);
    store.store(1, "hello", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    let (content, embedding) = store.get(1).unwrap().expect("present");
    assert_eq!(content, "hello");
    assert_eq!(embedding, vec![1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn test_get_unknown_id_is_none() {
    let store = WasmStore::new(4);
    assert!(store.get(999).unwrap().is_none());
}

#[test]
fn test_store_with_metadata_round_trips_via_get_metadata() {
    let store = WasmStore::new(4);
    let m = meta(&[("tag", Value::String("science".to_string()))]);
    store
        .store_with_metadata(1, "photosynthesis", &[1.0, 0.0, 0.0, 0.0], &m)
        .unwrap();
    let payload = store.get_metadata(1).unwrap().expect("metadata present");
    assert_eq!(
        payload.get("tag"),
        Some(&Value::String("science".to_string()))
    );
}

#[test]
fn test_update_metadata_merges_without_dropping_content() {
    let store = WasmStore::new(4);
    store.store(1, "a fact", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    let m = meta(&[("tag", Value::String("science".to_string()))]);
    store.update_metadata(1, &m).unwrap();

    let (content, _) = store.get(1).unwrap().expect("present");
    assert_eq!(content, "a fact", "content survives a metadata-only update");
    let payload = store.get_metadata(1).unwrap().expect("metadata present");
    assert_eq!(
        payload.get("tag"),
        Some(&Value::String("science".to_string()))
    );
}

#[test]
fn test_update_metadata_unknown_id_errors() {
    let store = WasmStore::new(4);
    let err = store.update_metadata(999, &Metadata::new()).unwrap_err();
    assert!(matches!(err, MemoryError::UnknownMemory(999)));
}

#[test]
fn test_delete_removes_the_fact() {
    let store = WasmStore::new(4);
    store.store(1, "ephemeral", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.delete(1).unwrap();
    assert!(store.get(1).unwrap().is_none());
}

#[test]
fn test_delete_cascades_relations() {
    let store = WasmStore::new(4);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.relate(1, 2, "decided_in").unwrap();
    store.delete(2).unwrap();

    assert!(
        store.relations(1).unwrap().is_empty(),
        "an edge dangling off a deleted memory must not survive it"
    );
}

#[test]
fn test_relate_and_relations_round_trip() {
    let store = WasmStore::new(4);
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
fn test_relations_only_returns_outgoing_edges() {
    let store = WasmStore::new(4);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.relate(1, 2, "decided_in").unwrap();

    assert!(
        store.relations(2).unwrap().is_empty(),
        "edge is directed 1 -> 2, not 2 -> 1"
    );
}

#[test]
fn test_query_filtered_matches_exact_metadata() {
    let store = WasmStore::new(4);
    let m = meta(&[("project", Value::String("veles".to_string()))]);
    store
        .store_with_metadata(1, "auth bug", &[1.0, 0.0, 0.0, 0.0], &m)
        .unwrap();
    store.store(2, "unrelated", &[0.0, 1.0, 0.0, 0.0]).unwrap();

    let hits = store
        .query_filtered(&[1.0, 0.0, 0.0, 0.0], 5, &m, 0)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 1);
}

#[test]
fn test_query_excluding_drops_matching_metadata() {
    let store = WasmStore::new(4);
    let hub_meta = meta(&[("_veles_hub", Value::Bool(true))]);
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
    let store = WasmStore::new(4);
    let early = meta(&[("year", Value::from(2003))]);
    store
        .store_with_metadata(1, "alice was CEO", &[1.0, 0.0, 0.0, 0.0], &early)
        .unwrap();
    let late = meta(&[("year", Value::from(2020))]);
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
fn test_query_columnar_strips_reserved_keys_from_metadata() {
    // Regression: the raw payload (which carries the reserved `content` key,
    // and `_veles_expires_at` for TTL'd facts) used to be returned verbatim
    // as `Recollection::metadata` — the native backend strips reserved keys
    // and collapses an empty result to `None`, and this backend must match.
    let store = WasmStore::new(4);
    let m = meta(&[("year", Value::from(2003))]);
    store
        .store_with_metadata(1, "alice was CEO", &[1.0, 0.0, 0.0, 0.0], &m)
        .unwrap();
    store
        .store_with_ttl(2, "bob was CEO", &[1.0, 0.0, 0.0, 0.0], 3600)
        .unwrap();

    let hits = store.query_columnar(&[1.0, 0.0, 0.0, 0.0], 5, &[]).unwrap();
    let alice = hits.iter().find(|r| r.id == 1).expect("alice present");
    let metadata = alice.metadata.as_ref().expect("caller metadata survives");
    assert_eq!(metadata.get("year"), Some(&Value::from(2003)));
    assert!(
        !metadata.contains_key("content"),
        "reserved `content` key must be stripped"
    );
    let bob = hits.iter().find(|r| r.id == 2).expect("bob present");
    assert!(
        bob.metadata.is_none(),
        "a fact with only reserved keys (content + TTL) has no caller metadata"
    );
}

#[test]
fn test_relate_to_missing_endpoint_errors() {
    let store = WasmStore::new(4);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();

    let err = store.relate(1, 999, "decided_in").unwrap_err();
    assert!(matches!(err, MemoryError::UnknownMemory(999)));
    let err = store.relate(999, 1, "decided_in").unwrap_err();
    assert!(matches!(err, MemoryError::UnknownMemory(999)));
    assert!(
        store.relations(1).unwrap().is_empty(),
        "no dangling edge may survive a rejected relate"
    );
}

#[test]
fn test_relations_excludes_edges_to_expired_targets() {
    // Regression: `entity_idf` divides by this degree, so counting an edge
    // into a TTL-expired fact under-weights every graph-reached fact
    // relative to the native backend (which filters expired targets).
    let store = WasmStore::new(4);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    store.relate(1, 2, "decided_in").unwrap();
    // Re-storing the target with an already-passed expiry simulates a fact
    // that expired after the edge was created (edges survive a re-store).
    store
        .store_with_ttl(2, "b", &[0.0, 1.0, 0.0, 0.0], 0)
        .unwrap();

    assert!(
        store.relations(1).unwrap().is_empty(),
        "an edge into an expired fact is dead and must not be returned"
    );
}

#[test]
fn test_query_columnar_rejects_reserved_field() {
    let store = WasmStore::new(4);
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
    let store = WasmStore::new(4);
    assert_eq!(store.count(), 0);
    store.store(1, "a", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store.store(2, "b", &[0.0, 1.0, 0.0, 0.0]).unwrap();
    assert_eq!(store.count(), 2);
}

#[test]
fn test_zero_ttl_expires_immediately() {
    let store = WasmStore::new(4);
    store
        .store_with_ttl(1, "ephemeral", &[1.0, 0.0, 0.0, 0.0], 0)
        .unwrap();
    assert!(
        store.get(1).unwrap().is_none(),
        "a 0-second TTL expires at the moment it's set"
    );
}

#[test]
fn test_positive_ttl_is_recallable_before_expiry() {
    let store = WasmStore::new(4);
    store
        .store_with_ttl(1, "not yet expired", &[1.0, 0.0, 0.0, 0.0], 3600)
        .unwrap();
    assert!(store.get(1).unwrap().is_some());
}

#[test]
fn test_expired_fact_is_excluded_from_vector_search() {
    let store = WasmStore::new(4);
    store
        .store_with_ttl(1, "expired", &[1.0, 0.0, 0.0, 0.0], 0)
        .unwrap();
    store.store(2, "live", &[1.0, 0.0, 0.0, 0.0]).unwrap();

    let hits = store
        .query_excluding(&[1.0, 0.0, 0.0, 0.0], 5, &Metadata::new())
        .unwrap();
    assert!(hits.iter().all(|h| h.0 != 1));
    assert!(hits.iter().any(|h| h.0 == 2));
}
