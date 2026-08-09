//! Regression: a TTL'd write is ONE combined store call, never two (#1641,
//! packaging follow-up #1650).
//!
//! The historical write path issued `store_with_ttl` then `update_metadata`:
//! the fact was live and already expiring between the two calls, so a short
//! TTL could lapse in the gap and the metadata write then failed with
//! `NotFound(... is expired ...)` on a fact that was valid when the caller
//! asked for it. The fix dispatches metadata+TTL as a SINGLE trait call —
//! `MemoryStore::store_with_metadata_and_ttl` — and the shipped backend
//! orders metadata BEFORE expiry, so no window exists where the fact is
//! TTL'd but metadata-less.
//!
//! Every behavioural test (`tests/ttl_bdd.rs`, `src/storage_tests.rs`) stays
//! green against a service that quietly reintroduces the two-call sequence:
//! nothing pinned the CALL SHAPE. This suite records the exact sequence of
//! [`MemoryStore`] calls the service makes and pins it at the one seam the
//! trait offers.

use std::sync::{Arc, Mutex};

use serde_json::Value;
use velesdb_memory::{
    BoundedMemoryEdges, ColumnFilter, HashEmbedder, MemoryEdge, MemoryError, MemoryService,
    MemoryStore, Metadata, Recollection, AUTO_DATE_FIELD, DEFAULT_DIMENSION,
};

/// One store call the service made, with exactly the detail the order
/// contract needs: which write primitive, and (for the combined call) which
/// metadata keys and TTL it carried.
#[derive(Debug, Clone, PartialEq)]
enum StoreCall {
    Store,
    StoreWithMetadata { keys: Vec<String> },
    StoreWithTtl { ttl: u64 },
    StoreWithMetadataAndTtl { keys: Vec<String>, ttl: u64 },
    UpdateMetadata,
    Delete,
}

/// The metadata's key set, sorted so assertions don't depend on map order.
fn sorted_keys(metadata: &Metadata) -> Vec<String> {
    let mut keys: Vec<String> = metadata.keys().cloned().collect();
    keys.sort();
    keys
}

/// A store double that RECORDS every mutating trait call, in order. It
/// overrides `store_with_metadata_and_ttl` on purpose: the trait's default
/// body reproduces the historical two-call sequence for pre-#1641 backends,
/// and recording through the default would blame the backend for a shape
/// only the SERVICE's dispatch can choose.
struct RecordingStore {
    calls: Arc<Mutex<Vec<StoreCall>>>,
}

impl RecordingStore {
    fn record(&self, call: StoreCall) {
        self.calls.lock().expect("calls lock").push(call);
    }
}

impl MemoryStore for RecordingStore {
    fn store(&self, _id: u64, _content: &str, _embedding: &[f32]) -> Result<(), MemoryError> {
        self.record(StoreCall::Store);
        Ok(())
    }

