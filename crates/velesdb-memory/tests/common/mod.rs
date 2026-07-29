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

/// A canned extractor: two facts that share the topic `rust` (and nothing
/// else), so the only path from one to the other runs through the shared
/// hub. One definition for every suite that needs a deterministic two-fact
/// graph — `extract_bdd` (graph liveness) and `forget_orphan_hubs_bdd`
/// (orphan collection) used to carry byte-identical private copies.
pub struct SharedTopicExtractor;

impl velesdb_memory::extract::Extractor for SharedTopicExtractor {
    fn extract(
        &self,
        _text: &str,
    ) -> Result<Vec<velesdb_memory::extract::ExtractedFact>, velesdb_memory::extract::ExtractError>
    {
        Ok(vec![
            velesdb_memory::extract::ExtractedFact {
                text: "Alice ships the parser in Rust.".to_string(),
                entities: vec!["rust".to_string(), "parser".to_string()],
            },
            velesdb_memory::extract::ExtractedFact {
                text: "Bob maintains the Rust toolchain.".to_string(),
                entities: vec!["rust".to_string()],
            },
        ])
    }
}
