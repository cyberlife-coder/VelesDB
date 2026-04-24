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
mod tests {
    use super::*;
    use velesdb_core::velesql::{
        Comparison, InCondition, Parser, SimilarityCondition, Value, VectorExpr,
    };

    fn parse_cond(sql: &str) -> Condition {
        let q = Parser::parse(sql).expect("test: parse");
        q.select.where_clause.expect("test: where")
    }

    // --- flip_compare_op ----------------------------------------------------

    #[test]
    fn test_flip_compare_op_is_logical_complement() {
        assert_eq!(flip_compare_op(CompareOp::Gt), CompareOp::Lte);
        assert_eq!(flip_compare_op(CompareOp::Gte), CompareOp::Lt);
        assert_eq!(flip_compare_op(CompareOp::Lt), CompareOp::Gte);
        assert_eq!(flip_compare_op(CompareOp::Lte), CompareOp::Gt);
        assert_eq!(flip_compare_op(CompareOp::Eq), CompareOp::NotEq);
        assert_eq!(flip_compare_op(CompareOp::NotEq), CompareOp::Eq);
    }

    // --- push_not_inward: pure AST-shape tests ------------------------------

    #[test]
    fn test_push_not_inward_no_not_is_identity_for_leaf() {
        let c = Condition::Comparison(Comparison {
            column: "x".into(),
            operator: CompareOp::Eq,
            value: Value::Integer(1),
        });
        assert_eq!(push_not_inward(c.clone()), c);
    }

    #[test]
    fn test_push_not_inward_not_comparison_flips_op() {
        let c = parse_cond("SELECT * FROM t WHERE NOT x = 1");
        let norm = push_not_inward(c);
        // Becomes `x != 1`.
        if let Condition::Comparison(cmp) = norm {
            assert_eq!(cmp.operator, CompareOp::NotEq);
        } else {
            panic!("expected Comparison, got {norm:?}");
        }
    }

