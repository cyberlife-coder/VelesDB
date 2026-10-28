//! Unit tests for `NativeStore`'s `MemoryStore` implementation.

use super::*;
use crate::model::ColumnOp;
use crate::{ExtractError, ExtractedFact, Extractor, Link};
use crate::{HashEmbedder, MemoryService};
use parking_lot::{Condvar, Mutex};
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[derive(Default)]
struct RecordingObserver {
    keys: Mutex<Vec<DirtyKey>>,
    failure: Mutex<Option<String>>,
}

impl MutationObserver for RecordingObserver {
    fn before_mutation(&self, key: DirtyKey) -> Result<(), MemoryError> {
        let failure = self.failure.lock().clone();
        if let Some(message) = failure {
            return Err(MemoryError::MigrationCapture(message));
        }
        self.keys.lock().push(key);
        Ok(())
    }
}

struct BlockingObserver {
    entered: mpsc::Sender<()>,
    released: Mutex<bool>,
    wake: Condvar,
}

struct FailOnObserver {
    key: DirtyKey,
    seen: Mutex<Vec<DirtyKey>>,
}

impl MutationObserver for FailOnObserver {
    fn before_mutation(&self, key: DirtyKey) -> Result<(), MemoryError> {
        self.seen.lock().push(key);
        if key == self.key {
            return Err(MemoryError::MigrationCapture("injected refusal".to_owned()));
        }
        Ok(())
    }
}

struct OneEntityExtractor;

impl Extractor for OneEntityExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![ExtractedFact {
            text: text.to_owned(),
            entities: vec!["migration".to_owned()],
        }])
    }
}

impl MutationObserver for BlockingObserver {
    fn before_mutation(&self, _key: DirtyKey) -> Result<(), MemoryError> {
        let _ = self.entered.send(());
        let mut released = self.released.lock();
        while !*released {
            self.wake.wait(&mut released);
        }
        Ok(())
    }
}

fn store() -> (tempfile::TempDir, NativeStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = NativeStore::open(dir.path(), 4).expect("open store");
    (dir, store)
}

#[test]
fn every_native_mutation_is_classified_before_the_source_write() {
    let (_dir, store) = store();
    let observer = Arc::new(RecordingObserver::default());
    store
        .set_mutation_observer(Some(observer.clone()))
        .expect("install observer");
    let mut metadata = Metadata::new();
    metadata.insert("tag".to_owned(), Value::from("test"));

    store.store(1, "one", &[1.0, 0.0, 0.0, 0.0]).unwrap();
    store
        .store_with_metadata(2, "two", &[0.0, 1.0, 0.0, 0.0], &metadata)
        .unwrap();
    store
        .store_with_ttl(3, "three", &[0.0, 0.0, 1.0, 0.0], 60)
        .unwrap();
    store
        .store_with_metadata_and_ttl(4, "four", &[0.0, 0.0, 0.0, 1.0], &metadata, 60)
        .unwrap();
    store.update_metadata(1, &metadata).unwrap();
    let edge_id = store.relate(1, 2, "supports").unwrap();
    assert!(store.unrelate_from(1, edge_id).unwrap());
    store.delete(1).unwrap();

    assert_eq!(
        *observer.keys.lock(),
        vec![
            DirtyKey::Fact(1),
            DirtyKey::Fact(2),
            DirtyKey::Fact(3),
            DirtyKey::Fact(4),
            DirtyKey::Fact(1),
            DirtyKey::OutgoingEdges(1),
            DirtyKey::OutgoingEdges(1),
            DirtyKey::Fact(1),
        ]
    );
}

#[test]
fn observer_failure_prevents_the_source_mutation() {
    let (_dir, store) = store();
    let observer = Arc::new(RecordingObserver::default());
    *observer.failure.lock() = Some("journal unavailable".to_owned());
    store
        .set_mutation_observer(Some(observer))
        .expect("install observer");

    let error = store
        .store(7, "must not persist", &[1.0, 0.0, 0.0, 0.0])
        .expect_err("capture refusal must be returned");

    assert!(matches!(error, MemoryError::MigrationCapture(_)));
    assert!(store.get(7).unwrap().is_none());
}

#[test]
fn exclusive_capture_activation_waits_for_the_in_flight_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service =
        Arc::new(MemoryService::open(dir.path(), HashEmbedder::new(4)).expect("open service"));
    let (entered_tx, entered_rx) = mpsc::channel();
    let blocking = Arc::new(BlockingObserver {
        entered: entered_tx,
        released: Mutex::new(false),
        wake: Condvar::new(),
    });
    service
        .install_mutation_observer(Some(blocking.clone()))
        .expect("install observer");

    let writer_service = Arc::clone(&service);
    let writer = std::thread::spawn(move || writer_service.remember("first", &[], None));
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("mutation reached observer");

    let next = Arc::new(RecordingObserver::default());
    let activation_service = Arc::clone(&service);
    let activation_observer = Arc::clone(&next);
    let (activated_tx, activated_rx) = mpsc::channel();
    let activation = std::thread::spawn(move || {
        let result = activation_service.install_mutation_observer(Some(activation_observer));
        let _ = activated_tx.send(result);
    });
    assert!(activated_rx
        .recv_timeout(Duration::from_millis(50))
        .is_err());

    *blocking.released.lock() = true;
    blocking.wake.notify_all();
    writer.join().expect("writer thread").expect("remember");
    let error = activated_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("activation result")
        .expect_err("second observer");
    assert!(error.to_string().contains("already active"), "{error}");
    activation.join().expect("activation thread");

    let second = service.remember("second", &[], None).expect("remember");
    assert!(next.keys.lock().is_empty());
    assert_ne!(second, 0);
}

