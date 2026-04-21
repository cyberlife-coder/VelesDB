//! TDD tests for BM25 snapshot + WAL persistence (issue #389).
//!
//! These tests define the behavioural contract for the hybrid
//! snapshot/WAL persistence path:
//!
//! 1. Full state round-trips bitwise through `save_snapshot` /
//!    `load_snapshot` (no silent recall loss).
//! 2. WAL append + replay preserves incremental mutations on crash.
//! 3. Corrupted snapshot surfaces an error — never a silent fallback.
//! 4. Absent snapshot falls back to the payload-rebuild path without
//!    raising an error (backward-compat with pre-persistence DBs).
//! 5. `wal_truncate` after snapshot guarantees zero replay on next open.
//! 6. Concurrent adds serialise correctly through the WAL.
//!
//! All tests run single-threaded by repo convention (`--test-threads=1`).

#![allow(clippy::unwrap_used)] // tests: .unwrap()/.expect() is idiomatic

use std::sync::{Arc, Barrier};
use std::thread;

use tempfile::tempdir;

use super::bm25::Bm25Index;
use super::bm25_persistence::{load_snapshot, save_snapshot, snapshot_path};
use super::bm25_persistence_wal::{
    wal_append_add_document, wal_append_remove_document, wal_path_for_bm25, wal_replay,
    wal_truncate,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Fixed corpus used by several tests — all documents share enough
/// overlap to exercise the inverted index.
fn sample_corpus() -> Vec<(u64, &'static str)> {
    vec![
        (1, "rust programming language systems"),
        (2, "python programming data science"),
        (3, "java programming enterprise"),
        (4, "rust memory safety concurrency"),
        (5, "typescript programming web frontend"),
    ]
}

/// Convenience: sorts a query result by `doc_id` so that comparisons
/// are deterministic under stable-sort.
fn sort_by_id(mut results: Vec<(u64, f32)>) -> Vec<(u64, f32)> {
    results.sort_by_key(|(id, _)| *id);
    results
}

/// Bitwise float equality via `to_bits` — BM25 scores must round-trip
/// exactly through serialization, not just within an epsilon.
fn assert_bitwise_eq(a: &[(u64, f32)], b: &[(u64, f32)]) {
    assert_eq!(a.len(), b.len(), "result lengths differ: {a:?} vs {b:?}");
    for (lhs, rhs) in a.iter().zip(b.iter()) {
        assert_eq!(lhs.0, rhs.0, "ids differ: {a:?} vs {b:?}");
        assert_eq!(
            lhs.1.to_bits(),
            rhs.1.to_bits(),
            "score bits differ for id {}: {:?} vs {:?}",
            lhs.0,
            lhs.1,
            rhs.1
        );
    }
}

// ---------------------------------------------------------------------------
// Test 1 — snapshot round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_save_load_snapshot_roundtrip() {
    let dir = tempdir().unwrap();
    let index = Bm25Index::new();
    for (id, text) in sample_corpus() {
        index.add_document(id, text);
    }

    save_snapshot(dir.path(), &index).expect("save_snapshot");

    let loaded = load_snapshot(dir.path())
        .expect("load_snapshot")
        .expect("snapshot file should exist");

    assert_eq!(loaded.len(), index.len(), "doc_count should match");
    for query in ["rust", "programming", "python", "web"] {
        let original = sort_by_id(index.search(query, 10));
        let round_trip = sort_by_id(loaded.search(query, 10));
        assert_bitwise_eq(&original, &round_trip);
    }
}

// ---------------------------------------------------------------------------
// Test 2 — WAL append `add_document` + replay
// ---------------------------------------------------------------------------

#[test]
fn test_wal_append_add_document_replay() {
    let dir = tempdir().unwrap();
    let wal_path = wal_path_for_bm25(dir.path());

    // Reference index — in-memory truth we replay against.
    let reference = Bm25Index::new();
    for i in 0u64..10 {
        let text = format!("document number {i} rust persistence");
        wal_append_add_document(&wal_path, i, &text).unwrap();
        reference.add_document(i, &text);
    }

    let replayed = Bm25Index::new();
    let count = wal_replay(&wal_path, &replayed).expect("wal_replay");
    assert_eq!(count, 10, "replay should apply all 10 entries");
    assert_eq!(replayed.len(), reference.len());

    for query in ["rust", "persistence", "number"] {
        let expected = sort_by_id(reference.search(query, 10));
        let actual = sort_by_id(replayed.search(query, 10));
        assert_bitwise_eq(&expected, &actual);
    }
}

