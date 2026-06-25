//! Shared helpers for the `velesdb-memory` integration tests.
//!
//! Uses the deterministic, network-free `HashEmbedder` so every suite is fully
//! reproducible and air-gapped (mirrors the repo's `fake_embed` examples).
#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use serde_json::Value;
use tempfile::TempDir;
use velesdb_memory::{HashEmbedder, MemoryService, Metadata};

/// Embedding dimension matching the SDK's `DEFAULT_DIMENSION`.
pub const DIM: usize = 384;

/// Open a fresh, isolated memory service backed by a tempdir.
///
/// The returned [`TempDir`] must be kept alive for the duration of the test.
pub fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("create tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open memory store");
    (dir, svc)
}

/// Build a [`Metadata`] map from key/value pairs.
pub fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}
