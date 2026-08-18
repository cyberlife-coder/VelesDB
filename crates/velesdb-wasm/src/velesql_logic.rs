//! Boolean-logic normalization for VelesQL WHERE conditions (WASM).
//!
//! The only transform currently provided is [`push_not_inward`], which
//! applies De Morgan's laws to push `NOT` operators toward the leaves of
//! a [`Condition`] tree. Once pushed all the way in, the tree no longer
//! contains any `NOT` wrapping a compound expression — which means the
//! similarity extractor in [`crate::velesql_similarity`] and the strip
//! helper in [`crate::velesql_similarity::strip_condition_if`] can rely
//! on the invariant "every remaining `NOT` wraps an unsupported leaf",
//! keeping their polarity logic trivially correct.
//!
//! This is a pure rewrite: it never reports errors and preserves query
//! semantics. Predicates that don't carry a negation flag (e.g. `LIKE`,
//! `BETWEEN`, `MATCH`, `CONTAINS`, geo predicates, similarity with
//! unknown ops) keep their surrounding `NOT` as a safe fallback — the
//! executor still evaluates `NOT p` by inversion in that case, so the
//! behaviour is unchanged for these leaves.

use velesdb_core::velesql::{CompareOp, Condition};

use crate::velesql_similarity::flip_similarity_op;

/// Pushes every `NOT` operator in `cond` inward using De Morgan's laws.
///
/// Transformations:
/// - `NOT (A AND B)` → `NOT A OR NOT B`
/// - `NOT (A OR B)`  → `NOT A AND NOT B`
/// - `NOT NOT A`     → `A`
/// - `NOT (col op v)` → `col flip(op) v`
/// - `NOT (col IN (..))` / `NOT (col NOT IN (..))` → toggles `negated`
/// - `NOT (sim(...) op t)` → `sim(...) flip(op) t`
/// - `NOT (Group(x))` → `Group(push_not_inward(NOT x))`
/// - `NOT (any other leaf)` → `NOT leaf` (unchanged — the executor
///   already handles leaf negation by inversion, so polarity stays
///   correct; we just don't try to rewrite leaves that carry no
///   negation flag, such as `LIKE`, `BETWEEN`, `MATCH`, geo, ...).
///
/// The function is pure and total: it always returns a value, never
/// fails, and does not allocate beyond the boxed children required by
/// the shape of the resulting AST.
pub(crate) fn push_not_inward(cond: Condition) -> Condition {
    match cond {
        Condition::Not(inner) => push_not(*inner),
        Condition::And(l, r) => {
            Condition::And(Box::new(push_not_inward(*l)), Box::new(push_not_inward(*r)))
        }
        Condition::Or(l, r) => {
            Condition::Or(Box::new(push_not_inward(*l)), Box::new(push_not_inward(*r)))
        }
        Condition::Group(inner) => Condition::Group(Box::new(push_not_inward(*inner))),
        leaf => leaf,
    }
}

/// Applies a single `NOT` around `cond`, then recurses to keep pushing.
///
/// Split out from [`push_not_inward`] so the two directions (walk vs.
/// negate) are easy to reason about independently. When `cond` is a
/// leaf we can't negate (e.g. `LIKE`), we fall back to returning
/// `NOT leaf` unchanged — the WHERE evaluator still handles that by
/// inversion.
fn push_not(cond: Condition) -> Condition {
    match cond {
        // NOT NOT x → x, then keep pushing in case x still has NOTs.
        Condition::Not(inner) => push_not_inward(*inner),
        // NOT (A AND B) → (NOT A) OR (NOT B)
        Condition::And(l, r) => Condition::Or(Box::new(push_not(*l)), Box::new(push_not(*r))),
        // NOT (A OR B) → (NOT A) AND (NOT B)
        Condition::Or(l, r) => Condition::And(Box::new(push_not(*l)), Box::new(push_not(*r))),
        // NOT Group(x) → Group(NOT x), keep pushing inside.
        Condition::Group(inner) => Condition::Group(Box::new(push_not(*inner))),
        // NOT (col op v) → col flip(op) v
        Condition::Comparison(mut c) => {
            c.operator = flip_compare_op(c.operator);
            Condition::Comparison(c)
        }
        // NOT (col IN/NOT IN (...)) → toggle the negated flag.
        Condition::In(mut c) => {
            c.negated = !c.negated;
            Condition::In(c)
        }
        // NOT (sim(...) op t) → sim(...) flip(op) t (existing helper).
        Condition::Similarity(mut s) => {
            s.operator = flip_similarity_op(s.operator);
            Condition::Similarity(s)
        }
        // Leaves without a negation flag: `LIKE`, `BETWEEN`, `IS NULL`,
        // `MATCH`, `CONTAINS`, geo, vector searches, ... Keep the NOT
        // wrapper — the WHERE evaluator already handles `Not(leaf)` by
        // negating the inner boolean. This preserves current behaviour
        // for unsupported-in-WASM leaves, and leaves room to add direct
        // De Morgan mappings as those leaves gain negation flags.
        other => Condition::Not(Box::new(other)),
    }
}

/// 6-way flip for comparison operators. Mirrors [`flip_similarity_op`]
/// but lives here so the [`push_not`] path doesn't need to go through
/// the similarity module for plain comparisons.
fn flip_compare_op(op: CompareOp) -> CompareOp {
    match op {
        CompareOp::Gt => CompareOp::Lte,
        CompareOp::Gte => CompareOp::Lt,
        CompareOp::Lt => CompareOp::Gte,
        CompareOp::Lte => CompareOp::Gt,
        CompareOp::Eq => CompareOp::NotEq,
        CompareOp::NotEq => CompareOp::Eq,
        // Reason: `CompareOp` is `#[non_exhaustive]`. Unknown variants
        // keep their original operator (identity). The WHERE evaluator
        // also defaults such variants to `false`, so the worst case is
        // a conservative filter, never a silently wrong polarity.
        _ => op,
    }
}

#[cfg(test)]
#[path = "velesql_logic_tests.rs"]
mod tests;
