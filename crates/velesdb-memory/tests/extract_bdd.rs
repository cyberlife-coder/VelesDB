//! Behaviour: `remember_extracted` makes `why()` alive on raw text.
//!
//! The wedge — `why()` returning a connected subgraph, not just the seed — was
//! inert in practice because nothing built the graph: `remember` only stores the
//! links you hand it. These tests prove `remember_extracted` closes that gap with
//! a deterministic, network-free `Extractor`: feed it a paragraph, and `why()`
//! reaches a sibling fact through a shared topic with no manual `relate()`.

use serde_json::Value;
use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, Extractor, HashEmbedder, MemoryError, MemoryService, Metadata,
    DEFAULT_DIMENSION,
};

/// Build a one-key metadata map for tests.
fn meta(key: &str, value: Value) -> Metadata {
    let mut m = Metadata::new();
    m.insert(key.to_string(), value);
    m
}

/// A canned extractor: two facts that share the topic `rust` (and nothing else),
/// so the only path from one to the other is through the shared hub.
struct StubExtractor;

impl Extractor for StubExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![
            ExtractedFact {
                text: "Alice ships the parser in Rust.".to_string(),
                entities: vec!["rust".to_string(), "parser".to_string()],
            },
            ExtractedFact {
                text: "Bob maintains the Rust toolchain.".to_string(),
                entities: vec!["rust".to_string()],
            },
        ])
    }
}

/// An extractor that always fails, to check the error path is surfaced.
struct FailingExtractor;

impl Extractor for FailingExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Err(ExtractError::Backend("model offline".to_string()))
    }
}

/// A fresh service over a temp store. The returned [`TempDir`] must outlive the
/// service — dropping it deletes the store out from under the open handle.
fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION))
        .expect("open service");
    (dir, svc)
}

#[test]
fn remember_extracted_stores_every_fact() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");
    assert_eq!(ids.len(), 2, "both extracted facts are stored");
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn why_traverses_the_auto_built_graph() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");

    // A query closest to the first fact; with an empty graph `why` would return
    // only that seed. The auto-wired fact↔topic edges must reach the sibling.
    let explanation = svc.why("parser shipped in rust", 2, None).expect("why");

    assert!(
        explanation.nodes.len() > 1,
        "graph is alive: why() reaches beyond the seed, got {} node(s)",
        explanation.nodes.len()
    );
    let reaches_sibling = explanation
        .nodes
        .iter()
        .any(|node| node.content.contains("Bob"));
    assert!(
        reaches_sibling,
        "why() hops through the shared `rust` topic to Bob's fact: {:?}",
        explanation
            .nodes
            .iter()
            .map(|node| node.content.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn shared_topic_collapses_onto_one_hub() {
    let (_dir, svc) = service();
    // Two separate calls mentioning the same topic must not spawn two hubs:
    // entity hubs are content-addressed, so the second call reuses the first.
    let first = svc
        .remember_extracted("x", &StubExtractor, None)
        .expect("first");
    let second = svc
        .remember_extracted("y", &StubExtractor, None)
        .expect("second");
    // Same canned facts → same stable fact ids → idempotent.
    assert_eq!(first, second, "identical facts are idempotent across calls");
}

#[test]
fn recall_excludes_entity_hubs() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");
    // `rust` is both a stored topic hub and a word in the facts; unfiltered
    // recall must return the facts, never the internal `Entity: rust` hub.
    let hits = svc.recall("rust", 8, None).expect("recall");
    assert!(!hits.is_empty(), "the facts are recalled");
    assert!(
        hits.iter().all(|hit| !hit.content.starts_with("Entity:")),
        "recall must not surface entity hubs: {:?}",
        hits.iter().map(|hit| &hit.content).collect::<Vec<_>>()
    );
}

#[test]
fn recall_where_with_no_filters_excludes_entity_hubs_like_plain_recall() {
    // Same regression family as the empty-map case below: `recall_where(q,
    // k, &[])` used to hit `query_columnar` directly — a bare vector search
    // with no hub exclusion — instead of behaving like the plain `recall`
    // it semantically is when no column predicate narrows it.
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");
    let hits = svc.recall_where("rust", 8, &[]).expect("recall_where");
    assert!(!hits.is_empty(), "the facts are still recalled");
    assert!(
        hits.iter().all(|hit| !hit.content.starts_with("Entity:")),
        "recall_where with no filters must exclude hubs: {:?}",
        hits.iter().map(|hit| &hit.content).collect::<Vec<_>>()
    );
}

#[test]
fn recall_with_an_empty_filter_map_excludes_entity_hubs_like_no_filter() {
    // Regression: `Some({})` (the natural `{}` idiom at the JS boundary) used
    // to take the include-filter path, whose "a filter can never match a hub"
    // shortcut only holds for NON-empty filters — an empty one matches every
    // payload, so internal `Entity:` hubs ranked as results. It must behave
    // exactly like `None`, mirroring `recall_fused`'s `matches_filter`.
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");
    let hits = svc
        .recall("rust", 8, Some(&Metadata::new()))
        .expect("recall");
    assert!(!hits.is_empty(), "the facts are still recalled");
    assert!(
        hits.iter().all(|hit| !hit.content.starts_with("Entity:")),
        "an empty filter map must exclude hubs exactly like no filter: {:?}",
        hits.iter().map(|hit| &hit.content).collect::<Vec<_>>()
    );
}

#[test]
fn why_seed_is_a_fact_not_a_hub() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");
    let explanation = svc.why("rust", 2, None).expect("why");
    assert!(!explanation.nodes.is_empty(), "why finds a seed");
    assert!(
        !explanation.nodes[0].content.starts_with("Entity:"),
        "the seed (primary answer) must be a real fact, got {:?}",
        explanation.nodes[0].content
    );
}