// ---------------------------------------------------------------------------
// Test 3 — WAL append `remove_document` + replay
// ---------------------------------------------------------------------------

#[test]
fn test_wal_append_remove_document_replay() {
    let dir = tempdir().unwrap();
    let wal_path = wal_path_for_bm25(dir.path());

    let reference = Bm25Index::new();
    for (id, text) in sample_corpus() {
        wal_append_add_document(&wal_path, id, text).unwrap();
        reference.add_document(id, text);
    }
    // Now remove ids 2 and 4 via WAL.
    wal_append_remove_document(&wal_path, 2).unwrap();
    wal_append_remove_document(&wal_path, 4).unwrap();
    reference.remove_document(2);
    reference.remove_document(4);

    let replayed = Bm25Index::new();
    let count = wal_replay(&wal_path, &replayed).expect("wal_replay");
    let expected_count = u64::try_from(sample_corpus().len() + 2).expect("sample corpus is small");
    assert_eq!(count, expected_count, "adds + removes must be counted");
    assert_eq!(replayed.len(), reference.len());

    let expected = sort_by_id(reference.search("programming", 10));
    let actual = sort_by_id(replayed.search("programming", 10));
    assert_bitwise_eq(&expected, &actual);
}

// ---------------------------------------------------------------------------
// Test 4 — snapshot + WAL replay preserves top-k queries
// ---------------------------------------------------------------------------

#[test]
fn test_snapshot_plus_wal_replay_preserves_query_topk() {
    let dir = tempdir().unwrap();
    let wal_path = wal_path_for_bm25(dir.path());

    // Phase A: build an index, snapshot it.
    let initial = Bm25Index::new();
    for (id, text) in sample_corpus() {
        initial.add_document(id, text);
    }
    save_snapshot(dir.path(), &initial).unwrap();
    // After snapshot, WAL should be truncated by the caller — here we
    // simulate that ordering explicitly.
    wal_truncate(&wal_path).unwrap();

    // Phase B: apply additional mutations directly and via WAL.
    let mutations: &[(u64, &str)] = &[
        (10, "rust async runtime tokio"),
        (11, "data science machine learning python"),
        (12, "web assembly rust performance"),
    ];
    for (id, text) in mutations {
        wal_append_add_document(&wal_path, *id, text).unwrap();
        initial.add_document(*id, text);
    }
    wal_append_remove_document(&wal_path, 3).unwrap();
    initial.remove_document(3);

    // Phase C: reload from snapshot + replay WAL.
    let mut reloaded = load_snapshot(dir.path())
        .unwrap()
        .expect("snapshot should exist");
    let replayed_count = wal_replay(&wal_path, &reloaded).unwrap();
    let expected_replay = u64::try_from(mutations.len() + 1).expect("mutations is small");
    assert_eq!(replayed_count, expected_replay);
    assert_eq!(reloaded.len(), initial.len());

    for query in ["rust", "python", "web", "programming"] {
        let expected = sort_by_id(initial.search(query, 10));
        let actual = sort_by_id(reloaded.search(query, 10));
        assert_bitwise_eq(&expected, &actual);
    }

    // Silence unused_mut on platforms where `reloaded` is only read
    // after `wal_replay` — the mutability is required by the API.
    let _ = &mut reloaded;
}

// ---------------------------------------------------------------------------
// Test 5 — absent snapshot returns Ok(None) (backward-compat)
// ---------------------------------------------------------------------------

#[test]
fn test_no_snapshot_falls_back_to_payload_rebuild() {
    let dir = tempdir().unwrap();
    // No files at all in `dir`.
    let result = load_snapshot(dir.path()).expect("load_snapshot should not error on missing file");
    assert!(
        result.is_none(),
        "absent snapshot must signal the caller to run the legacy rebuild path"
    );

    // WAL-only also loads cleanly (zero entries).
    let wal_path = wal_path_for_bm25(dir.path());
    let idx = Bm25Index::new();
    let count = wal_replay(&wal_path, &idx).expect("wal_replay");
    assert_eq!(count, 0);
    assert!(idx.is_empty());
}

// ---------------------------------------------------------------------------
// Test 6 — corrupt snapshot surfaces Err (no silent data loss)
// ---------------------------------------------------------------------------

