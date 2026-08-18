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
    let c =
        parse_cond("SELECT * FROM t WHERE NOT (x = 1 OR (y = 2 AND similarity(vector, $q) > 0.5))");
    let norm = push_not_inward(c);
    let top = match norm {
        Condition::Group(g) => *g,
        other => other,
    };
    let Condition::And(left, right) = top else {
        panic!("expected top AND");
    };
    assert!(matches!(*left, Condition::Comparison(ref cmp) if cmp.operator == CompareOp::NotEq));
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
