//! Crash-simulation tests for `save_sidecars` / `load_sidecars` atomicity
//! (issue #617).
//!
//! `save_sidecars` writes three sidecar files sequentially
//! (`native_mappings.bin`, `native_vectors.bin`, `native_meta.bin`). Each
//! individual file uses atomic write-tmp-fsync-rename, but the 3-file
//! sequence itself is not atomic. A crash between two renames leaves
//! the on-disk state inconsistent, which can silently corrupt search or
//! panic at load time.
//!
//! To detect a crashed save we stamp every sidecar with a monotonic
//! `generation: u64`. `meta` (written last) is the authoritative commit
//! point. Any file with a stale generation is proof of an incomplete
//! prior save, and `load_sidecars` must reject the database with
//! `io::ErrorKind::InvalidData` rather than silently return a torn state.
//!
//! These tests simulate the crashed states by manually rewriting the
//! sidecar files with mismatched generations, no real crash is needed.
//! Backward-compat cases (legacy DBs without the generation counter)
//! must keep loading unchanged.
//!
//! NOTE: this module is an internal `#[cfg(test)]` sibling of
//! [`super::persistence`], which means every `pub(crate)` helper in
//! `persistence.rs` is reachable from here.

use super::persistence::{
    self, load_sidecars, save_sidecars, HnswMappingsData, HnswMeta, HnswVectorsData,
};
use super::sharded_mappings::ShardedMappings;
use super::sharded_vectors::ShardedVectors;
use crate::distance::DistanceMetric;
use crate::StorageMode;
use std::collections::HashMap;
use std::path::Path;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Builds a minimal `HnswMeta` for the atomicity tests.
fn build_meta(generation: u64) -> HnswMeta {
    HnswMeta {
        dimension: 4,
        metric: DistanceMetric::Cosine,
        enable_vector_storage: true,
        storage_mode: StorageMode::Full,
        generation,
    }
}

/// Builds a populated `ShardedMappings` containing two id→idx pairs.
fn build_mappings() -> ShardedMappings {
    let mut id_to_idx = HashMap::new();
    id_to_idx.insert(1_u64, 0_usize);
    id_to_idx.insert(2_u64, 1_usize);
    let mut idx_to_id = HashMap::new();
    idx_to_id.insert(0_usize, 1_u64);
    idx_to_id.insert(1_usize, 2_u64);
    ShardedMappings::from_parts(id_to_idx, idx_to_id, 2)
}

/// Builds a populated `ShardedVectors` containing two 4-d vectors.
fn build_vectors() -> ShardedVectors {
    let vectors = ShardedVectors::new(4);
    vectors.insert_batch(vec![
        (0_usize, vec![1.0_f32, 0.0, 0.0, 0.0]),
        (1_usize, vec![0.0_f32, 1.0, 0.0, 0.0]),
    ]);
    vectors
}

/// Writes the legacy 4-tuple meta format (v1.7.2+, without the generation
/// field) directly via postcard, bypassing `save_meta` so the file ends
/// up at a real pre-fix format.
fn write_legacy_4tuple_meta(path: &Path, meta: &HnswMeta) -> std::io::Result<()> {
    let metric_u8 = meta.metric as u8;
    let storage_mode_u8 = match meta.storage_mode {
        StorageMode::Full => 0u8,
        StorageMode::SQ8 => 1,
        StorageMode::Binary => 2,
        StorageMode::ProductQuantization => 3,
        StorageMode::RaBitQ => 4,
    };
    let bytes = postcard::to_allocvec(&(
        meta.dimension,
        metric_u8,
        meta.enable_vector_storage,
        storage_mode_u8,
    ))
    .map_err(std::io::Error::other)?;
    std::fs::write(path.join("native_meta.bin"), bytes)
}

/// Writes the legacy 3-tuple meta format (pre-v1.7.2, without storage mode
/// or generation) directly via postcard.
fn write_legacy_3tuple_meta(path: &Path, meta: &HnswMeta) -> std::io::Result<()> {
    let metric_u8 = meta.metric as u8;
    let bytes = postcard::to_allocvec(&(meta.dimension, metric_u8, meta.enable_vector_storage))
        .map_err(std::io::Error::other)?;
    std::fs::write(path.join("native_meta.bin"), bytes)
}

/// Writes the legacy 3-tuple mappings format (pre-generation) directly.
fn write_legacy_3tuple_mappings(path: &Path, data: &HnswMappingsData) -> std::io::Result<()> {
    let bytes = postcard::to_allocvec(&(&data.id_to_idx, &data.idx_to_id, data.next_idx))
        .map_err(std::io::Error::other)?;
    std::fs::write(path.join("native_mappings.bin"), bytes)
}

