//! Native (`cargo test -p velesdb-wasm`) tests for `WasmMemoryService`.
//!
//! `wasm-bindgen`'s `JsValue` cannot be touched off `wasm32` at all —
//! constructing even `JsValue::UNDEFINED`, or `JsValue::from_str` on an
//! error path, aborts the process natively ("cannot call wasm-bindgen
//! imported functions on non-wasm targets"; it's an opaque handle into a
//! JS-engine-side table with no native fallback). This crate's own
//! `wasm_error_tests.rs` establishes the pattern for this: test the
//! *pre-JsValue* data (there, `WasmError`; here, `velesdb_memory::MemoryError`
//! via `svc.inner`, the private, pure-Rust `MemoryService` field) and never
//! call the JsValue-producing conversion natively.
//!
//! What *is* provably safe here: `relate`/`forget`'s success paths — a
//! `Result::Ok` never invokes `.map_err`'s JsValue-constructing closure — and
//! anything reached purely through `svc.inner`. Every method that takes a
//! `JsValue` parameter, or whose *error* path is exercised (malformed id,
//! unknown target via the wasm boundary), is untestable here; full coverage
//! lives in the `wasm-bindgen-test` suite (a real JS host via
//! `wasm-pack test --node`).

use super::*;

#[test]
fn test_inner_remember_with_oversized_metadata_errors() {
    // metadata is capped at 64 KiB serialized (DoS guard: metadata is a
    // keyed lookup facet, not a payload) — exercised through `svc.inner`,
    // never the JsValue-producing `remember` wrapper (see module docs).
    let svc = WasmMemoryService::new(4);
    let mut meta = Metadata::new();
    meta.insert("v".to_owned(), Value::String("x".repeat(65 * 1024)));
    let err = svc
        .inner
        .remember("x", &[], Some(&meta))
        .expect_err("oversized metadata must be rejected");
    assert!(
        matches!(err, MemoryError::MetadataTooLarge { .. }),
        "expected MetadataTooLarge, got {err:?}"
    );
}

#[test]
fn test_relate_and_forget_round_trip_through_the_wasm_boundary() {
    let svc = WasmMemoryService::new(4);
    let a = svc.inner.remember("fact a", &[], None).unwrap();
    let b = svc.inner.remember("fact b", &[], None).unwrap();

    let edge = svc
        .relate(&a.to_string(), &b.to_string(), "references")
        .unwrap();
    assert!(
        edge.parse::<u64>().is_ok(),
        "edge id must be a decimal string"
    );
    let explanation = svc.inner.why("fact a", 1, None).unwrap();
    assert!(
        explanation.nodes.iter().any(|n| n.id == b),
        "relate() through the wasm boundary must produce a traversable edge"
    );

    let found = svc.forget(&a.to_string()).unwrap();
    assert!(found, "forget() of an existing fact must report found=true");
    let hits = svc.inner.recall("fact a", 5, None).unwrap();
    assert!(
        hits.iter().all(|h| h.id != a),
        "forget() through the wasm boundary must actually delete the fact"
    );
}

#[test]
fn test_forget_unknown_id_is_ok_but_reports_not_found() {
    let svc = WasmMemoryService::new(4);
    // Deleting a never-stored id is a no-op, not an error (idempotent
    // delete) — but the caller must be able to tell it apart from a real
    // deletion, so it reports found=false. Stays on the Ok path and never
    // touches JsValue, so it is safe to probe natively.
    assert!(
        !svc.forget("999999999").unwrap(),
        "an id that was never stored must report found=false"
    );
}

// --- Pure-Rust coverage of the underlying orchestration (via `svc.inner`,
// bypassing the JsValue-taking wrapper methods entirely) — proves WasmStore
// correctly backs the exact same MemoryService wedge those wrappers delegate
// to, including error variants the wasm boundary can't be probed for here.

