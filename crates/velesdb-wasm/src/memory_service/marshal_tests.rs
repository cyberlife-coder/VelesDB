//! Native tests of the count arguments' conversion: everything here is
//! pre-`JsValue` (`MemoryError`), so it runs off `wasm32` (see
//! `memory_service_tests.rs`'s module docs for why a `JsValue` cannot).

use super::{fragment_index, recall_limit, reserve_tokens, to_fusion_input, ttl_seconds, why_hops};
use velesdb_memory::limits::{DEFAULT_WHY_HOPS, MAX_RECALL_LIMIT, MAX_WHY_HOPS};
use velesdb_memory::{FusionOptions, MemoryError};

/// Each value `wasm-bindgen` used to coerce silently into `usize`/`u64`:
/// `Infinity`/`NaN` became 0 (an empty recall), `-1` 4294967295, `2.9` 2, and
/// a `-1n` TTL `u64::MAX` — "never expires".
const WRAPPED: [f64; 6] = [f64::INFINITY, f64::NAN, -1.0, 2.9, 4_294_967_296.0, -0.5];

fn refused_as<T: std::fmt::Debug>(result: Result<T, MemoryError>, name: &str) {
    match result {
        Err(MemoryError::InvalidCount { name: got, .. }) => assert_eq!(got, name),
        other => panic!("{name}: expected InvalidCount, got {other:?}"),
    }
}

#[test]
fn every_count_argument_refuses_what_the_host_would_wrap() {
    for bad in WRAPPED {
        refused_as(recall_limit(Some(bad)), "k");
        refused_as(why_hops(Some(bad)), "maxHops");
        refused_as(ttl_seconds(Some(bad)), "ttlSeconds");
        refused_as(fragment_index(Some(bad)), "fragmentIndex");
        refused_as(reserve_tokens(Some(bad)), "reserveTokens");
        refused_as(to_fusion_input(Some(bad), None, None), "hops");
        refused_as(to_fusion_input(None, None, Some(bad)), "pool");
    }
}

#[test]
fn a_valid_count_keeps_its_default_and_cap() {
    assert_eq!(recall_limit(None).ok(), Some(10));
    assert_eq!(recall_limit(Some(3.0)).ok(), Some(3));
    assert_eq!(recall_limit(Some(1e6)).ok(), Some(MAX_RECALL_LIMIT));
    assert_eq!(why_hops(None).ok(), Some(DEFAULT_WHY_HOPS));
    assert_eq!(why_hops(Some(50.0)).ok(), Some(MAX_WHY_HOPS));
    assert_eq!(ttl_seconds(None).ok(), Some(None));
    assert_eq!(ttl_seconds(Some(60.0)).ok(), Some(Some(60)));
    assert_eq!(fragment_index(Some(0.0)).ok(), Some(Some(0)));
    assert_eq!(reserve_tokens(None).ok(), Some(0));
    assert_eq!(reserve_tokens(Some(512.0)).ok(), Some(512));

    // Built by `FusionOptions::from_knobs`, as Node, Python and MCP build
    // theirs: `pool` is clamped to the recall cap like `hops` to its own.
    let opts = to_fusion_input(Some(3.0), Some(0.25), Some(1e6));
    assert_eq!(
        opts.map(|o| (o.hops, o.graph_boost, o.pool)).ok(),
        Some((3, 0.25, Some(MAX_RECALL_LIMIT)))
    );
    let defaults = FusionOptions::default();
    let opts = to_fusion_input(None, None, None);
    assert_eq!(
        opts.map(|o| (o.hops, o.pool)).ok(),
        Some((defaults.hops, defaults.pool))
    );
}
