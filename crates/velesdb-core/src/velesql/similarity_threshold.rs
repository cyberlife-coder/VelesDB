//! `similarity() <op> threshold`: the one comparison every engine applies —
//! core's SELECT and MATCH, and the WASM executor.
//!
//! `similarity()` reads "more similar" under every metric:
//! `similarity(v, $q) > 0.8` keeps the rows *closer* than 0.8, so under a distance metric
//! (`higher_is_better == false`) the relational operators compare the
//! distance the other way round. `=` and `!=` allow
//! [`SIMILARITY_EQ_TOLERANCE`], as a score computed in `f32` never lands
//! exactly on a decimal threshold. The comparison runs in `f64`, into which
//! an `f32` score converts exactly, so the threshold is the one the query
//! wrote.

use super::CompareOp;

/// How far a score may be from a threshold and still be `=` to it.
pub const SIMILARITY_EQ_TOLERANCE: f64 = 0.001;

/// Whether a similarity `score` satisfies `similarity() <op> threshold` (see
/// the module docs).
#[must_use]
pub fn similarity_holds(op: CompareOp, score: f32, threshold: f64, higher_is_better: bool) -> bool {
    let score = f64::from(score);
    let close = (score - threshold).abs() < SIMILARITY_EQ_TOLERANCE;
    match (op, higher_is_better) {
        (CompareOp::Eq, _) => close,
        (CompareOp::NotEq, _) => !close,
        (CompareOp::Gt, true) | (CompareOp::Lt, false) => score > threshold,
        (CompareOp::Gte, true) | (CompareOp::Lte, false) => score >= threshold,
        (CompareOp::Lt, true) | (CompareOp::Gt, false) => score < threshold,
        (CompareOp::Lte, true) | (CompareOp::Gte, false) => score <= threshold,
    }
}

#[cfg(test)]
#[path = "similarity_threshold_tests.rs"]
mod tests;
