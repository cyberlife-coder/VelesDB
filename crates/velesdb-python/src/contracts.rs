//! Module-level functions exposing velesdb-core contracts to Python.
//!
//! Each one answers a question the Python integrations (LangChain,
//! LlamaIndex, Haystack, through `velesdb_common`) would otherwise answer
//! with a hand-written copy of core logic: the canonical string → `u64` point
//! id, the direction of a metric's score, and the search quality grammar.
//! Every function is a call into core, so a change there reaches Python with
//! the next wheel instead of waiting for someone to edit a copy.

use pyo3::prelude::*;

/// The canonical string → `u64` point id: [`velesdb_core::hash_id`], FNV-1a
/// 64-bit over the UTF-8 bytes, the derivation core, `velesdb-memory` and
/// `velesdb-migrate` use for a non-numeric id.
#[pyfunction]
fn hash_id(value: &str) -> u64 {
    velesdb_core::hash_id(value)
}

/// Whether a higher score is a closer match under `metric`:
/// [`velesdb_core::DistanceMetric::higher_is_better`], the order core sorts
/// results by. `True` for the similarities (cosine, dot, Jaccard), `False`
/// for the distances (Euclidean, Hamming). Accepts the aliases
/// `Database.create_collection` accepts; raises `ValueError` for any other
/// name.
#[pyfunction]
fn metric_higher_is_better(metric: &str) -> PyResult<bool> {
    Ok(crate::utils::parse_metric(metric)?.higher_is_better())
}

/// Checks a search quality mode the way `Collection.search_with_quality`
/// will read it: [`velesdb_core::api_types::parse_search_mode`], raising
/// `ValueError` with core's message for a mode no search would accept.
#[pyfunction]
fn validate_search_quality(mode: &str) -> PyResult<()> {
    crate::collection::parse_search_quality(mode).map(drop)
}

/// Registers the module-level contract functions on the `velesdb` module.
pub fn register_contracts(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hash_id, m)?)?;
    m.add_function(wrap_pyfunction!(metric_higher_is_better, m)?)?;
    m.add_function(wrap_pyfunction!(validate_search_quality, m)?)?;
    Ok(())
}

#[cfg(test)]
#[path = "contracts_tests.rs"]
mod tests;
