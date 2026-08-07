//! BDD integration tests for the `remember / recall / relate / forget`
//! operations of the memory service domain core.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(feature = "persistence")]

mod common;

use common::{meta, service};
use serde_json::json;
use velesdb_memory::limits::MAX_EMBEDDABLE_TEXT_BYTES;
use velesdb_memory::{ErrorCategory, Link, MemoryError, AUTO_DATE_FIELD};

// --- Nominal ---------------------------------------------------------------

#[test]
fn remember_then_recall_returns_the_fact() {
    let (_dir, svc) = service();
    let fact = "we chose parking_lot to avoid lock poisoning";
    let id = svc.remember(fact, &[], None).expect("remember");

    let hits = svc
        .recall("parking_lot lock poisoning", 5, None)
        .expect("recall");

    assert!(
        hits.iter().any(|h| h.id == id),
        "recalled set should contain the stored fact"
    );
}

#[test]
fn recall_round_trips_caller_metadata_for_dated_context() {
    let (_dir, svc) = service();
    let m = meta(&[("occurred_at", json!("2026-01-05"))]);
    let id = svc
        .remember(
            "we chose parking_lot to avoid lock poisoning",
            &[],
            Some(&m),
        )
        .expect("remember with metadata");

    let hits = svc
        .recall("parking_lot lock poisoning", 5, None)
        .expect("recall");
    let hit = hits.iter().find(|h| h.id == id).expect("fact present");

    assert_eq!(
        hit.metadata.as_ref().and_then(|m| m.get("occurred_at")),
        Some(&json!("2026-01-05")),
        "recall() must round-trip caller metadata, not just recall_where()"
    );
}

#[test]
fn recall_metadata_holds_only_the_auto_date_when_the_fact_carries_no_caller_metadata() {
    // Since `remember` now auto-stamps every fact with `AUTO_DATE_FIELD`
    // (see `tests/auto_date_bdd.rs`), a fact given no caller metadata no
    // longer round-trips as `metadata: None` — it round-trips as
    // `Some({AUTO_DATE_FIELD: <today>})` and nothing else.
    let (_dir, svc) = service();
    svc.remember("a fact with no metadata", &[], None)
        .expect("remember");

    let hits = svc
        .recall("a fact with no metadata", 5, None)
        .expect("recall");

    let metadata = hits[0].metadata.as_ref().expect("auto date stamped");
    assert_eq!(
        metadata.keys().collect::<Vec<_>>(),
        vec![AUTO_DATE_FIELD],
        "an unmetadata'd fact must carry the auto date and nothing else"
    );
}

#[test]
fn remember_is_idempotent_on_identical_content() {
    let (_dir, svc) = service();
    let fact = "VelesDB ships as a single binary";
    let first = svc.remember(fact, &[], None).expect("first remember");
    let second = svc.remember(fact, &[], None).expect("second remember");

    assert_eq!(
        first, second,
        "identical content must yield the same stable id"
    );
}

#[test]
fn remember_with_links_creates_edges_without_error() {
    let (_dir, svc) = service();
    let pr = svc
        .remember("PR #42 introduces parking_lot", &[], None)
        .expect("remember target");
    let decision = svc
        .remember(
            "we chose parking_lot to avoid lock poisoning",
            &[Link {
                target: pr,
                relation: "decided_in".to_owned(),
            }],
            None,
        )
        .expect("remember with link");

    assert_ne!(decision, pr, "distinct facts must have distinct ids");
}

// --- Edge ------------------------------------------------------------------

#[test]
fn relate_two_existing_memories_returns_edge_id() {
    let (_dir, svc) = service();
    let a = svc
        .remember("ticket EPIC-317 tracks the lock rework", &[], None)
        .expect("remember a");
    let b = svc
        .remember("benchmark shows no contention regression", &[], None)
        .expect("remember b");

    let edge = svc.relate(a, b, "references").expect("relate");

    // Edge id is an opaque handle; we only assert the call succeeded and is usable.
    let _ = edge;
}

#[test]
fn relate_with_unknown_endpoint_errors_without_creating_a_dangling_edge() {
    let (_dir, svc) = service();
    let real = svc
        .remember("ticket EPIC-317 tracks the lock rework", &[], None)
        .expect("remember real");

    // Either endpoint missing must be rejected as client input, not silently
    // create an edge dangling off a memory that was never stored.
    let bad_target = svc
        .relate(real, 999, "references")
        .expect_err("relate to a missing target must error");
    assert!(matches!(bad_target, MemoryError::UnknownMemory(999)));

    let bad_source = svc
        .relate(999, real, "references")
        .expect_err("relate from a missing source must error");
    assert!(matches!(bad_source, MemoryError::UnknownMemory(999)));
}