#[test]
fn test_inner_remember_then_recall_finds_the_fact() {
    let svc = WasmMemoryService::new(4);
    let id = svc
        .inner
        .remember("parking_lot avoids lock poisoning", &[], None)
        .unwrap();
    let hits = svc.inner.recall("lock poisoning", 5, None).unwrap();
    assert!(hits.iter().any(|h| h.id == id));
}

#[test]
fn test_inner_relate_to_unknown_target_errors() {
    let svc = WasmMemoryService::new(4);
    let a = svc.inner.remember("fact a", &[], None).unwrap();
    let err = svc.inner.relate(a, 999_999_999, "references").unwrap_err();
    assert!(matches!(err, MemoryError::UnknownMemory(999_999_999)));
}

#[test]
fn test_inner_recall_fused_reaches_a_graph_connected_fact() {
    let svc = WasmMemoryService::new(4);
    let decision = svc
        .inner
        .remember("we chose parking_lot to avoid lock poisoning", &[], None)
        .unwrap();
    let ticket = svc
        .inner
        .remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .unwrap();
    let distractor = svc
        .inner
        .remember("the quarterly report is due next Friday", &[], None)
        .unwrap();
    svc.inner.relate(decision, ticket, "decided_in").unwrap();

    let fused = svc
        .inner
        .recall_fused(
            "we chose parking_lot to avoid lock poisoning",
            3,
            None,
            velesdb_memory::FusionOptions::default(),
        )
        .unwrap();
    // `expect` both ranks: `Option`'s ordering makes `None < Some(_)` true,
    // so comparing raw `position()` results would pass vacuously if the
    // graph-reached fact were dropped from the results entirely.
    let rank_of = |id: u64| {
        fused
            .iter()
            .position(|r| r.id == id)
            .expect("both facts must be present in the fused results")
    };
    assert!(
        rank_of(ticket) < rank_of(distractor),
        "graph-reached fact must outrank the distractor"
    );
}

#[test]
fn test_inner_why_reaches_a_two_hop_fact_vector_search_misses() {
    let svc = WasmMemoryService::new(4);
    let decision = svc
        .inner
        .remember("we chose parking_lot to avoid lock poisoning", &[], None)
        .unwrap();
    let ticket = svc
        .inner
        .remember("EPIC-317 xyzzy quux frobnicate", &[], None)
        .unwrap();
    svc.inner.relate(decision, ticket, "decided_in").unwrap();

    let explanation = svc
        .inner
        .why("we chose parking_lot to avoid lock poisoning", 2, None)
        .unwrap();
    assert!(explanation.nodes.iter().any(|n| n.id == ticket));
}

#[test]
fn test_inner_recall_fused_dated_builds_a_chronological_timeline() {
    // The `recallFusedDated` JS boundary can't be touched natively (JsValue),
    // but its logic is the pure-Rust `inner.recall_fused_dated` the wrapper
    // delegates to — so the timeline/now behavior is provable here.
    let svc = WasmMemoryService::new(4);
    let mut newer = Metadata::new();
    newer.insert("ts".to_owned(), serde_json::json!(20_260_701));
    svc.inner
        .remember("the release shipped", &[], Some(&newer))
        .unwrap();
    let mut older = Metadata::new();
    older.insert("ts".to_owned(), serde_json::json!(20_260_103));
    svc.inner
        .remember("the project kicked off", &[], Some(&older))
        .unwrap();

    let (_hits, ctx) = svc
        .inner
        .recall_fused_dated(
            "project release timeline",
            10,
            None,
            velesdb_memory::FusionOptions::default(),
            "ts",
        )
        .unwrap();
    assert!(ctx
        .timeline
        .contains("- [2026-01-03] the project kicked off"));
    assert!(ctx.timeline.contains("- [2026-07-01] the release shipped"));
    assert!(
        ctx.timeline.find("2026-01-03").unwrap() < ctx.timeline.find("2026-07-01").unwrap(),
        "timeline is oldest-first"
    );
    assert_eq!(ctx.now.as_deref(), Some("2026-07-01"));
}
