//! USING FUSION semantic validation (bugs #10, #15, #16).
//!
//! Separated from `validation.rs` to keep each file under the 500 NLOC limit.
//! These checks run at validate-time so that misconfigured fusion clauses fail
//! loudly instead of silently degrading to RRF (or being decorative no-ops):
//!
//! - **#16** USING FUSION requires at least two fusable retrieval branches
//!   (NEAR + MATCH, NEAR + SPARSE_NEAR, …) or a single `NEAR_FUSED`.
//! - **#10** RSF weights must sum to ~1.0; Weighted weights must be
//!   non-negative — so the execution-time RRF fallback is unreachable.
//! - **#15** `NEAR_FUSED` rejects `weighted`/`rsf` (ill-defined over N
//!   homogeneous query vectors).

use super::ast::{Condition, FusionStrategyType, SelectStatement};
use super::validation_types::{ValidationError, ValidationErrorKind};

/// Tolerance for the RSF weight-sum check (matches `fusion::strategy`).
const WEIGHT_SUM_EPSILON: f32 = 0.001;

/// Counts of fusable retrieval branches present in a WHERE condition tree.
#[derive(Default)]
struct BranchCounts {
    near: usize,
    sparse: usize,
    text_match: usize,
    fused: usize,
}

impl BranchCounts {
    /// Total fusable branches, treating a `NEAR_FUSED` as already-fused (it
    /// carries its own multi-vector fusion and counts as one fusable unit).
    fn fusable_total(&self) -> usize {
        self.near + self.sparse + self.text_match + self.fused
    }
}

/// Walks the condition tree accumulating fusable-branch counts.
fn count_branches(cond: &Condition, counts: &mut BranchCounts) {
    match cond {
        Condition::VectorSearch(_) => counts.near += 1,
        Condition::SparseVectorSearch(_) => counts.sparse += 1,
        Condition::Match(_) => counts.text_match += 1,
        Condition::VectorFusedSearch(_) => counts.fused += 1,
        Condition::And(l, r) | Condition::Or(l, r) => {
            count_branches(l, counts);
            count_branches(r, counts);
        }
        Condition::Not(inner) | Condition::Group(inner) => count_branches(inner, counts),
        _ => {}
    }
}

/// Builds a FUSION applicability/misconfiguration validation error (#10/#16).
fn fusion_error(fragment: impl Into<String>, suggestion: impl Into<String>) -> ValidationError {
    ValidationError::new(
        ValidationErrorKind::SimilarityWithoutContext,
        None,
        fragment,
        suggestion,
    )
}

/// Rejects a negative weight pair, labelling the error with `strategy_label`
/// and naming the offending `weights` (e.g. `"Weighted vector_weight/graph_weight"`).
/// Generic over the weight float type (`dense`/`sparse` are `f32`,
/// `vector`/`graph` are `f64`).
fn reject_negative_pair<T: PartialOrd + Default + Copy>(
    strategy_label: &str,
    a: T,
    b: T,
    weights: &str,
) -> Result<(), ValidationError> {
    if a < T::default() || b < T::default() {
        return Err(fusion_error(
            strategy_label,
            format!("USING FUSION {weights} must be non-negative"),
        ));
    }
    Ok(())
}

/// Validates the USING FUSION clause (if any) against the WHERE condition.
pub(super) fn validate_fusion(stmt: &SelectStatement) -> Result<(), ValidationError> {
    let mut counts = BranchCounts::default();
    if let Some(ref cond) = stmt.where_clause {
        count_branches(cond, &mut counts);
    }

    // #15: NEAR_FUSED `weighted`/`rsf` are rejected regardless of the trailing
    // USING FUSION(...) clause, since the inline FusionConfig carries them.
    if counts.fused > 0 {
        validate_near_fused_strategy(stmt)?;
    }

    let Some(ref fc) = stmt.fusion_clause else {
        return Ok(());
    };

    validate_fusion_applicability(&counts)?;
    validate_fusion_weights(fc)
}