/// Writes the legacy plain-`Vec` vectors format (pre-generation) directly.
fn write_legacy_plain_vectors(path: &Path, data: &HnswVectorsData) -> std::io::Result<()> {
    let bytes = postcard::to_allocvec(&data.vectors).map_err(std::io::Error::other)?;
    std::fs::write(path.join("native_vectors.bin"), bytes)
}

/// Returns the `HnswMappingsData` in the same shape `save_sidecars` expects.
fn mappings_data(mappings: &ShardedMappings, generation: u64) -> HnswMappingsData {
    let (id_to_idx, idx_to_id, next_idx) = mappings.as_parts();
    HnswMappingsData {
        id_to_idx,
        idx_to_id,
        next_idx,
        generation,
    }
}

/// Returns the `HnswVectorsData` in the same shape `save_sidecars` expects.
fn vectors_data(vectors: &ShardedVectors, generation: u64) -> HnswVectorsData {
    HnswVectorsData {
        vectors: vectors.collect_for_parallel(),
        generation,
    }
}

// -----------------------------------------------------------------------
// Test 1 — save_sidecars stamps a monotonically increasing generation
// -----------------------------------------------------------------------

#[test]
fn test_save_sidecars_stamps_monotonic_generation() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();
    let meta = build_meta(0);

    save_sidecars(
        path,
        &mappings,
        &vectors,
        &meta,
        persistence::next_generation(path).expect("test: next_generation"),
    )
    .expect("test: first save");
    save_sidecars(
        path,
        &mappings,
        &vectors,
        &meta,
        persistence::next_generation(path).expect("test: next_generation"),
    )
    .expect("test: second save");
    save_sidecars(
        path,
        &mappings,
        &vectors,
        &meta,
        persistence::next_generation(path).expect("test: next_generation"),
    )
    .expect("test: third save");

    let loaded_meta = persistence::load_meta(path).expect("test: load meta");
    assert_eq!(
        loaded_meta.generation, 3,
        "meta generation should be bumped once per save"
    );

    let loaded_mappings = persistence::load_mappings(path).expect("test: load mappings");
    assert_eq!(
        loaded_mappings.generation, 3,
        "mappings generation must match meta"
    );

    let loaded_vectors = persistence::load_vectors(path).expect("test: load vectors");
    assert_eq!(
        loaded_vectors.generation, 3,
        "vectors generation must match meta"
    );
}

// -----------------------------------------------------------------------
// Test 2 — load_sidecars detects stale mappings
// -----------------------------------------------------------------------

