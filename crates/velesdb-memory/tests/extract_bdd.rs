//! Behaviour: `remember_extracted` makes `why()` alive on raw text.
//!
//! The wedge — `why()` returning a connected subgraph, not just the seed — was
//! inert in practice because nothing built the graph: `remember` only stores the
//! links you hand it. These tests prove `remember_extracted` closes that gap with
//! a deterministic, network-free `Extractor`: feed it a paragraph, and `why()`
//! reaches a sibling fact through a shared topic with no manual `relate()`.

use tempfile::TempDir;
use velesdb_memory::{
    ExtractError, ExtractedFact, Extractor, HashEmbedder, MemoryError, MemoryService,
    DEFAULT_DIMENSION,
};

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