/// #16: USING FUSION requires at least two fusable branches, or a NEAR_FUSED.
fn validate_fusion_applicability(counts: &BranchCounts) -> Result<(), ValidationError> {
    // A single NEAR_FUSED is self-fusing and a valid FUSION target.
    if counts.fused > 0 {
        return Ok(());
    }
    if counts.fusable_total() >= 2 {
        return Ok(());
    }
    Err(fusion_error(
        "USING FUSION",
        "USING FUSION requires at least two fusable branches (e.g. vector NEAR + MATCH, \
         vector NEAR + SPARSE_NEAR) or a NEAR_FUSED predicate; a single-branch query \
         has nothing to fuse",
    ))
}

/// #10: validates the FUSION clause weight parameters at parse-time.
fn validate_fusion_weights(fc: &super::ast::FusionClause) -> Result<(), ValidationError> {
    match fc.strategy {
        FusionStrategyType::Rsf => validate_rsf_weights(fc),
        FusionStrategyType::Weighted => validate_weighted_weights(fc),
        _ => Ok(()),
    }
}

/// RSF dense/sparse weights must sum to ~1.0 and be non-negative.
fn validate_rsf_weights(fc: &super::ast::FusionClause) -> Result<(), ValidationError> {
    let dw = fc.dense_weight.unwrap_or(0.5);
    let sw = fc.sparse_weight.unwrap_or(0.5);
    reject_negative_pair(
        "USING FUSION(strategy='rsf')",
        dw,
        sw,
        "RSF dense_weight/sparse_weight",
    )?;
    if (dw + sw - 1.0).abs() > WEIGHT_SUM_EPSILON {
        return Err(fusion_error(
            "USING FUSION(strategy='rsf')",
            format!(
                "USING FUSION RSF dense_weight + sparse_weight must sum to 1.0 (got {})",
                dw + sw
            ),
        ));
    }
    Ok(())
}

/// Weighted weights must be non-negative on both the dense/sparse and the
/// vector/graph (NEAR + MATCH) branches.
///
/// The NEAR + MATCH `weighted` path normalizes
/// `vector_weight / (vector_weight + graph_weight)`, so a negative
/// `graph_weight` (or `vector_weight`) would otherwise pass validation and be
/// silently clamped to `1.0` at execution time. Rejecting it here keeps the
/// execution-time fallback unreachable.
fn validate_weighted_weights(fc: &super::ast::FusionClause) -> Result<(), ValidationError> {
    let dw = fc.dense_weight.unwrap_or(0.5);
    let sw = fc.sparse_weight.unwrap_or(0.5);
    reject_negative_pair(
        "USING FUSION(strategy='weighted')",
        dw,
        sw,
        "Weighted dense_weight/sparse_weight",
    )?;
    let vw = fc.vector_weight.unwrap_or(0.5);
    let gw = fc.graph_weight.unwrap_or(0.5);
    reject_negative_pair(
        "USING FUSION(strategy='weighted')",
        vw,
        gw,
        "Weighted vector_weight/graph_weight",
    )?;
    Ok(())
}

/// #15: rejects `weighted`/`rsf` on a `NEAR_FUSED` predicate.
fn validate_near_fused_strategy(stmt: &SelectStatement) -> Result<(), ValidationError> {
    let Some(ref cond) = stmt.where_clause else {
        return Ok(());
    };
    if fused_strategy_is_rejected(cond) {
        return Err(fusion_error(
            "NEAR_FUSED USING FUSION 'weighted'/'rsf'",
            "NEAR_FUSED USING FUSION supports only rrf, average, or maximum; weighted/rsf \
             are ill-defined over homogeneous query vectors",
        ));
    }
    Ok(())
}

/// Returns true if any `NEAR_FUSED` in the tree requests weighted/rsf fusion.
fn fused_strategy_is_rejected(cond: &Condition) -> bool {
    match cond {
        Condition::VectorFusedSearch(v) => {
            matches!(
                v.fusion.strategy.to_lowercase().as_str(),
                "weighted" | "rsf"
            )
        }
        Condition::And(l, r) | Condition::Or(l, r) => {
            fused_strategy_is_rejected(l) || fused_strategy_is_rejected(r)
        }
        Condition::Not(inner) | Condition::Group(inner) => fused_strategy_is_rejected(inner),
        _ => false,
    }
}
