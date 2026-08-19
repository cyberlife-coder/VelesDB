//! Behaviour: an entity profile is BOUNDED and says when it is partial
//! (#1820, résidu 2).
//!
//! Resolving an entity used to follow every non-scaffolding edge of its hub,
//! full target content included — the same explosion class #1743 bounded on
//! the `why` walk, on a surface that fix never covered: `entity("X")` on a
//! name mentioned by thousands of facts was a constructible multi-megabyte
//! response. The caps are named constants
//! ([`velesdb_memory::limits::MAX_ENTITY_RELATIONS`],
//! [`velesdb_memory::limits::MAX_ENTITY_SCAN_EDGES`]) and the honesty
//! criterion is the walk's own: truncation is REPORTED
//! (`relations_truncated`/`relations_in_truncated`), never silent — a list
//! holding exactly the cap being otherwise indistinguishable from a cut one.
//!
//! Three behaviours, each with its refusal proven by mutation during
//! development: the resolution cap cuts and says so; an under-cap profile is
//! NOT reported truncated (the flag must never cry wolf); and a truncation
//! the STORE reports (its raw scan window) reaches the profile even when the
//! resolved list is small.

#![cfg(feature = "persistence")]

use std::fmt::Write as _;

use tempfile::TempDir;
use velesdb_memory::limits::MAX_ENTITY_RELATIONS;
use velesdb_memory::{
    BoundedMemoryEdges, FactStore, GraphStore, HashEmbedder, MemoryEdge, MemoryError,
    MemoryService, Metadata, OutlineExtractor, DEFAULT_DIMENSION,
};

/// A fresh service over a temp store. The [`TempDir`] must outlive the service.
fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open service");
    (dir, svc)
}

/// The profile of `name`, which must exist — the lookup boilerplate every
/// case shares, whatever store the service runs on.
fn profile_of<S: FactStore + GraphStore>(
    svc: &MemoryService<HashEmbedder, S>,
    name: &str,
) -> velesdb_memory::EntityProfile {
    svc.entity_profile(name)
        .expect("profile lookup")
        .unwrap_or_else(|| panic!("{name} has a hub"))
}

/// An outline passage wiring `count` typed edges out of one hub, each to a
/// distinct person.
fn hub_with_typed_edges(count: usize) -> String {
    let mut passage = String::new();
    for i in 0..count {
        writeln!(passage, "edge: Hub Corp | emploie | Person {i}").expect("write to string");
    }
    passage
}

#[test]
fn a_hub_past_the_resolution_cap_is_cut_at_the_named_budget_and_says_so() {
    let (_dir, svc) = service();
    let over_cap = MAX_ENTITY_RELATIONS + 6;
    svc.remember_extracted(&hub_with_typed_edges(over_cap), &OutlineExtractor, None)
        .expect("outline remember");

    let profile = profile_of(&svc, "hub corp");
    assert_eq!(
        profile.relations.len(),
        MAX_ENTITY_RELATIONS,
        "the outgoing list is cut at the named cap"
    );
    assert!(
        profile.relations_truncated,
        "a cut list must SAY it is partial — silence here is the defect #1820 names"
    );
    assert!(
        !profile.relations_in_truncated,
        "the incoming side is under every cap and must not cry wolf"
    );

    // The far end of one edge still sees the hub from its own side, whole.
    let person = profile_of(&svc, "person 0");
    assert_eq!(person.relations_in.len(), 1, "one edge points at person 0");
    assert!(
        !person.relations_in_truncated && !person.relations_truncated,
        "an under-cap profile carries no truncation flag"
    );
}

#[test]
fn a_profile_under_every_cap_is_not_reported_truncated() {
    let (_dir, svc) = service();
    svc.remember_extracted(
        "edge: Alice Martin | travaille chez | Wiscale",
        &OutlineExtractor,
        None,
    )
    .expect("outline remember");

    let profile = profile_of(&svc, "alice martin");
    assert_eq!(profile.relations.len(), 1);
    assert!(
        !profile.relations_truncated && !profile.relations_in_truncated,
        "an exact, complete profile must not claim to be partial"
    );
}

// --- The store's own scan truncation must reach the profile ----------------

/// A store double whose bounded OUTGOING scan always reports truncation —
/// the case where the raw window ([`MAX_ENTITY_SCAN_EDGES`]) cut before the
/// resolution cap ever filled. The inner store answers everything else.
struct ScanCutStore {
    inner: velesdb_memory::NativeStore,
}

impl FactStore for ScanCutStore {
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

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        self.inner.get(id)
    }

    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
        self.inner.get_metadata(id)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.inner.get_metadata_batch(ids)
    }

    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
        self.inner.update_metadata(id, metadata)
    }

    fn delete(&self, id: u64) -> Result<(), MemoryError> {
        self.inner.delete(id)
    }

    fn count(&self) -> usize {
        self.inner.count()
    }
}

/// The graph facet, inner-delegated except the bounded outgoing scan under
/// test. The recall and columnar facets are deliberately NOT implemented:
/// this scenario never searches, and a drift into those paths is now a
/// compile error instead of a silent delegation.
impl GraphStore for ScanCutStore {
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
        let mut bounded = self.inner.relations_bounded(id, cap)?;
        bounded.truncated = true;
        Ok(bounded)
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

#[test]
fn a_scan_window_cut_reported_by_the_store_reaches_the_profile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let native = velesdb_memory::NativeStore::open(dir.path(), DEFAULT_DIMENSION)
        .expect("open native store");
    let svc = MemoryService::with_store(
        ScanCutStore { inner: native },
        HashEmbedder::new(DEFAULT_DIMENSION),
    );
    svc.remember_extracted(
        "edge: Alice Martin | travaille chez | Wiscale",
        &OutlineExtractor,
        None,
    )
    .expect("outline remember");

    let profile = profile_of(&svc, "alice martin");
    assert_eq!(
        profile.relations.len(),
        1,
        "the resolved list itself is tiny — the cut happened in the raw scan"
    );
    assert!(
        profile.relations_truncated,
        "a truncation the store reports must reach the caller, however small \
         the resolved list — this is the OR the resolver must not drop"
    );
    assert!(
        !profile.relations_in_truncated,
        "the untouched incoming side keeps its honest false"
    );
}