#[test]
fn forget_removes_the_fact_from_recall() {
    let (_dir, svc) = service();
    let id = svc
        .remember("ephemeral scratch note about France", &[], None)
        .expect("remember");
    let found = svc.forget(id).expect("forget");
    assert!(found, "forget of an existing fact must report found=true");

    let hits = svc.recall("France", 5, None).expect("recall after forget");

    assert!(
        !hits.iter().any(|h| h.id == id),
        "forgotten fact must not be recalled"
    );
}

#[test]
fn forget_unknown_id_reports_not_found_instead_of_silent_success() {
    let (_dir, svc) = service();

    let found = svc
        .forget(999_999)
        .expect("forget on an unknown id must not error");

    assert!(
        !found,
        "forget of an id that was never stored must report found=false, \
         so a caller can distinguish a real deletion from a typo"
    );
}

// --- Negative --------------------------------------------------------------

#[test]
fn recall_on_empty_store_is_empty() {
    let (_dir, svc) = service();

    let hits = svc
        .recall("anything at all", 10, None)
        .expect("recall on empty store");

    assert!(hits.is_empty(), "fresh store has nothing to recall");
}

#[test]
fn remember_rejects_empty_fact() {
    let (_dir, svc) = service();

    let err = svc
        .remember("   ", &[], None)
        .expect_err("empty fact must be rejected");

    assert!(matches!(err, MemoryError::EmptyFact));
}

#[test]
fn remember_with_unknown_link_target_errors_and_stores_nothing() {
    let (_dir, svc) = service();

    let err = svc
        .remember(
            "a decision",
            &[Link {
                target: 999,
                relation: "x".to_owned(),
            }],
            None,
        )
        .expect_err("unknown link target must error");
    assert!(matches!(err, MemoryError::UnknownMemory(999)));

    // The fact must not have been persisted (no partial write).
    let hits = svc.recall("a decision", 5, None).expect("recall");
    assert!(
        hits.is_empty(),
        "a failed link must not leave the fact half-written"
    );
}

/// A relation label just over the 512-byte cap, shared by the
/// link-failure atomicity tests below.
fn oversized_label() -> String {
    "x".repeat(600)
}

#[test]
fn remember_with_invalid_relation_label_errors_and_stores_nothing() {
    // Regression: relation labels used to be validated only AFTER the fact
    // was stored (inside the edge-creation loop), so a bad label left the
    // fact persisted with no links — the exact half-written state the API
    // docs rule out. Labels are now validated before any write.
    let (_dir, svc) = service();
    let target = svc.remember("a target fact", &[], None).expect("remember");

    let err = svc
        .remember(
            "a decision",
            &[Link {
                target,
                relation: oversized_label(),
            }],
            None,
        )
        .expect_err("oversized relation label must error");
    assert!(matches!(err, MemoryError::InvalidRelation(_)));

    let hits = svc.recall("a decision", 5, None).expect("recall");
    assert!(
        hits.iter().all(|h| h.content != "a decision"),
        "a failed link must roll the fresh fact back, not leave it half-written"
    );
}

#[test]
fn remember_link_failure_keeps_a_fact_that_already_existed() {
    // The rollback must only remove a FRESH fact: re-remembering an existing
    // fact with a bad link errors, but deleting the fact would destroy the
    // state an earlier, successful call legitimately established.
    let (_dir, svc) = service();
    let target = svc.remember("a target fact", &[], None).expect("remember");
    svc.remember("a decision", &[], None)
        .expect("first remember succeeds");

    svc.remember(
        "a decision",
        &[Link {
            target,
            relation: oversized_label(),
        }],
        None,
    )
    .expect_err("oversized relation label must error");

    let hits = svc.recall("a decision", 5, None).expect("recall");
    assert!(
        hits.iter().any(|h| h.content == "a decision"),
        "the pre-existing fact must survive the failed re-remember"
    );
}

