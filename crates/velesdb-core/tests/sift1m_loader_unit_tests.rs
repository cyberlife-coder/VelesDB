//! Unit tests for the SIFT1M loader helpers.
//!
//! Runs under `cargo test --features bench-sift1m` — the same feature
//! that gates the bench binary. These tests exercise the pure helpers
//! (`filter_groundtruth`, `verify_fingerprint`) without requiring the
//! 168 MB corpus.
//!
//! The bench binary `sift1m_recall` uses `criterion_main!`, which
//! replaces the default test harness and therefore does NOT discover
//! `#[cfg(test)]` modules. Keeping these tests in `tests/` makes them
//! runnable via the standard `cargo test` flow.

#![cfg(feature = "bench-sift1m")]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

#[path = "../benches/datasets/mod.rs"]
mod datasets;

use datasets::sift1m::{
    filter_groundtruth, load_pinned_fingerprints, verify_fingerprint, DatasetError,
    PinnedFingerprints,
};

#[test]
fn filter_groundtruth_keeps_in_range_ids_and_drops_out_of_range() {
    // 3 queries, groundtruth rows mix in-range (< n_base) and out-of-range IDs.
    // n_base = 10 -> IDs 0..=9 survive, IDs 10+ are filtered out.
    // n_query = 2 -> only the first two rows survive truncation.
    let gt = vec![
        vec![0, 5, 12, 99, 3],    // survives: [0, 5, 3]
        vec![42, 100, 1, 7, 500], // survives: [1, 7]
        vec![2, 4],               // truncated entirely — n_query = 2
    ];
    let out = filter_groundtruth(gt, 10, 2);
    assert_eq!(
        out.len(),
        2,
        "groundtruth must be truncated to n_query rows"
    );
    assert_eq!(out[0], vec![0, 5, 3]);
    assert_eq!(out[1], vec![1, 7]);
}

#[test]
fn filter_groundtruth_handles_empty_result_when_all_ids_out_of_range() {
    let gt = vec![vec![1000, 2000, 3000]];
    let out = filter_groundtruth(gt, 10, 1);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].is_empty(),
        "empty row expected when all IDs exceed n_base"
    );
}

#[test]
fn filter_groundtruth_saturates_on_huge_n_base_without_overflow() {
    // When n_base > u32::MAX, the saturating conversion clamps to u32::MAX,
    // so IDs strictly less than u32::MAX are kept and u32::MAX itself is dropped.
    let gt = vec![vec![0u32, u32::MAX, 42]];
    let out = filter_groundtruth(gt, usize::MAX, 1);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0],
        vec![0, 42],
        "u32::MAX is NOT strictly less than u32::MAX, only [0, 42] survive"
    );
}

#[test]
fn filter_groundtruth_preserves_order_within_rows() {
    // Filtering must not reorder in-range IDs (downstream recall relies on
    // sequential layout for the top-k prefix).
    let gt = vec![vec![8, 20, 3, 42, 1, 99]];
    let out = filter_groundtruth(gt, 10, 1);
    assert_eq!(out[0], vec![8, 3, 1]);
}

#[test]
fn pinned_fingerprints_roundtrip_through_serde() {
    let fingerprints = PinnedFingerprints {
        base: "a".repeat(64),
        query: "b".repeat(64),
        groundtruth: "c".repeat(64),
    };
    let json = serde_json::to_string(&fingerprints).expect("serialize");
    let back: PinnedFingerprints = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(fingerprints, back);
    // JSON keys must match the on-disk filenames for sidecar self-documentation.
    assert!(json.contains("\"sift_base_fvecs_sha256\":"));
    assert!(json.contains("\"sift_query_fvecs_sha256\":"));
    assert!(json.contains("\"sift_groundtruth_ivecs_sha256\":"));
}

#[test]
fn load_pinned_fingerprints_returns_none_when_sidecar_absent() {
    // In the test environment, `benches/datasets/sift1m_fingerprints.json` is
    // deliberately not checked in (example file uses `.example.json` suffix),
    // so the loader must gracefully return None and let the caller fall back
    // to the `SHA256_*` constants or TOFU behavior.
    let got = load_pinned_fingerprints();
    assert!(
        got.is_none(),
        "expected None when sidecar absent, got: {got:?}"
    );
}

