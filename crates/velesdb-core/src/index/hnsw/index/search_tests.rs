//! Tests for the adaptive-search escalation rule.

use super::should_escalate;
use crate::distance::DistanceMetric;

/// A cosine tail that merely lands near zero is not a hard query.
///
/// Under a baseline of `min(|first|, |last|)` this reads as a spread of ~91
/// and escalates; measured from cosine's own floor of `-1.0` it is ~0.92 and
/// correctly does not. Every `Adaptive`/`AutoTune` query whose result tail
/// crosses zero would otherwise pay a second traversal.
#[test]
fn a_cosine_tail_near_zero_is_not_a_hard_query() {
    assert!(
        !should_escalate(DistanceMetric::Cosine, 0.9, -0.01),
        "a 0.9 -> -0.01 spread is ordinary for cosine and must not escalate"
    );
    // What the zero-based baseline would have computed, for contrast.
    let naive = (0.9_f32 - -0.01).abs() / 0.9_f32.abs().min(0.01);
    assert!(
        naive > 2.0,
        "the zero-based baseline really would have escalated here ({naive})"
    );
}

/// A genuinely heterogeneous cosine neighbourhood still escalates.
#[test]
fn a_wide_cosine_neighbourhood_escalates() {
    assert!(should_escalate(DistanceMetric::Cosine, 0.95, -0.9));
}

/// A tightly clustered cosine result set does not.
#[test]
fn a_tight_cosine_neighbourhood_does_not_escalate() {
    assert!(!should_escalate(DistanceMetric::Cosine, 0.95, 0.90));
}

/// Unbounded distance metrics keep the original zero-based relative spread.
///
/// Zero *is* the floor for a distance, so nothing changes for these: the ratio
/// is the meaningful quantity because the scale is set by the data.
#[test]
fn unbounded_metrics_keep_the_zero_based_relative_spread() {
    // A 1.0 -> 100.0 Euclidean gap is a 99x ratio: hard.
    assert!(should_escalate(DistanceMetric::Euclidean, 1.0, 100.0));
    // 10.0 -> 11.0 is a 0.1 ratio: easy.
    assert!(!should_escalate(DistanceMetric::Euclidean, 10.0, 11.0));
    assert!(should_escalate(DistanceMetric::Hamming, 1.0, 64.0));
}

/// Jaccard's floor is zero, so its baseline is unchanged by the floor rule.
#[test]
fn jaccard_floor_is_zero_so_its_baseline_is_unchanged() {
    // 0.9 -> 0.1 : baseline 0.1, spread 8 — hard either way.
    assert!(should_escalate(DistanceMetric::Jaccard, 0.9, 0.1));
    // 0.9 -> 0.5 : baseline 0.5, spread 0.8 — easy either way.
    assert!(!should_escalate(DistanceMetric::Jaccard, 0.9, 0.5));
}

/// A degenerate all-equal result set must not divide by zero into an escalation.
#[test]
fn an_all_equal_result_set_does_not_escalate() {
    assert!(!should_escalate(DistanceMetric::Cosine, 0.5, 0.5));
    assert!(!should_escalate(DistanceMetric::Euclidean, 0.0, 0.0));
    // Cosine pinned at its floor: baseline 0, diff 0 — still no escalation.
    assert!(!should_escalate(DistanceMetric::Cosine, -1.0, -1.0));
}

// --- nodes an upsert or a delete leaves behind ---

const DIM: usize = 16;
const POINTS: u64 = 300;

/// A deterministic, well-spread vector for `id`.
fn spread(id: u64) -> Vec<f32> {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            f32::from(u16::try_from(state >> 48).expect("test: 16 bits")) / 65_536.0
        })
        .collect()
}

/// An index without exact-distance features, as a fast-insert collection
/// runs, which never reranks the walk: the shape the dropped results showed
/// on. Points are inserted one at a time, so the graph is the same on every
/// run.
fn indexes() -> [(&'static str, super::HnswIndex); 1] {
    let fast =
        super::HnswIndex::new_fast_insert(DIM, DistanceMetric::Euclidean).expect("test: index");
    for id in 0..POINTS {
        assert!(fast.insert_and_assign(id, &spread(id)), "premise: placed");
    }
    [("fast-insert", fast)]
}

fn first_id(index: &super::HnswIndex, query: &[f32], quality: crate::SearchQuality) -> Option<u64> {
    index
        .search_with_quality(query, 1, quality)
        .expect("test: search")
        .first()
        .map(|hit| hit.id)
}

/// An upsert leaves the id's old node in the graph, unmapped. A `k = 1`
/// search at the old vector reaches that node first; it must still return
/// the id's live node rather than nothing. 50 ids, each moved a little.
#[test]
fn a_node_an_upsert_left_behind_does_not_take_the_only_place() {
    for (shape, index) in indexes() {
        for id in 0..50 {
            let old = spread(id);
            let mut moved = old.clone();
            moved[0] += 1e-3;
            assert!(index.insert_and_assign(id, &moved), "premise: upserted");
            for quality in [crate::SearchQuality::Fast, crate::SearchQuality::Balanced] {
                assert_eq!(
                    first_id(&index, &old, quality),
                    Some(id),
                    "{shape} index, {quality:?}: id {id} after its upsert"
                );
            }
        }
    }
}

/// A delete leaves the node in the graph, unmapped. A `k = 1` search at
/// its vector must return the nearest point still there, not nothing.
#[test]
fn a_node_a_delete_left_behind_does_not_take_the_only_place() {
    for (shape, index) in indexes() {
        for id in 100..150 {
            assert!(index.remove(id), "premise: deleted");
            for quality in [crate::SearchQuality::Fast, crate::SearchQuality::Balanced] {
                let hit = first_id(&index, &spread(id), quality);
                assert!(
                    hit.is_some_and(|hit| hit != id),
                    "{shape} index, {quality:?}: deleted {id} left {hit:?}"
                );
            }
        }
    }
}
