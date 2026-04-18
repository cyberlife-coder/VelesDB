//! Standardized dataset loaders for ANN benchmarks.
//!
//! Datasets are downloaded to `target/bench-data/<name>/` on first run
//! (respecting the per-dataset `VELESDB_<NAME>_DIR` env override). Files
//! are validated via SHA-256 fingerprints to catch corruption.
//!
//! Loaders are compiled only when the parent benchmark's feature flag
//! is enabled (e.g. `--features bench-sift1m`), keeping the download
//! dependencies (`flate2`, `tar`, `ureq`, `sha2`) out of the default
//! build graph.

#![allow(dead_code)]

pub mod sift1m;
