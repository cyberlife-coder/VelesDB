//! BDD integration tests for `recall_fused` — vector+graph score fusion.
//!
//! This proves the tri-engine ranking measured on HotpotQA/TimeQA/LoCoMo
//! (`examples/multihop`, `examples/timeqa`, `examples/locomo`) now lives in
//! the shipped recall path: `recall_fused` surfaces a graph-connected fact
//! that pure vector similarity misses, without evicting genuinely strong
//! vector hits or leaking internal entity-hub scaffolding.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::service;
use serde_json::Value;
use velesdb_memory::{
    ExtractError, ExtractedFact, Extractor, FusionOptions, HashEmbedder, Link, MemoryError,
    MemoryService, RerankError, Reranker,
};

const DECISION: &str = "we chose parking_lot to avoid lock poisoning";

/// Same chain as `why_bdd.rs`: decision -[`decided_in`]-> PR -[`tracked_by`]-> ticket,
/// with the ticket wording dissimilar to the decision so only the graph reaches it.
fn seeded_chain(svc: &MemoryService<HashEmbedder>) -> (u64, u64, u64) {
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    let pr = svc
        .remember("PR #42 swaps the mutex implementation", &[], None)
        .expect("remember pr");
    let ticket = svc
        .remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .expect("remember ticket");
    svc.relate(decision, pr, "decided_in")
        .expect("relate decision->pr");
    svc.relate(pr, ticket, "tracked_by")
        .expect("relate pr->ticket");
    (decision, pr, ticket)
}

/// A canned extractor: two facts sharing only the topic `rust`, so the sole
/// path from one to the other is through the auto-wired entity hub — the
/// same fixture `extract_bdd.rs` uses for `why()`.
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

const SEED_TEXT: &str = "seed fact anchors the walk";

/// One seed fact tagged with both a rare topic (linking only one sibling) and
/// a common topic (linking three siblings) — isolates the idf weighting
/// itself: every sibling is reached at the same hop, so only the connecting
/// hub's specificity can differentiate their rank.
struct WeightedExtractor;

impl Extractor for WeightedExtractor {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![
            ExtractedFact {
                text: SEED_TEXT.to_string(),
                entities: vec!["raretopic".to_string(), "commontopic".to_string()],
            },
            ExtractedFact {
                text: "specific sibling via the rare topic".to_string(),
                entities: vec!["raretopic".to_string()],
            },
            ExtractedFact {
                text: "common sibling one via the common topic".to_string(),
                entities: vec!["commontopic".to_string()],
            },
            ExtractedFact {
                text: "common sibling two via the common topic".to_string(),
                entities: vec!["commontopic".to_string()],
            },
            ExtractedFact {
                text: "common sibling three via the common topic".to_string(),
                entities: vec!["commontopic".to_string()],
            },
        ])
    }
}

/// A reranker that reverses the candidate order — proof the pool actually
/// reaches the caller's hook and its output order is honored, not ignored.
struct ReverseReranker;

impl Reranker for ReverseReranker {
    fn rerank(
        &self,
        _query: &str,
        mut candidates: Vec<velesdb_memory::Recollection>,
    ) -> Result<Vec<velesdb_memory::Recollection>, RerankError> {
        candidates.reverse();
        Ok(candidates)
    }
}

/// A reranker that always fails, to check the error path is surfaced.
struct FailingReranker;

impl Reranker for FailingReranker {
    fn rerank(
        &self,
        _query: &str,
        _candidates: Vec<velesdb_memory::Recollection>,
    ) -> Result<Vec<velesdb_memory::Recollection>, RerankError> {
        Err(RerankError::Backend("cross-encoder offline".to_string()))
    }
}

// --- Nominal ---------------------------------------------------------------

