//! An autograph enrichment that RUNS and fails part-way is counted and
//! reported — not discarded.
//!
//! `autograph`'s wiring helpers propagate with `?`, so the first failing
//! write ends its stage. The caller used to discard that error with `let _`:
//! not counted, not logged, and `memory_status` reported a healthy worker
//! while the fact's graph structure was partial. The doc's "never silent"
//! covered the QUEUE (a full queue's drops) and nothing else. This suite pins
//! the other half: a wiring failure lands in `autograph_failed`, distinct
//! from `autograph_dropped`, and the fact itself is stored regardless.
//!
//! The failure is induced where the code's own comments say it happens in
//! production — on a hub write — through a store that refuses to persist an
//! entity hub (`_veles_hub` metadata) while accepting everything else.

// `NativeStore` is the backing store this suite wraps, and it is
// `persistence`-gated: the same crate-level guard `auto_date_bdd.rs` and
// `column_filter_conformance_bdd.rs` carry, so the Lint job's per-feature
// `--all-targets` loop (which compiles every integration test under
// `--features context` alone) does not try to build this one without it.
#![cfg(feature = "persistence")]

use std::sync::Arc;

use serde_json::Value;
use velesdb_memory::{
    BoundedMemoryEdges, ExtractError, ExtractedFact, Extraction, Extractor, FactStore, GraphStore,
    HashEmbedder, MemoryEdge, MemoryError, MemoryService, Metadata, NativeStore, DEFAULT_DIMENSION,
};

/// Refuses to store an entity hub; delegates everything else.
struct HubRefusingStore {
    inner: NativeStore,
}

/// The hub marker `MemoryService` stamps on every entity hub. The crate does
/// not re-export the constant (it is a storage-layer detail, not API), and
/// exporting it for one test would widen the public surface for no caller;
/// the literal IS the on-disk contract this suite wraps, so it is spelled out.
const HUB_FIELD: &str = "_veles_hub";

fn is_hub(metadata: &Metadata) -> bool {
    metadata.get(HUB_FIELD) == Some(&Value::Bool(true))
}

impl FactStore for HubRefusingStore {
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError> {
        self.inner.store(id, content, embedding)
    }
    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError> {
        if is_hub(metadata) {
            return Err(MemoryError::Extract(ExtractError::Backend(
                "simulated: the store refused to persist an entity hub".to_owned(),
            )));
        }
        self.inner
            .store_with_metadata(id, content, embedding, metadata)
    }
    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.inner
            .store_with_ttl(id, content, embedding, ttl_seconds)
    }
    fn store_with_metadata_and_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        if is_hub(metadata) {
            return Err(MemoryError::Extract(ExtractError::Backend(
                "simulated: the store refused to persist an entity hub".to_owned(),
            )));
        }
        self.inner
            .store_with_metadata_and_ttl(id, content, embedding, metadata, ttl_seconds)
    }
    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
        self.inner.update_metadata(id, metadata)
    }
    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        self.inner.get(id)
    }
    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
        self.inner.get_metadata(id)
    }
    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.inner.get_metadata_batch(ids)
    }
    fn delete(&self, id: u64) -> Result<(), MemoryError> {
        self.inner.delete(id)
    }
    fn count(&self) -> usize {
        self.inner.count()
    }
}

impl GraphStore for HubRefusingStore {
    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        self.inner.relate(from, to, relation)
    }
    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        self.inner.relations(id)
    }
    fn incoming_relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        self.inner.incoming_relations(id)
    }
    fn relations_bounded(&self, id: u64, cap: usize) -> Result<BoundedMemoryEdges, MemoryError> {
        self.inner.relations_bounded(id, cap)
    }
    fn incoming_relations_bounded(
        &self,
        id: u64,
        cap: usize,
    ) -> Result<BoundedMemoryEdges, MemoryError> {
        self.inner.incoming_relations_bounded(id, cap)
    }
    fn unrelate(&self, edge_id: u64) -> Result<bool, MemoryError> {
        self.inner.unrelate(edge_id)
    }
}

/// Always finds one topic — enough for `wire_entities` to try a hub write.
struct OneTopicExtractor;

impl Extractor for OneTopicExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![ExtractedFact {
            text: text.to_owned(),
            entities: vec!["paris".to_owned()],
        }])
    }
    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        Ok(Extraction {
            facts: self.extract(text)?,
            relations: Vec::new(),
            attributes: Vec::new(),
        })
    }
}

fn service(dir: &tempfile::TempDir) -> MemoryService<HashEmbedder, HubRefusingStore> {
    let inner = NativeStore::open(dir.path(), DEFAULT_DIMENSION).expect("open native store");
    MemoryService::with_store(
        HubRefusingStore { inner },
        HashEmbedder::new(DEFAULT_DIMENSION),
    )
    .with_autograph(Arc::new(OneTopicExtractor))
}

#[test]
fn a_wiring_failure_is_counted_and_the_fact_is_still_stored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = service(&dir);
    // No worker spawned: autograph runs INLINE, so the count is observable
    // as soon as `remember` returns.
    assert_eq!(svc.autograph_failed(), 0);

    svc.remember("Alice moved to Paris in 2024.", &[], None)
        .expect("the fact is stored even though its enrichment will fail");

    assert_eq!(
        svc.autograph_failed(),
        1,
        "one enrichment ran and its hub write failed: that is one counted failure"
    );
    assert_eq!(
        svc.autograph_dropped(),
        0,
        "nothing was dropped — the enrichment RAN; drops and failures are distinct"
    );
    assert_eq!(
        svc.fact_count(),
        1,
        "a wiring failure never loses the fact — and the refused hub is not a fact"
    );
}

#[test]
fn every_failed_enrichment_is_counted_not_just_the_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = service(&dir);
    for fact in [
        "Bob visited Paris.",
        "Carol lives in Paris.",
        "Dan left Paris.",
    ] {
        svc.remember(fact, &[], None).expect("remember");
    }
    assert_eq!(svc.autograph_failed(), 3);
    assert_eq!(
        svc.fact_count(),
        3,
        "every fact stored, every enrichment counted as failed"
    );
}