#[test]
fn test_load_sidecars_detects_stale_mappings() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Simulate a save at generation 4 (all four artefacts consistent:
    // graph marker, mappings, vectors, meta).
    let meta_4 = build_meta(4);
    persistence::save_graph_generation(path, 4).expect("test: graph gen 4");
    persistence::save_mappings(path, &mappings_data(&mappings, 4)).expect("test: save mappings 4");
    persistence::save_vectors(path, &vectors_data(&vectors, 4)).expect("test: save vectors 4");
    persistence::save_meta(path, &meta_4).expect("test: save meta 4");

    // Now simulate a crash at generation 5 that only rewrote mappings.
    persistence::save_mappings(path, &mappings_data(&mappings, 5)).expect("test: save mappings 5");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    let err = load_sidecars(path, &loaded_meta)
        .expect_err("test: stale mappings must trigger InvalidData");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("mappings generation"),
        "error should mention mappings generation, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Test 3 — load_sidecars detects stale vectors
// -----------------------------------------------------------------------

#[test]
fn test_load_sidecars_detects_stale_vectors() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Consistent state at generation 4 (graph + mappings + vectors + meta).
    persistence::save_graph_generation(path, 4).expect("test: graph gen 4");
    persistence::save_mappings(path, &mappings_data(&mappings, 4)).expect("test: save mappings 4");
    persistence::save_vectors(path, &vectors_data(&vectors, 4)).expect("test: save vectors 4");
    persistence::save_meta(path, &build_meta(4)).expect("test: save meta 4");

    // Simulate a crash at generation 5 that rewrote graph + mappings + meta,
    // leaving vectors at gen 4.
    persistence::save_graph_generation(path, 5).expect("test: graph gen 5");
    persistence::save_mappings(path, &mappings_data(&mappings, 5)).expect("test: save mappings 5");
    persistence::save_meta(path, &build_meta(5)).expect("test: save meta 5");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    let err = load_sidecars(path, &loaded_meta)
        .expect_err("test: stale vectors must trigger InvalidData");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("vectors generation"),
        "error should mention vectors generation, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Test 4 — load_sidecars detects mappings newer than meta
// -----------------------------------------------------------------------

#[test]
fn test_load_sidecars_detects_newer_mappings_than_meta() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Meta is older (gen 5) than mappings (gen 10) — `meta` is authoritative.
    // Graph marker aligned with meta to isolate the mappings check.
    persistence::save_graph_generation(path, 5).expect("test: graph gen 5");
    persistence::save_mappings(path, &mappings_data(&mappings, 10))
        .expect("test: save mappings 10");
    persistence::save_vectors(path, &vectors_data(&vectors, 5)).expect("test: save vectors 5");
    persistence::save_meta(path, &build_meta(5)).expect("test: save meta 5");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    let err = load_sidecars(path, &loaded_meta)
        .expect_err("test: newer mappings than meta must trigger InvalidData");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("mappings generation"),
        "error should mention mappings generation, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Test 5 — backward compat: legacy 4-tuple meta + plain sidecars load
// -----------------------------------------------------------------------

#[test]
fn test_backward_compat_legacy_meta_without_generation_loads() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Pretend the database was written by a pre-fix binary: legacy 4-tuple
    // meta (v1.7.2+ storage mode, no generation), legacy 3-tuple mappings,
    // plain-Vec vectors.
    let meta = build_meta(0); // generation ignored by legacy writer
    write_legacy_4tuple_meta(path, &meta).expect("test: legacy meta");
    write_legacy_3tuple_mappings(path, &mappings_data(&mappings, 0))
        .expect("test: legacy mappings");
    write_legacy_plain_vectors(path, &vectors_data(&vectors, 0)).expect("test: legacy vectors");

    let loaded_meta = persistence::load_meta(path).expect("test: legacy meta loads");
    assert_eq!(
        loaded_meta.generation, 0,
        "legacy meta must default to generation 0"
    );

    let (_mappings, _vectors, enable_vs) =
        load_sidecars(path, &loaded_meta).expect("test: legacy sidecars load");
    assert!(enable_vs, "enable_vector_storage must round-trip from meta");
}

// -----------------------------------------------------------------------
// Test 6 — backward compat: pre-v1.7.2 3-tuple meta + legacy sidecars load
// -----------------------------------------------------------------------

#[test]
fn test_backward_compat_legacy_3tuple_meta_loads() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Pre-v1.7.2 meta: only (dim, metric, enable_vs) — no storage_mode, no
    // generation. Paired with legacy 3-tuple mappings + plain vectors.
    let meta = build_meta(0);
    write_legacy_3tuple_meta(path, &meta).expect("test: 3-tuple meta");
    write_legacy_3tuple_mappings(path, &mappings_data(&mappings, 0))
        .expect("test: legacy mappings");
    write_legacy_plain_vectors(path, &vectors_data(&vectors, 0)).expect("test: legacy vectors");

    let loaded_meta = persistence::load_meta(path).expect("test: 3-tuple meta loads");
    assert_eq!(
        loaded_meta.generation, 0,
        "3-tuple meta must default to generation 0"
    );
    assert_eq!(
        loaded_meta.storage_mode,
        StorageMode::Full,
        "3-tuple meta must default storage_mode to Full"
    );

    load_sidecars(path, &loaded_meta).expect("test: legacy sidecars load cleanly");
}

// -----------------------------------------------------------------------
// Test 7 — existing DB at generation 7 → save → load → generation 8
// -----------------------------------------------------------------------

#[test]
fn test_save_then_load_roundtrip_gen_bumped() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Prime the directory with a consistent state at generation 7.
    persistence::save_mappings(path, &mappings_data(&mappings, 7)).expect("test: seed mappings");
    persistence::save_vectors(path, &vectors_data(&vectors, 7)).expect("test: seed vectors");
    persistence::save_meta(path, &build_meta(7)).expect("test: seed meta");

    // Callers are expected to compute `next_generation(path)` and pass it
    // explicitly; this must read the current on-disk generation and bump to 8.
    let meta_in = build_meta(0); // caller-provided generation ignored
    let new_gen = persistence::next_generation(path).expect("test: next_generation");
    assert_eq!(new_gen, 8, "next_generation must bump from 7 to 8");
    save_sidecars(path, &mappings, &vectors, &meta_in, new_gen).expect("test: save bumps gen");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    assert_eq!(
        loaded_meta.generation, 8,
        "save_sidecars must bump to next generation"
    );
}

