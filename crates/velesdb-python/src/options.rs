//! Typed options dataclasses exposing core configuration to Python.
//!
//! Wave 3 B2 Commit 10 — replaces the flat `m=`, `ef_construction=`,
//! `expected_vectors=` kwargs on `Database.create_collection` with
//! explicit `#[pyclass]` dataclasses that map 1:1 to the core config
//! structs.
//!
//! Options types:
//! - [`HnswOptions`] — per-collection HNSW parameters (maps to
//!   [`velesdb_core::index::hnsw::HnswParams`])
//! - [`LimitsOptions`] — global tenant-wide limits (maps to
//!   [`velesdb_core::config::LimitsConfig`])
//! - [`AutoReindexOptions`] — per-collection auto-reindex policy (maps
//!   to [`velesdb_core::collection::auto_reindex::AutoReindexConfig`])
//! - [`SearchConfigOptions`] / [`HnswConfigOptions`] /
//!   [`StorageOptions`] / [`QuantizationOptions`] — engine config
//!   sections (issue #1549; map to the same-named
//!   `velesdb_core::config` sections)
//! - [`VelesConfigOptions`] — global database-level configuration
//!   wrapper (maps to [`velesdb_core::config::VelesConfig`]), with TOML
//!   loading via `from_toml` / `from_toml_path`
//!
//! There is no WAL batching option: the core `[wal_batch]` table it
//! would have mirrored was never wired and was removed in 7.0.0 (#2174).
//! Concurrent WAL writing is a velesdb-premium Enterprise feature (see
//! `docs/guides/WRITE_CONCURRENCY.md`).

mod auto_reindex;
mod config;
mod hnsw;
mod hnsw_config;
mod limits;
mod quantization;
mod search;
mod storage;

pub use auto_reindex::AutoReindexOptions;
pub use config::VelesConfigOptions;
pub use hnsw::HnswOptions;
pub use hnsw_config::HnswConfigOptions;
pub use limits::LimitsOptions;
pub use quantization::QuantizationOptions;
pub use search::SearchConfigOptions;
pub use storage::StorageOptions;

use pyo3::prelude::*;

/// Registers every option dataclass on the top-level `velesdb` module.
pub fn register_options(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<HnswOptions>()?;
    m.add_class::<LimitsOptions>()?;
    m.add_class::<AutoReindexOptions>()?;
    m.add_class::<SearchConfigOptions>()?;
    m.add_class::<HnswConfigOptions>()?;
    m.add_class::<StorageOptions>()?;
    m.add_class::<QuantizationOptions>()?;
    m.add_class::<VelesConfigOptions>()?;
    Ok(())
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