#[test]
fn test_corrupted_snapshot_surfaces_error_without_silent_data_loss() {
    let dir = tempdir().unwrap();
    let path = snapshot_path(dir.path());

    // Write garbage bytes — no legitimate postcard payload begins with
    // these high-value byte sequences.
    std::fs::write(&path, b"\xFF\x00\xAA\xBB\xCC\xDE\xAD\xBE\xEF\x42\x17").unwrap();

    let result = load_snapshot(dir.path());
    match result {
        Err(err) => {
            let msg = err.to_string().to_lowercase();
            assert!(
                msg.contains("bm25") || msg.contains("snapshot"),
                "error message should identify BM25 or snapshot context: {msg}"
            );
        }
        Ok(_) => {
            panic!("corrupt snapshot MUST surface as Err, never Ok(None) (issue #618 learning)")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 7 — snapshot truncates WAL so next open replays zero entries
// ---------------------------------------------------------------------------

#[test]
fn test_wal_append_then_save_snapshot_truncates_wal() {
    let dir = tempdir().unwrap();
    let wal_path = wal_path_for_bm25(dir.path());

    let index = Bm25Index::new();
    for (id, text) in sample_corpus() {
        wal_append_add_document(&wal_path, id, text).unwrap();
        index.add_document(id, text);
    }
    assert!(wal_path.exists(), "WAL file should exist after appends");
    let wal_len_before = std::fs::metadata(&wal_path).unwrap().len();
    assert!(
        wal_len_before > 0,
        "WAL should be non-empty before snapshot"
    );

    save_snapshot(dir.path(), &index).unwrap();
    wal_truncate(&wal_path).unwrap();

    let wal_len_after = std::fs::metadata(&wal_path).unwrap().len();
    assert_eq!(
        wal_len_after, 0,
        "wal_truncate must reduce WAL to zero bytes"
    );

    // Next open: reload + replay → zero replayed entries, state intact.
    let reloaded = load_snapshot(dir.path()).unwrap().expect("snapshot exists");
    let replayed = wal_replay(&wal_path, &reloaded).unwrap();
    assert_eq!(replayed, 0, "truncated WAL must replay zero entries");
    assert_eq!(reloaded.len(), index.len());
}

// ---------------------------------------------------------------------------
// Test 8 — concurrent WAL append preserves every mutation
// ---------------------------------------------------------------------------

const CONCURRENT_THREADS: u64 = 4;
const CONCURRENT_PER_THREAD: u64 = 25;

#[test]
fn test_concurrent_add_document_safe_with_wal_append() {
    let dir = tempdir().unwrap();
    let wal_path = Arc::new(wal_path_for_bm25(dir.path()));

    let thread_count =
        usize::try_from(CONCURRENT_THREADS).expect("CONCURRENT_THREADS fits in usize");
    let barrier = Arc::new(Barrier::new(thread_count));

    let mut handles = Vec::with_capacity(thread_count);
    for t in 0..CONCURRENT_THREADS {
        let wal = Arc::clone(&wal_path);
        let bar = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            bar.wait();
            for i in 0..CONCURRENT_PER_THREAD {
                let id = t * CONCURRENT_PER_THREAD + i;
                // Unique-per-doc marker ensures we can spot-check
                // every id after replay without relying on top-k
                // ranking across a shared vocabulary. The tokenizer
                // splits on non-alphanumeric and drops single-char
                // tokens, so we embed the id directly in a single
                // alphanumeric token (e.g. `markerXX123`).
                let marker = format!("marker{id:05}");
                let text = format!("concurrent thread{t} doc{i} {marker}");
                wal_append_add_document(&wal, id, &text).expect("wal append");
            }
        }));
    }
    for h in handles {
        h.join().expect("thread join");
    }

    let replayed_index = Bm25Index::new();
    let count = wal_replay(&wal_path, &replayed_index).expect("wal_replay");
    let expected = CONCURRENT_THREADS * CONCURRENT_PER_THREAD;
    assert_eq!(
        count, expected,
        "all {expected} concurrent appends must survive replay"
    );
    let expected_usize = usize::try_from(expected).expect("expected fits in usize");
    assert_eq!(replayed_index.len(), expected_usize);

    // Every id must be present — each doc has a unique `markerXXXXX`
    // token (0-padded), so search for that token returns exactly one
    // hit.
    for t in 0..CONCURRENT_THREADS {
        for i in 0..CONCURRENT_PER_THREAD {
            let id = t * CONCURRENT_PER_THREAD + i;
            let needle = format!("marker{id:05}");
            let hits = replayed_index.search(&needle, 2);
            assert!(
                hits.iter().any(|(hid, _)| *hid == id),
                "id {id} missing after concurrent replay (hits: {hits:?})"
            );
        }
    }
}