// -----------------------------------------------------------------------
// Test 8 — fresh directory: first save starts at generation 1
// -----------------------------------------------------------------------

#[test]
fn test_save_when_no_prior_state_starts_at_gen_1() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Fresh directory, no prior meta. Caller passes generation=0 (ignored).
    let meta_in = build_meta(0);
    let new_gen = persistence::next_generation(path).expect("test: next_generation");
    assert_eq!(
        new_gen, 1,
        "next_generation on a fresh directory must return 1"
    );
    save_sidecars(path, &mappings, &vectors, &meta_in, new_gen).expect("test: first save");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    assert_eq!(
        loaded_meta.generation, 1,
        "first save on a fresh directory must land at generation 1"
    );
}

// -----------------------------------------------------------------------
// Test 9 (Devin follow-up) — load_sidecars detects stale HNSW graph
// -----------------------------------------------------------------------

#[test]
fn test_load_sidecars_detects_stale_graph_generation() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();
    let mappings = build_mappings();
    let vectors = build_vectors();

    // Consistent state at generation 4 across all four artefacts.
    persistence::save_graph_generation(path, 4).expect("test: graph gen 4");
    persistence::save_mappings(path, &mappings_data(&mappings, 4)).expect("test: save mappings 4");
    persistence::save_vectors(path, &vectors_data(&vectors, 4)).expect("test: save vectors 4");
    persistence::save_meta(path, &build_meta(4)).expect("test: save meta 4");

    // Simulate a crash after the graph dump (new graph + new marker at gen=5)
    // but BEFORE any sidecar was rewritten — mappings / vectors / meta still
    // at gen=4.
    persistence::save_graph_generation(path, 5).expect("test: graph gen 5 only");

    let loaded_meta = persistence::load_meta(path).expect("test: reload meta");
    let err =
        load_sidecars(path, &loaded_meta).expect_err("test: stale graph must trigger InvalidData");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("graph generation"),
        "error should mention graph generation, got: {err}"
    );
}

// -----------------------------------------------------------------------
// Test 10 (Devin follow-up) — backward compat without graph marker
// -----------------------------------------------------------------------

#[test]
fn test_backward_compat_no_graph_generation_marker_loads_as_zero() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();

    // Fresh directory: no native_hnsw.gen exists → must read as 0 rather
    // than surfacing a NotFound error. This is what makes legacy DBs (all
    // sidecars at gen=0 by backward-compat default) pass the atomicity
    // check trivially.
    let observed = persistence::load_graph_generation(path)
        .expect("test: missing graph marker must not be an error");
    assert_eq!(
        observed, 0,
        "missing native_hnsw.gen must be treated as generation 0"
    );
}

// -----------------------------------------------------------------------
// Test 11 (Devin follow-up) — save_graph_generation round-trip
// -----------------------------------------------------------------------

#[test]
fn test_save_graph_generation_roundtrip() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();

    persistence::save_graph_generation(path, 42).expect("test: save marker");
    let observed = persistence::load_graph_generation(path).expect("test: reload marker");
    assert_eq!(observed, 42, "graph generation marker must round-trip");

    // Overwriting with a larger generation also round-trips (atomic_write
    // handles replacement).
    persistence::save_graph_generation(path, 9999).expect("test: overwrite marker");
    let observed = persistence::load_graph_generation(path).expect("test: reload marker 2");
    assert_eq!(observed, 9999, "overwritten marker must round-trip");
}

// -----------------------------------------------------------------------
// Test 12 (Devin follow-up #2) — corrupted meta must abort next save,
// not silently reset generation to 1
// -----------------------------------------------------------------------

#[test]
fn test_next_generation_propagates_corrupted_meta_error() {
    let dir = TempDir::new().expect("test: temp dir");
    let path = dir.path();

    // Write garbage bytes that postcard cannot parse as any of the 3/4/5
    // tuple shapes accepted by `load_meta`.
    std::fs::write(path.join("native_meta.bin"), [0xFF_u8; 32]).expect("test: seed corrupted meta");

    let result = persistence::next_generation(path);
    assert!(
        result.is_err(),
        "corrupted meta must propagate, not silently reset to gen 1 (got {result:?})"
    );

    // Missing meta (fresh directory) must NOT be an error — it is the
    // legitimate "start at generation 1" case.
    let fresh = TempDir::new().expect("test: fresh dir");
    let gen =
        persistence::next_generation(fresh.path()).expect("test: missing meta is not an error");
    assert_eq!(
        gen, 1,
        "missing meta must yield generation 1, not propagate NotFound"
    );
}