#[test]
fn remember_link_failure_leaves_a_pre_existing_facts_payload_untouched() {
    // Regression: relation labels used to be validated only inside the
    // edge-creation loop, AFTER store_fact — so a failed re-remember had
    // already overwritten the fact's metadata. All link input is now
    // validated before any write. (Asserted here via metadata survival;
    // the same pre-validation also prevents the failed call's TTL from
    // being armed, which recall can't observe directly — `_veles_*` keys
    // are stripped from caller-facing metadata.)
    let (_dir, svc) = service();
    let target = svc.remember("a target fact", &[], None).expect("remember");
    svc.remember(
        "a decision",
        &[],
        Some(&meta(&[("source", json!("trusted"))])),
    )
    .expect("first remember");

    svc.remember_with_ttl(
        "a decision",
        &[Link {
            target,
            relation: oversized_label(),
        }],
        Some(&meta(&[("source", json!("overwritten"))])),
        Some(60),
    )
    .expect_err("oversized relation label must error");

    let hits = svc.recall("a decision", 5, None).expect("recall");
    let hit = hits
        .iter()
        .find(|h| h.content == "a decision")
        .expect("fact still present");
    assert_eq!(
        hit.metadata.as_ref().and_then(|m| m.get("source")),
        Some(&json!("trusted")),
        "a failed call must not overwrite the pre-existing fact's metadata"
    );
}

#[test]
fn recall_with_empty_query_returns_empty() {
    let (_dir, svc) = service();
    svc.remember("some stored fact", &[], None)
        .expect("remember");

    let hits = svc.recall("   ", 5, None).expect("recall with blank query");

    assert!(hits.is_empty(), "a blank query recalls nothing");
}

#[test]
fn remember_refuses_a_fact_past_the_embeddable_cap_and_names_the_limit() {
    // Regression (#1654-2): an over-long fact used to reach the embedder and
    // come back as `embedding error: embedding backend error: ollama
    // embeddings call failed` — a raw backend fault naming neither a limit nor
    // the size that broke it. The cap must be enforced BEFORE the embedder, so
    // the message is actionable whatever the backend is.
    let (_dir, svc) = service();
    let too_long = "c".repeat(MAX_EMBEDDABLE_TEXT_BYTES + 1);

    let err = svc
        .remember(&too_long, &[], None)
        .expect_err("a fact past the embeddable cap must be refused");

    assert!(
        matches!(err, MemoryError::FactTooLarge { bytes, max }
            if bytes == MAX_EMBEDDABLE_TEXT_BYTES + 1 && max == MAX_EMBEDDABLE_TEXT_BYTES),
        "expected FactTooLarge carrying both sizes, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains(&MAX_EMBEDDABLE_TEXT_BYTES.to_string())
            && message.contains(&(MAX_EMBEDDABLE_TEXT_BYTES + 1).to_string()),
        "the message must name the limit AND the received size, got {message}"
    );
    assert_eq!(err.category(), ErrorCategory::InvalidInput);

    // Nothing was stored: the refusal happens before any write.
    let hits = svc.recall(&too_long, 5, None).expect("recall");
    assert!(hits.is_empty(), "a refused fact must not be persisted");
}

#[test]
fn remember_accepts_a_fact_exactly_at_the_embeddable_cap() {
    let (_dir, svc) = service();
    let at_cap = "c".repeat(MAX_EMBEDDABLE_TEXT_BYTES);

    svc.remember(&at_cap, &[], None)
        .expect("a fact exactly at the cap must still be accepted");
}

#[test]
fn relate_refuses_a_self_loop() {
    // Regression (#1654-5): `relate(X, X, "soi")` used to create an edge. A
    // self-loop states nothing and `why` traverses it like any other edge, so
    // it only adds noise to the evidence trail.
    let (_dir, svc) = service();
    let id = svc
        .remember("a fact that points at itself", &[], None)
        .expect("remember");

    let err = svc
        .relate(id, id, "soi")
        .expect_err("a self-loop must be refused");

    assert!(
        matches!(err, MemoryError::SelfRelation(endpoint) if endpoint == id),
        "expected SelfRelation naming the endpoint, got {err:?}"
    );
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
    let message = err.to_string();
    assert!(
        message.contains("two different ids"),
        "the message must say what to do instead, got {message}"
    );
}

#[test]
fn remember_trims_whitespace_for_idempotence() {
    let (_dir, svc) = service();

    let id_bare = svc
        .remember("hello world", &[], None)
        .expect("remember bare");
    let id_padded = svc
        .remember("  hello world  ", &[], None)
        .expect("remember padded");

    assert_eq!(
        id_bare, id_padded,
        "leading/trailing whitespace must not change the stable id"
    );
}