#[test]
fn empty_text_is_rejected() {
    let (_dir, svc) = service();
    assert!(matches!(
        svc.remember_extracted("   ", &StubExtractor, None),
        Err(MemoryError::EmptyFact)
    ));
}

#[test]
fn extractor_failure_is_surfaced() {
    let (_dir, svc) = service();
    assert!(matches!(
        svc.remember_extracted("anything", &FailingExtractor, None),
        Err(MemoryError::Extract(ExtractError::Backend(_)))
    ));
}

#[test]
fn user_metadata_kind_entity_is_not_excluded() {
    // The hub marker is the reserved `_veles_hub`, NOT `kind`, so a caller may
    // legitimately use kind="entity" in its own taxonomy without its fact being
    // silently dropped from recall by the hub-exclusion filter.
    let (_dir, svc) = service();
    let id = svc
        .remember(
            "Orders entity is processed nightly",
            &[],
            Some(&meta("kind", Value::String("entity".to_string()))),
        )
        .expect("remember with kind=entity");
    let hits = svc
        .recall("orders entity nightly", 8, None)
        .expect("recall");
    assert!(
        hits.iter().any(|h| h.id == id),
        "a user fact tagged kind=entity must still be recalled"
    );
}

#[test]
fn reserved_veles_key_is_rejected() {
    let (_dir, svc) = service();
    // A caller may not set a `_veles_`-namespaced system key (e.g. forge a hub).
    assert!(matches!(
        svc.remember("sneaky", &[], Some(&meta("_veles_hub", Value::Bool(true)))),
        Err(MemoryError::ReservedKey(k)) if k == "_veles_hub"
    ));
    // `content` is reserved too.
    assert!(matches!(
        svc.recall(
            "q",
            5,
            Some(&meta("content", Value::String("x".to_string())))
        ),
        Err(MemoryError::ReservedKey(_))
    ));
}

#[test]
fn reingesting_the_same_text_does_not_duplicate_edges() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("first ingest");
    let after_first = svc.why("parser shipped in rust", 2, None).expect("why 1");
    // Re-ingest identical text: facts and hubs are deterministic, so the graph
    // must be unchanged — not gain a second parallel about/mentions edge.
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("second ingest");
    let after_second = svc.why("parser shipped in rust", 2, None).expect("why 2");
    assert_eq!(
        after_first.edges.len(),
        after_second.edges.len(),
        "re-ingestion must be idempotent: no duplicate edges"
    );
}
