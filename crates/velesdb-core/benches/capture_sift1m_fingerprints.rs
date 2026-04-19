//! SIFT1M fingerprints capture bench.
//!
//! Single-shot utility: downloads SIFT1M (on first run), hashes the three
//! files, writes `benches/datasets/sift1m_fingerprints.json` ready to commit,
//! and prints the pastable JSON to stdout. Run once on the reference machine
//! to close `KNOWN_LIMITATIONS` #5 for the repository.
//!
//! Usage:
//! ```text
//! cargo bench -p velesdb-core --features bench-sift1m \
//!     --bench capture_sift1m_fingerprints
//! ```
//!
//! This bench does not use criterion — it produces a JSON artefact, not a
//! throughput measurement. `harness = false` in `Cargo.toml` lets it run as
//! a plain `fn main`.

#![cfg(feature = "bench-sift1m")]

#[path = "datasets/mod.rs"]
mod datasets;

use datasets::sift1m::{
    compute_fingerprints, default_cache_directory, write_pinned_fingerprints_json, DatasetError,
};

fn main() {
    match run() {
        Ok(written_path) => {
            eprintln!(
                "\n[capture] SIFT1M fingerprints written to: {}\n[capture] Commit this file to pin the dataset hashes for CI and reproducible benchmarks.",
                written_path.display()
            );
        }
        Err(e) => {
            eprintln!("[capture] failed: {e}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<std::path::PathBuf, DatasetError> {
    let cache_dir = default_cache_directory();
    eprintln!(
        "[capture] cache dir: {} (override with VELESDB_SIFT1M_DIR)",
        cache_dir.display()
    );
    let fingerprints = compute_fingerprints(&cache_dir)?;
    let json = serde_json::to_string_pretty(&fingerprints)
        .map_err(|e| DatasetError::Parse(format!("serialize: {e}")))?;
    println!("{json}");
    write_pinned_fingerprints_json(&fingerprints)
}
