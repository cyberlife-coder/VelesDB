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

mod common;
use common::SharedTopicExtractor as StubExtractor;

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
        .expect("extract and remember")
        .ids;
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
        .expect("first")
        .ids;
    let second = svc
        .remember_extracted("y", &StubExtractor, None)
        .expect("second")
        .ids;
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

/// One fact past the embeddable cap among sound ones. The old behaviour
/// failed the WHOLE call at that fact — everything already written stayed
/// (no rollback), the graph wiring never ran, and the caller did not even
/// get the ids of what had been persisted. Inconsistent with the rest of
/// the pipeline, where a malformed triple or a blank entity is skipped
/// without being fatal: one unusable element must not cost the others.
struct OneOversizedExtractor;

impl Extractor for OneOversizedExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![
            ExtractedFact {
                text: "Alice ships the parser in Rust.".to_string(),
                entities: vec!["rust".to_string()],
            },
            ExtractedFact {
                // 4x the embeddable cap: would previously abort the call.
                text: "x".repeat(8192),
                entities: vec!["rust".to_string()],
            },
            ExtractedFact {
                text: "Bob maintains the Rust toolchain.".to_string(),
                entities: vec!["rust".to_string()],
            },
        ])
    }
}

#[test]
fn an_oversized_fact_is_skipped_not_fatal() {
    let (_dir, svc) = service();
    let outcome = svc
        .remember_extracted(
            "passage with one oversized fact",
            &OneOversizedExtractor,
            None,
        )
        .expect("one oversized fact must not fail the whole call");
    assert_eq!(
        outcome.ids.len(),
        2,
        "the two sound facts are stored; the oversized one is skipped"
    );
    assert_eq!(
        outcome.skipped_over_cap, 1,
        "the caller is told exactly how many facts were dropped, and why"
    );
}

// --- The always-available backend: `OutlineExtractor` -------------------------
//
// These prove the two contracts that had NO offline proof before it existed,
// because reaching them meant reaching a generative model over the network:
// the HIT branch of `entity_profile.relations_in`, and a non-zero
// `skipped_over_cap`. Both were declared KNOWN_GAP in the binding parity guard
// on that basis (issues #1690, #1692).

/// An entity hub is only ever born of extraction, so before this backend the
/// INCOMING half of a profile was unreachable without a live model — which is
/// precisely why every binding could drop `relations_in` unnoticed.
#[test]
fn an_outlined_edge_reaches_the_far_end_as_an_incoming_relation() {
    let (_dir, svc) = service();
    svc.remember_extracted(
        "fact: Camille works at Wiscale. | camille, wiscale\n\
         edge: Camille | works at | Wiscale",
        &velesdb_memory::OutlineExtractor,
        None,
    )
    .expect("an outlined passage extracts without a model");

    let far_end = svc
        .entity_profile("Wiscale")
        .expect("read the profile")
        .expect("the outlined object is a known entity");
    let names: Vec<&str> = far_end
        .relations_in
        .iter()
        .map(|relation| relation.predicate.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["works at"],
        "the edge LEAVES camille, so it can only be seen from wiscale by looking at what \
         points AT it — the whole point of `relations_in`"
    );
    assert!(
        far_end.relations.is_empty(),
        "and it must NOT show up as outgoing: that is the confusion the field exists to end"
    );
}

/// The mirror of the assertion above, from the other end. Without it, a
/// binding that relayed `relations_in` by copying `relations` would pass.
#[test]
fn the_outlined_subject_keeps_the_edge_as_outgoing() {
    let (_dir, svc) = service();
    svc.remember_extracted(
        "edge: Camille | works at | Wiscale",
        &velesdb_memory::OutlineExtractor,
        None,
    )
    .expect("extract");

    let subject = svc
        .entity_profile("camille")
        .expect("read the profile")
        .expect("the outlined subject is a known entity");
    assert_eq!(subject.relations.len(), 1, "outgoing, from this end");
    assert!(
        subject.relations_in.is_empty(),
        "nothing points at camille here"
    );
}

/// The oversized fact comes from the INPUT, not from a fabricated backend:
/// that is what makes this a proof about `remember_extracted` rather than
/// about a test double.
#[test]
fn an_outlined_fact_past_the_cap_is_counted_not_fatal() {
    let (_dir, svc) = service();
    let outcome = svc
        .remember_extracted(
            &format!(
                "fact: Camille ships the parser. | camille\n\
                 fact: {}\n\
                 edge: Camille | works at | Wiscale",
                "x".repeat(4096)
            ),
            &velesdb_memory::OutlineExtractor,
            None,
        )
        .expect("one oversized fact must not fail the whole call");
    assert_eq!(outcome.ids.len(), 1, "the sound fact is stored");
    assert_eq!(
        outcome.skipped_over_cap, 1,
        "and the caller is told the other one was dropped for its size"
    );
}

/// A directive the backend cannot read is an error, never a silently dropped
/// line: a graph that quietly loses half of what it was handed is worse than
/// one that refuses.
#[test]
fn a_malformed_directive_refuses_instead_of_dropping_the_line() {
    let (_dir, svc) = service();
    let err = svc
        .remember_extracted(
            "edge: Camille | works at",
            &velesdb_memory::OutlineExtractor,
            None,
        )
        .expect_err("a two-field `edge:` is not an edge");
    assert!(
        format!("{err}").contains("3 `|`-separated fields, 2 given"),
        "the refusal names what was wrong with the line, got: {err}"
    );
}

/// Attributes keep their JSON type on the way in, because `recall_where`
/// comparisons are type-strict — an age arriving as `"15"` would silently
/// never match a numeric filter.
#[test]
fn an_outlined_attribute_keeps_its_json_type() {
    let (_dir, svc) = service();
    svc.remember_extracted(
        "attr: Theo Durand | age | 15",
        &velesdb_memory::OutlineExtractor,
        None,
    )
    .expect("extract");

    let profile = svc
        .entity_profile("Theo Durand")
        .expect("read the profile")
        .expect("the outlined entity is known");
    assert_eq!(
        profile.attributes.get("age"),
        Some(&Value::from(15)),
        "a number stays a number"
    );
}