#[test]
fn recall_fused_reranked_lets_reranker_reorder_results() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let plain = svc
        .recall_fused(DECISION, 3, None, FusionOptions::default())
        .expect("recall_fused");
    let reranked = svc
        .recall_fused_reranked(
            DECISION,
            3,
            None,
            FusionOptions::default(),
            &ReverseReranker,
        )
        .expect("recall_fused_reranked");

    assert_eq!(plain.len(), reranked.len());
    let plain_ids: Vec<u64> = plain.iter().map(|r| r.id).collect();
    let reranked_ids: Vec<u64> = reranked.iter().map(|r| r.id).collect();
    assert_ne!(
        plain_ids, reranked_ids,
        "the reranker's reordering must be honored, not ignored"
    );
    assert_eq!(
        reranked_ids,
        plain_ids.into_iter().rev().collect::<Vec<_>>(),
        "reversing reranker output must exactly reverse the fused order"
    );
}

#[test]
fn recall_fused_reranked_truncates_to_k_after_reranking_the_wider_pool() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let reranked = svc
        .recall_fused_reranked(
            DECISION,
            1,
            None,
            FusionOptions::default(),
            &ReverseReranker,
        )
        .expect("recall_fused_reranked");
    assert_eq!(reranked.len(), 1);
}

#[test]
fn recall_fused_pool_override_narrows_the_candidate_set() {
    let (_dir, svc) = service();
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    svc.remember("PR #42 swaps the mutex implementation", &[], None)
        .expect("remember pr");
    svc.remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .expect("remember ticket");

    let default_pool = svc
        .recall_fused(DECISION, 5, None, FusionOptions::default())
        .expect("recall_fused");
    assert_eq!(default_pool.len(), 3, "default pool oversamples past k");

    let narrowed = svc
        .recall_fused(
            DECISION,
            5,
            None,
            FusionOptions {
                pool: Some(1),
                ..FusionOptions::default()
            },
        )
        .expect("recall_fused");
    assert_eq!(
        narrowed.len(),
        1,
        "pool: Some(1) caps the candidate set at 1, even though k=5"
    );
    assert_eq!(
        narrowed[0].id, decision,
        "the sole admitted candidate must be the top vector hit"
    );
}

#[test]
fn recall_fused_round_trips_caller_metadata_on_a_graph_reached_fact() {
    let (_dir, svc) = service();
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    let mut ticket_meta = velesdb_memory::Metadata::new();
    ticket_meta.insert(
        "occurred_at".to_string(),
        Value::String("2026-01-05".to_string()),
    );
    let ticket = svc
        .remember("EPIC-317 xyzzy quux frobnicate", &[], Some(&ticket_meta))
        .expect("remember ticket with metadata");
    svc.relate(decision, ticket, "decided_in")
        .expect("relate decision->ticket");

    let fused = svc
        .recall_fused(DECISION, 5, None, FusionOptions::default())
        .expect("recall_fused");
    let hit = fused
        .iter()
        .find(|r| r.id == ticket)
        .expect("ticket present");

    assert_eq!(
        hit.metadata
            .as_ref()
            .and_then(|m| m.get("occurred_at"))
            .cloned(),
        Some(Value::String("2026-01-05".to_string())),
        "a graph-reached fact's metadata must round-trip through recall_fused too"
    );
}

#[test]
fn recall_fused_ranks_graph_reached_fact_above_a_vector_tied_distractor() {
    let (_dir, svc) = service();
    let decision = svc
        .remember(DECISION, &[], None)
        .expect("remember decision");
    let ticket = svc
        .remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .expect("remember ticket");
    let distractor = svc
        .remember("the quarterly report is due next Friday", &[], None)
        .expect("remember distractor");
    svc.relate(decision, ticket, "decided_in")
        .expect("relate decision->ticket");

    // Vector similarity alone cannot distinguish the graph-connected ticket
    // from the unrelated distractor: neither shares wording with the query,
    // so both score identically under recall().
    let vector_only = svc.recall(DECISION, 3, None).expect("recall");
    let score_of = |id: u64| vector_only.iter().find(|h| h.id == id).map(|h| h.score);
    assert_eq!(
        score_of(ticket),
        score_of(distractor),
        "vector recall alone ties the connected fact with the unrelated one"
    );

    // The graph breaks the tie: fused ranks the connected ticket strictly
    // above the disconnected distractor — the promotion vector search alone
    // cannot do.
    let fused = svc
        .recall_fused(DECISION, 3, None, FusionOptions::default())
        .expect("recall_fused");
    let rank_of = |id: u64| fused.iter().position(|r| r.id == id);
    assert!(
        rank_of(ticket).expect("graph-reached ticket must be present")
            < rank_of(distractor).expect("distractor must be present"),
        "the graph-reached fact must outrank the vector-tied distractor"
    );
}