#[test]
fn service_mutation_surfaces_reach_the_native_observer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = MemoryService::open(dir.path(), HashEmbedder::new(4)).expect("open service");
    let observer = Arc::new(RecordingObserver::default());
    service
        .install_mutation_observer(Some(observer.clone()))
        .expect("install observer");
    let mut metadata = Metadata::new();
    metadata.insert("kind".to_owned(), Value::from("contract"));

    let first = service
        .remember_with_ttl("first", &[], Some(&metadata), Some(60))
        .expect("remember ttl");
    service.feedback(first, true).expect("feedback");
    let second = service.remember("second", &[], None).expect("remember");
    service.relate(first, second, "supports").expect("relate");
    service
        .unrelate(first, second, "supports")
        .expect("unrelate");
    service.forget(second).expect("forget");

    assert_eq!(
        *observer.keys.lock(),
        vec![
            DirtyKey::Fact(first),
            DirtyKey::Fact(first),
            DirtyKey::Fact(second),
            DirtyKey::OutgoingEdges(first),
            DirtyKey::OutgoingEdges(first),
            DirtyKey::Fact(second),
        ]
    );
}

#[test]
fn autograph_and_extraction_writes_are_captured() {
    let extraction_dir = tempfile::tempdir().expect("tempdir");
    let extraction_service =
        MemoryService::open(extraction_dir.path(), HashEmbedder::new(4)).expect("open service");
    let extraction_observer = Arc::new(RecordingObserver::default());
    extraction_service
        .install_mutation_observer(Some(extraction_observer.clone()))
        .expect("install observer");
    extraction_service
        .remember_extracted("extracted fact", &OneEntityExtractor, None)
        .expect("remember extracted");
    assert!(extraction_observer
        .keys
        .lock()
        .iter()
        .any(|key| matches!(key, DirtyKey::OutgoingEdges(_))));

    let autograph_dir = tempfile::tempdir().expect("tempdir");
    let autograph_service = MemoryService::open(autograph_dir.path(), HashEmbedder::new(4))
        .expect("open service")
        .with_autograph(Arc::new(OneEntityExtractor));
    let autograph_observer = Arc::new(RecordingObserver::default());
    autograph_service
        .install_mutation_observer(Some(autograph_observer.clone()))
        .expect("install observer");
    autograph_service
        .remember("autograph fact", &[], None)
        .expect("remember autograph");
    assert!(autograph_observer
        .keys
        .lock()
        .iter()
        .any(|key| matches!(key, DirtyKey::OutgoingEdges(_))));
}

#[test]
fn captured_link_failure_also_captures_the_rollback_delete() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = MemoryService::open(dir.path(), HashEmbedder::new(4)).expect("open service");
    let target = service
        .remember("target", &[], None)
        .expect("remember target");
    let source = crate::id::stable_id("source");
    let observer = Arc::new(FailOnObserver {
        key: DirtyKey::OutgoingEdges(source),
        seen: Mutex::new(Vec::new()),
    });
    service
        .install_mutation_observer(Some(observer.clone()))
        .expect("install observer");

    let error = service
        .remember(
            "source",
            &[Link {
                target,
                relation: "supports".to_owned(),
            }],
            None,
        )
        .expect_err("edge capture refusal must fail remember");

    assert!(matches!(error, MemoryError::MigrationCapture(_)));
    assert_eq!(
        *observer.seen.lock(),
        vec![
            DirtyKey::Fact(source),
            DirtyKey::OutgoingEdges(source),
            DirtyKey::Fact(source),
        ]
    );
    assert_eq!(service.fact_count(), 1, "rollback removed the source fact");
}

#[cfg(feature = "context")]
#[test]
fn context_source_and_event_writes_are_captured() {
    use crate::context::{CompilePolicy, CompileRequest, ContextCompiler, ContextFragment};

    let dir = tempfile::tempdir().expect("tempdir");
    let service = MemoryService::open(dir.path(), HashEmbedder::new(4)).expect("open service");
    let observer = Arc::new(RecordingObserver::default());
    service
        .install_mutation_observer(Some(observer.clone()))
        .expect("install observer");
    let request = CompileRequest {
        query: "migration".to_owned(),
        fragments: vec![ContextFragment {
            path: None,
            id: None,
            content: "source body".to_owned(),
            kind: None,
            priority: None,
            metadata: None,
            media: None,
        }],
        project: Some("velesdb".to_owned()),
        target_model: None,
        token_budget: 1_000,
        memory_scope: None,
        policy: None,
    };

    service
        .compile_context(&ContextCompiler::new(CompilePolicy::default()), &request)
        .expect("compile context");

    let keys = observer.keys.lock();
    assert_eq!(keys.len(), 2, "one source fact and one event fact");
    assert!(keys.iter().all(|key| matches!(key, DirtyKey::Fact(_))));
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