    #[test]
    fn test_push_not_inward_not_and_becomes_or_of_nots() {
        // NOT (x = 1 AND y = 2) → x != 1 OR y != 2
        // The parser wraps parenthesized exprs in Group, so the actual
        // shape is `Group(Or(NotEq, NotEq))`.
        let c = parse_cond("SELECT * FROM t WHERE NOT (x = 1 AND y = 2)");
        let norm = push_not_inward(c);
        let inner = match norm {
            Condition::Group(g) => *g,
            other => panic!("expected Group, got {other:?}"),
        };
        match inner {
            Condition::Or(l, r) => {
                match *l {
                    Condition::Comparison(cmp) => assert_eq!(cmp.operator, CompareOp::NotEq),
                    other => panic!("expected Comparison, got {other:?}"),
                }
                match *r {
                    Condition::Comparison(cmp) => assert_eq!(cmp.operator, CompareOp::NotEq),
                    other => panic!("expected Comparison, got {other:?}"),
                }
            }
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn test_push_not_inward_not_or_becomes_and_of_nots() {
        // NOT (x = 1 OR y = 2) → x != 1 AND y != 2 (wrapped in Group).
        let c = parse_cond("SELECT * FROM t WHERE NOT (x = 1 OR y = 2)");
        let norm = push_not_inward(c);
        let inner = match norm {
            Condition::Group(g) => *g,
            other => panic!("expected Group, got {other:?}"),
        };
        match inner {
            Condition::And(l, r) => {
                assert!(
                    matches!(*l, Condition::Comparison(ref cmp) if cmp.operator == CompareOp::NotEq)
                );
                assert!(
                    matches!(*r, Condition::Comparison(ref cmp) if cmp.operator == CompareOp::NotEq)
                );
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn test_push_not_inward_double_negation_cancels() {
        // NOT NOT (x = 1) → x = 1
        let c = Condition::Not(Box::new(Condition::Not(Box::new(Condition::Comparison(
            Comparison {
                column: "x".into(),
                operator: CompareOp::Eq,
                value: Value::Integer(1),
            },
        )))));
        let norm = push_not_inward(c);
        assert!(matches!(
            norm,
            Condition::Comparison(ref cmp) if cmp.operator == CompareOp::Eq
        ));
    }

    #[test]
    fn test_push_not_inward_not_in_toggles_negated() {
        // NOT (x IN (1, 2)) → x NOT IN (1, 2)
        let c = Condition::Not(Box::new(Condition::In(InCondition {
            column: "x".into(),
            values: vec![Value::Integer(1), Value::Integer(2)],
            negated: false,
        })));
        let norm = push_not_inward(c);
        match norm {
            Condition::In(inc) => assert!(inc.negated),
            other => panic!("expected In(negated=true), got {other:?}"),
        }
    }

    #[test]
    fn test_push_not_inward_not_not_in_toggles_back() {
        // NOT (x NOT IN (1)) → x IN (1)
        let c = Condition::Not(Box::new(Condition::In(InCondition {
            column: "x".into(),
            values: vec![Value::Integer(1)],
            negated: true,
        })));
        let norm = push_not_inward(c);
        match norm {
            Condition::In(inc) => assert!(!inc.negated),
            other => panic!("expected In(negated=false), got {other:?}"),
        }
    }

    #[test]
    fn test_push_not_inward_not_similarity_flips_op() {
        // NOT (sim > 0.5) → sim <= 0.5
        let c = Condition::Not(Box::new(Condition::Similarity(SimilarityCondition {
            field: "vector".into(),
            vector: VectorExpr::Parameter("q".into()),
            operator: CompareOp::Gt,
            threshold: 0.5,
        })));
        let norm = push_not_inward(c);
        match norm {
            Condition::Similarity(s) => assert_eq!(s.operator, CompareOp::Lte),
            other => panic!("expected Similarity, got {other:?}"),
        }
    }

    #[test]
    fn test_push_not_inward_nested_compound() {
        // NOT (A OR (B AND sim > 0.5))
        //   → NOT A AND NOT (B AND sim > 0.5)
        //   → (A!) AND ((B!) OR (sim <= 0.5))
        // Parser groups every parenthesized subexpr, so the concrete
        // shape has Group wrappers that we peel through.
        let c = parse_cond(
            "SELECT * FROM t WHERE NOT (x = 1 OR (y = 2 AND similarity(vector, $q) > 0.5))",
        );
        let norm = push_not_inward(c);
        let top = match norm {
            Condition::Group(g) => *g,
            other => other,
        };
        let Condition::And(left, right) = top else {
            panic!("expected top AND");
        };
        assert!(
            matches!(*left, Condition::Comparison(ref cmp) if cmp.operator == CompareOp::NotEq)
        );
        // `right` may be Group-wrapped (from the inner parens). Peel if so.
        let right_inner = match *right {
            Condition::Group(g) => *g,
            other => other,
        };
        let Condition::Or(rl, rr) = right_inner else {
            panic!("expected inner OR");
        };
        assert!(matches!(*rl, Condition::Comparison(ref cmp) if cmp.operator == CompareOp::NotEq));
        assert!(matches!(*rr, Condition::Similarity(ref s) if s.operator == CompareOp::Lte));
    }

    #[test]
    fn test_push_not_inward_preserves_simple_predicates() {
        // `x = 1 AND y = 2` has no NOT → unchanged.
        let original = parse_cond("SELECT * FROM t WHERE x = 1 AND y = 2");
        assert_eq!(push_not_inward(original.clone()), original);
    }

    #[test]
    fn test_push_not_inward_keeps_not_on_like_leaf() {
        // NOT (name LIKE 'a%') — `LikeCondition` has no `negated` flag,
        // so we must keep the NOT wrapper (safe fallback). Executor
        // evaluates it by inversion.
        let c = parse_cond("SELECT * FROM t WHERE NOT name LIKE 'a%'");
        let norm = push_not_inward(c);
        match norm {
            Condition::Not(inner) => assert!(matches!(*inner, Condition::Like(_))),
            other => panic!("expected Not(Like), got {other:?}"),
        }
    }
}