#[test]
fn recall_fused_keeps_the_top_vector_hit_first() {
    let (_dir, svc) = service();
    let (decision, _pr, _ticket) = seeded_chain(&svc);

    let fused = svc
        .recall_fused(DECISION, 3, None, FusionOptions::default())
        .expect("recall_fused");
    assert_eq!(
        fused[0].id, decision,
        "the exact-match seed keeps the top rank"
    );
}

#[test]
fn recall_fused_promotes_sibling_fact_via_shared_entity_hub() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");

    // Bob's fact shares no wording with Alice's — pure vector recall at k=1
    // returns only Alice's fact.
    let vector_only = svc
        .recall("Alice ships the parser in Rust.", 1, None)
        .expect("recall");
    assert!(vector_only[0].content.contains("Alice"));

    let fused = svc
        .recall_fused(
            "Alice ships the parser in Rust.",
            2,
            None,
            FusionOptions::default(),
        )
        .expect("recall_fused");
    assert!(
        fused.iter().any(|r| r.content.contains("Bob")),
        "the shared-topic sibling fact surfaces via the 2-hop hub bridge"
    );
}

#[test]
fn recall_fused_ranks_rare_hub_sibling_above_common_hub_sibling() {
    let (_dir, svc) = service();
    svc.remember_extracted(SEED_TEXT, &WeightedExtractor, None)
        .expect("extract and remember");

    // Every sibling shares no wording with the seed query, so vector
    // similarity alone cannot rank one above another.
    let vector_only = svc.recall(SEED_TEXT, 10, None).expect("recall");
    let seed_score = vector_only[0].score;
    assert!(
        vector_only[1..].iter().all(|h| h.score < seed_score),
        "only the exact-match seed should stand out under vector search alone"
    );

    // idf weighting must rank the sibling reached through the rare,
    // single-fact hub above every sibling reached through the common hub
    // that links three facts.
    let fused = svc
        .recall_fused(SEED_TEXT, 10, None, FusionOptions::default())
        .expect("recall_fused");
    let rank_of = |needle: &str| fused.iter().position(|r| r.content.contains(needle));
    let rare_rank = rank_of("specific sibling").expect("rare-hub sibling present");
    for common in [
        "common sibling one",
        "common sibling two",
        "common sibling three",
    ] {
        let common_rank = rank_of(common).expect("common-hub sibling present");
        assert!(
            rare_rank < common_rank,
            "the rare-hub sibling must outrank {common:?} (idf weighting)"
        );
    }
}

#[test]
fn recall_fused_never_returns_entity_hubs() {
    let (_dir, svc) = service();
    svc.remember_extracted("Alice and Bob both work in Rust.", &StubExtractor, None)
        .expect("extract and remember");

    let fused = svc
        .recall_fused(
            "Alice ships the parser in Rust.",
            10,
            None,
            FusionOptions::default(),
        )
        .expect("recall_fused");
    assert!(
        fused.iter().all(|r| !r.content.starts_with("Entity: ")),
        "internal hub scaffolding is never returned, exactly like recall()"
    );
}

#[test]
fn recall_fused_via_links_argument_is_traversable() {
    let (_dir, svc) = service();
    let pr = svc
        .remember("PR #99 refactors the lock layer", &[], None)
        .expect("remember pr");
    let decision = svc
        .remember(
            DECISION,
            &[Link {
                target: pr,
                relation: "decided_in".to_owned(),
            }],
            None,
        )
        .expect("remember decision with link");

    let fused = svc
        .recall_fused(DECISION, 2, None, FusionOptions::default())
        .expect("recall_fused");
    let ids: Vec<u64> = fused.iter().map(|r| r.id).collect();
    assert!(ids.contains(&decision) && ids.contains(&pr));
}

// --- Edge --------------------------------------------------------------