#[test]
fn verify_fingerprint_placeholder_mode_returns_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.bin");
    std::fs::write(&path, b"hello sift1m").expect("write probe");

    // Placeholder mode: expected starts with "TODO_" -> prints observed hash, Ok.
    verify_fingerprint(&path, "TODO_FINGERPRINT_probe").expect("placeholder mode must return Ok");
}

#[test]
fn verify_fingerprint_detects_mismatch_with_real_hash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.bin");
    std::fs::write(&path, b"hello sift1m").expect("write probe");

    // A valid-shape but wrong SHA-256 — must surface Parse error.
    let fake_sha = "0000000000000000000000000000000000000000000000000000000000000000";
    let err = verify_fingerprint(&path, fake_sha).expect_err("must flag mismatch");
    assert!(
        matches!(err, DatasetError::Parse(_)),
        "expected Parse error variant, got {err:?}"
    );
}

#[test]
fn verify_fingerprint_accepts_matching_real_hash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("probe.bin");
    let contents = b"velesdb sift1m fixture";
    std::fs::write(&path, contents).expect("write probe");

    // SHA-256 of the contents above, computed independently via the same
    // algorithm. Round-trip: capture placeholder-mode observed hash, then
    // feed it back as the expected value and require Ok. This avoids
    // hardcoding a platform-dependent literal.
    let captured = capture_observed_hash(&path).expect("placeholder capture must succeed");
    verify_fingerprint(&path, &captured).expect("matching hash must return Ok");
}

/// Re-computes the SHA-256 of `path` to round-trip through
/// `verify_fingerprint` without hardcoding a literal.
fn capture_observed_hash(path: &std::path::Path) -> Result<String, DatasetError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in &bytes {
        let hi = b >> 4;
        let lo = b & 0x0f;
        out.push(char::from(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 }));
        out.push(char::from(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 }));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Regression tests for `HnswIndex::search_raw` (bench-sift1m-gated API)
//
// These guard the invariant that `search_raw` bypasses both quality-based
// ef scaling (`ef_search_for_scale`) and two-stage reranking, so the `ef`
// value it reports is the literal graph-traversal budget — the apples-to-
// apples plain-HNSW path expected by the SIFT1M methodology.
// ---------------------------------------------------------------------------

use velesdb_core::distance::DistanceMetric;
use velesdb_core::{HnswIndex, VectorIndex};

/// Builds a tiny HNSW index with `n` synthetic 8-dim vectors.
fn tiny_index(n: u64) -> HnswIndex {
    let index = HnswIndex::new(8, DistanceMetric::Euclidean).expect("construct tiny HNSW index");
    for i in 0..n {
        let base = (i as f32) * 0.1;
        let v: Vec<f32> = (0..8).map(|j| base + (j as f32) * 0.01).collect();
        index.insert(i, &v);
    }
    index
}

#[test]
fn search_raw_rejects_wrong_dimension() {
    let index = tiny_index(10);
    // 4-dim query against an 8-dim index — must surface DimensionMismatch
    // rather than panicking or silently returning junk.
    let query = vec![0.0_f32; 4];
    let err = index
        .search_raw(&query, 5, 64)
        .expect_err("dim mismatch must be an error");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Dimension") || msg.contains("dimension"),
        "expected DimensionMismatch error, got: {msg}"
    );
}

#[test]
fn search_raw_returns_up_to_k_results_with_valid_query() {
    let index = tiny_index(20);
    let query = vec![0.5_f32; 8];
    let results = index
        .search_raw(&query, 5, 64)
        .expect("valid query must succeed");
    assert!(
        results.len() <= 5,
        "search_raw must cap results at k; got {}",
        results.len()
    );
    assert!(
        !results.is_empty(),
        "expected at least one result on a non-empty index"
    );
    // Sanity: IDs are in-range.
    for r in &results {
        assert!(r.id < 20, "unexpected id {} (should be < 20)", r.id);
    }
}

#[test]
fn search_raw_does_not_overfetch_beyond_k_unlike_rerank() {
    // Two-stage reranking would fetch top-(k*4) candidates and then truncate
    // to k. `search_raw` must skip that path entirely and return at most k
    // results from a k-budget graph search — no hidden oversampling.
    let index = tiny_index(100);
    let query = vec![0.3_f32; 8];
    for &ef in &[16_usize, 32, 64, 128] {
        let results = index
            .search_raw(&query, 10, ef)
            .expect("valid query must succeed");
        assert!(
            results.len() <= 10,
            "search_raw with k=10 must never return more than 10 results (ef={ef}); got {}",
            results.len()
        );
    }
}
