//! High-cardinality qualification for issue #1805.
//!
//! The original report claimed a hard transition from a correct exact match
//! at 1,192 memories to no result at 1,193. The report's temporary probe was
//! not retained, so this test keeps the boundary executable: same
//! `HashEmbedder`, native store, one hub-shaped fan-out, plain recall, exact
//! metadata-filtered recall, and `why` traversal on both sides of the alleged
//! cliff. It characterizes current `develop`; it does not explain the
//! historical signal that could not be reproduced.
//!
//! It performs more than two thousand durable point/edge writes and is kept
//! out of the ordinary unit-test budget. Run it explicitly with:
//!
//! ```text
//! cargo test -p velesdb-memory --test recall_threshold_bdd \
//!   --no-default-features --features persistence -- --ignored --exact \
//!   recall_and_why_stay_live_across_1193_memories
//! ```

#![cfg(feature = "persistence")]

mod common;

use common::{meta, service};
use serde_json::json;
use velesdb_memory::{FusionOptions, HashEmbedder, MemoryService};

const SEED: &str = "seed fact anchors the walk";
const REPORTED_FAILURE_TOTAL: usize = 1_193;

fn assert_recall_is_live(svc: &MemoryService<HashEmbedder>, seed_id: u64, expected_total: usize) {
    let unfiltered = svc.recall(SEED, 1, None).expect("plain recall");
    assert_eq!(unfiltered.len(), 1, "plain recall became empty");
    assert_eq!(
        unfiltered[0].id, seed_id,
        "plain recall lost the exact seed"
    );
    assert!(
        unfiltered[0].score > 0.999,
        "exact self-match score degraded: {}",
        unfiltered[0].score
    );

    let seed_filter = meta(&[("probe", json!("seed"))]);
    let filtered = svc
        .recall(SEED, 1, Some(&seed_filter))
        .expect("filtered recall");
    assert_eq!(filtered.len(), 1, "filtered recall became empty");
    assert_eq!(
        filtered[0].id, seed_id,
        "filtered recall lost the exact seed"
    );

    let explanation = svc.why(SEED, 2, None).expect("why");
    assert_eq!(
        explanation.nodes.len(),
        expected_total,
        "why did not traverse the complete hub fan-out"
    );
}

#[test]
#[ignore = "manual high-cardinality persistence qualification for issue #1805"]
fn recall_and_why_stay_live_across_1193_memories() {
    let (_dir, svc) = service();
    let seed_filter = meta(&[("probe", json!("seed"))]);
    let seed_id = svc
        .remember(SEED, &[], Some(&seed_filter))
        .expect("remember seed");
    let hub_id = svc
        .remember("Entity: threshold-probe", &[], None)
        .expect("remember hub");
    svc.relate(seed_id, hub_id, "about")
        .expect("relate seed to hub");

    for index in 0..(REPORTED_FAILURE_TOTAL - 2) {
        let fact_id = svc
            .remember(
                &format!("fact number {index} about the threshold probe"),
                &[],
                None,
            )
            .expect("remember fan-out fact");
        svc.relate(hub_id, fact_id, "mentions")
            .expect("relate hub to fact");

        let total = index + 3;
        if matches!(total, 1_192 | REPORTED_FAILURE_TOTAL) {
            assert_recall_is_live(&svc, seed_id, total);
        }
    }

    let fused = svc
        .recall_fused(SEED, 10, None, FusionOptions::default())
        .expect("fused recall");
    assert!(
        fused.iter().any(|memory| memory.id == seed_id),
        "fused recall lost the exact seed at the reported boundary"
    );
}