#[test]
fn recall_fused_on_isolated_memory_still_returns_it_via_vector() {
    let (_dir, svc) = service();
    let lone = svc
        .remember("a fact with no relations", &[], None)
        .expect("remember");

    let fused = svc
        .recall_fused(
            "a fact with no relations",
            5,
            None,
            FusionOptions::default(),
        )
        .expect("recall_fused");
    assert_eq!(fused.len(), 1);
    assert_eq!(fused[0].id, lone);
}

#[test]
fn recall_fused_respects_k_truncation() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let fused = svc
        .recall_fused(DECISION, 1, None, FusionOptions::default())
        .expect("recall_fused");
    assert_eq!(fused.len(), 1);
}

#[test]
fn recall_fused_with_zero_hops_behaves_like_pure_vector_recall() {
    let (_dir, svc) = service();
    let (_decision, _pr, ticket) = seeded_chain(&svc);

    let opts = FusionOptions {
        hops: 0,
        ..FusionOptions::default()
    };
    let fused = svc
        .recall_fused(DECISION, 2, None, opts)
        .expect("recall_fused");
    assert!(
        fused.iter().all(|r| r.id != ticket),
        "zero hops means no graph reach, so the ticket cannot surface"
    );
}

#[test]
fn recall_fused_blank_query_is_empty() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let fused = svc
        .recall_fused("   ", 5, None, FusionOptions::default())
        .expect("recall_fused");
    assert!(fused.is_empty());
}

#[test]
fn recall_fused_zero_k_is_empty() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let fused = svc
        .recall_fused(DECISION, 0, None, FusionOptions::default())
        .expect("recall_fused");
    assert!(fused.is_empty());
}

#[test]
fn recall_fused_never_leaks_a_graph_reached_fact_outside_the_caller_filter() {
    // Regression: the graph walk used to be filter-blind past the seed, so a
    // fact outside the caller's scope (a different tenant here) could leak
    // in just by being graph-connected to the seed — a cross-tenant read.
    let (_dir, svc) = service();
    let mut alice_meta = velesdb_memory::Metadata::new();
    alice_meta.insert("tenant".to_string(), Value::String("alice".to_string()));
    let alice_fact = svc
        .remember(DECISION, &[], Some(&alice_meta))
        .expect("remember alice fact");

    let mut bob_meta = velesdb_memory::Metadata::new();
    bob_meta.insert("tenant".to_string(), Value::String("bob".to_string()));
    let bob_fact = svc
        .remember("EPIC-317 xyzzy quux frobnicate", &[], Some(&bob_meta))
        .expect("remember bob fact");
    svc.relate(alice_fact, bob_fact, "related_to")
        .expect("relate");

    let fused = svc
        .recall_fused(DECISION, 5, Some(&alice_meta), FusionOptions::default())
        .expect("recall_fused");
    assert!(
        fused.iter().all(|r| r.id != bob_fact),
        "a graph-reached fact outside the tenant filter must not leak into the result"
    );
    assert!(
        fused.iter().any(|r| r.id == alice_fact),
        "the in-scope seed fact must still be returned"
    );
}

// --- Negative ------------------------------------------------------------

#[test]
fn recall_fused_on_empty_store_is_empty() {
    let (_dir, svc) = service();

    let fused = svc
        .recall_fused("anything", 5, None, FusionOptions::default())
        .expect("recall_fused on empty store");
    assert!(fused.is_empty());
}

#[test]
fn recall_fused_rejects_reserved_filter_key() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let mut filter = velesdb_memory::Metadata::new();
    filter.insert("content".to_string(), Value::String("x".to_string()));

    let err = svc
        .recall_fused(DECISION, 5, Some(&filter), FusionOptions::default())
        .expect_err("reserved key must be rejected");
    assert!(matches!(err, MemoryError::ReservedKey(_)));
}

#[test]
fn recall_fused_reranked_propagates_reranker_failure() {
    let (_dir, svc) = service();
    seeded_chain(&svc);

    let err = svc
        .recall_fused_reranked(
            DECISION,
            3,
            None,
            FusionOptions::default(),
            &FailingReranker,
        )
        .expect_err("reranker failure must surface, not be swallowed");
    assert!(matches!(err, MemoryError::Rerank(_)));
}