    fn store_with_metadata(
        &self,
        _id: u64,
        _content: &str,
        _embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError> {
        self.record(StoreCall::StoreWithMetadata {
            keys: sorted_keys(metadata),
        });
        Ok(())
    }

    fn store_with_ttl(
        &self,
        _id: u64,
        _content: &str,
        _embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.record(StoreCall::StoreWithTtl { ttl: ttl_seconds });
        Ok(())
    }

    fn store_with_metadata_and_ttl(
        &self,
        _id: u64,
        _content: &str,
        _embedding: &[f32],
        metadata: &Metadata,
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.record(StoreCall::StoreWithMetadataAndTtl {
            keys: sorted_keys(metadata),
            ttl: ttl_seconds,
        });
        Ok(())
    }

    fn update_metadata(&self, _id: u64, _metadata: &Metadata) -> Result<(), MemoryError> {
        self.record(StoreCall::UpdateMetadata);
        Ok(())
    }

    fn get(&self, _id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        Ok(None)
    }

    fn get_metadata(&self, _id: u64) -> Result<Option<Metadata>, MemoryError> {
        Ok(None)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        Ok(vec![None; ids.len()])
    }

    fn delete(&self, _id: u64) -> Result<(), MemoryError> {
        self.record(StoreCall::Delete);
        Ok(())
    }

    fn query_filtered(
        &self,
        _embedding: &[f32],
        _k: usize,
        _filter: &Metadata,
        _offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        Ok(Vec::new())
    }

    fn query_excluding(
        &self,
        _embedding: &[f32],
        _k: usize,
        _exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        Ok(Vec::new())
    }

    fn query_columnar(
        &self,
        _embedding: &[f32],
        _k: usize,
        _filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError> {
        Ok(Vec::new())
    }

    fn relate(&self, _from: u64, _to: u64, _relation: &str) -> Result<u64, MemoryError> {
        Ok(1)
    }

    fn relations(&self, _id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        Ok(Vec::new())
    }

    fn incoming_relations(&self, _id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        Ok(Vec::new())
    }

    fn relations_bounded(&self, _id: u64, _cap: usize) -> Result<BoundedMemoryEdges, MemoryError> {
        Ok(BoundedMemoryEdges {
            edges: Vec::new(),
            truncated: false,
        })
    }

    fn incoming_relations_bounded(
        &self,
        _id: u64,
        _cap: usize,
    ) -> Result<BoundedMemoryEdges, MemoryError> {
        Ok(BoundedMemoryEdges {
            edges: Vec::new(),
            truncated: false,
        })
    }

    fn unrelate(&self, _edge_id: u64) -> Result<bool, MemoryError> {
        Ok(false)
    }

    fn count(&self) -> usize {
        0
    }
}

/// A service over a recording store, plus the shared call log.
fn recording_service() -> (
    Arc<Mutex<Vec<StoreCall>>>,
    MemoryService<HashEmbedder, RecordingStore>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let store = RecordingStore {
        calls: Arc::clone(&calls),
    };
    (
        calls,
        MemoryService::with_store(store, HashEmbedder::new(DEFAULT_DIMENSION)),
    )
}

/// The order contract itself: at NO point in the recorded sequence may the
/// fact exist with its TTL applied but its metadata missing. At the trait
/// seam that window has exactly one spelling — a bare `store_with_ttl`
/// (fact live and expiring, metadata not yet written) or a trailing
/// `update_metadata` (the second half of the historical pair) — so both are
/// banned outright.
fn assert_no_mid_write_expiry_window(log: &[StoreCall]) {
    assert!(
        !log.iter()
            .any(|call| matches!(call, StoreCall::StoreWithTtl { .. })),
        "a TTL'd remember must never issue a bare store_with_ttl: it leaves \
         the fact live and expiring before its metadata lands — the #1641 \
         race. Recorded sequence: {log:?}"
    );
    assert!(
        !log.iter()
            .any(|call| matches!(call, StoreCall::UpdateMetadata)),
        "a TTL'd remember must never patch metadata in a second call: the \
         fact can expire in the gap and the patch then fails on a fact that \
         was valid when the caller asked for it (#1641). Recorded sequence: \
         {log:?}"
    );
}

#[test]
fn a_ttld_write_with_metadata_is_one_combined_store_call() {
    let (calls, svc) = recording_service();
    let mut meta = Metadata::new();
    meta.insert("project".to_owned(), Value::from("veles"));

    svc.remember_with_ttl(
        "the staging token rotates nightly",
        &[],
        Some(&meta),
        Some(3_600),
    )
    .expect("remember with metadata + ttl");

    let log = calls.lock().expect("calls lock").clone();
    assert_eq!(
        log.len(),
        1,
        "a metadata+TTL write is exactly ONE store call, got {log:?}"
    );
    let StoreCall::StoreWithMetadataAndTtl { keys, ttl } = &log[0] else {
        panic!(
            "the one call must be the combined store_with_metadata_and_ttl — \
             any other primitive reopens the #1641 mid-write expiry window. \
             Got {log:?}"
        )
    };
    assert_eq!(*ttl, 3_600, "the combined call carries the caller's TTL");
    assert!(
        keys.contains(&"project".to_owned()),
        "the combined call carries the caller's metadata, got keys {keys:?}"
    );
    assert!(
        keys.contains(&AUTO_DATE_FIELD.to_owned()),
        "the combined call carries the auto date stamp too, got keys {keys:?}"
    );
    assert_no_mid_write_expiry_window(&log);
}

#[test]
fn even_without_caller_metadata_a_ttld_write_takes_the_combined_call() {
    // The auto date stamp means metadata is ALWAYS present on a native
    // write, so every TTL'd remember takes the combined arm — this is
    // exactly why #1641 was not a narrow edge case, and why a bare
    // store_with_ttl showing up here would mean the stamp (and its window
    // protection) was lost.
    let (calls, svc) = recording_service();

    svc.remember_with_ttl("short-lived secret", &[], None, Some(60))
        .expect("remember with ttl only");

    let log = calls.lock().expect("calls lock").clone();
    assert_eq!(
        log.len(),
        1,
        "a TTL-only write is still exactly ONE store call, got {log:?}"
    );
    assert_eq!(
        log[0],
        StoreCall::StoreWithMetadataAndTtl {
            keys: vec![AUTO_DATE_FIELD.to_owned()],
            ttl: 60,
        },
        "the auto date stamp routes even a metadata-less TTL'd write through \
         the combined call"
    );
    assert_no_mid_write_expiry_window(&log);
}
