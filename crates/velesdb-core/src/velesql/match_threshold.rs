//! The threshold of a MATCH query vector: the one rule REST `/match`, the
//! Python `match_query` calls and the WASM `executeMatch` apply, through
//! core, before any row is scored.
//!
//! A MATCH may score its rows against a query vector: each row's last node
//! is scored, and a threshold keeps the rows scoring at least it under a
//! similarity metric, at most it under a distance metric. This module says
//! which thresholds core can apply; where it applies one, the metric
//! decides the direction.

use crate::error::{Error, Result};

/// Refuses a MATCH threshold core cannot apply: one that is not a finite
/// number — no score passes a NaN, so it would drop every row without a
/// word, and an infinite one is the absence of a threshold `None` already
/// spells — and one given without a `query_vector`, which scores no row for
/// it to keep or drop. Any finite threshold with a vector is accepted: its
/// range is the metric's (a distance threshold can exceed 1).
///
/// # Errors
///
/// [`Error::Query`] (`VELES-010`) naming the threshold.
pub fn validate_match_threshold(
    query_vector: Option<&[f32]>,
    threshold: Option<f32>,
) -> Result<()> {
    match (query_vector, threshold) {
        (_, Some(t)) if !t.is_finite() => Err(Error::Query(format!(
            "MATCH threshold must be a finite number, got {t}"
        ))),
        (None, Some(t)) => Err(Error::Query(format!(
            "MATCH threshold {t} needs a query vector: without one no row is scored"
        ))),
        _ => Ok(()),
    }
}

/// Whether a row scoring `score` passes the MATCH `threshold`: it scores at
/// least the threshold under a similarity metric, at most it under a
/// distance metric — `similarity() >= threshold` read the way every
/// `similarity()` comparison reads ([`super::similarity_holds`]). `None`
/// keeps every scored row. The one rule REST `/match` and the WASM
/// `executeMatch` keep or drop a row by.
#[must_use]
pub fn match_threshold_keeps(threshold: Option<f32>, score: f32, higher_is_better: bool) -> bool {
    threshold.is_none_or(|threshold| {
        super::similarity_holds(
            super::CompareOp::Gte,
            score,
            f64::from(threshold),
            higher_is_better,
        )
    })
}

#[cfg(test)]
#[path = "match_threshold_tests.rs"]
mod tests;
