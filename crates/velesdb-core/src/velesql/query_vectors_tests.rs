use std::collections::HashMap;

use serde_json::json;

use super::{match_query_vectors, select_query_vectors, QueryVectorTarget};
use crate::error::Error;
use crate::velesql::{Parser, VectorExpr};

fn select_vectors(sql: &str) -> Vec<VectorExpr> {
    let query = Parser::parse(sql).expect("test: parse");
    select_query_vectors(&query.select)
        .into_iter()
        .cloned()
        .collect()
}

fn match_vectors(sql: &str) -> Vec<VectorExpr> {
    let query = Parser::parse(sql).expect("test: parse");
    match_query_vectors(query.match_clause.as_ref().expect("test: a MATCH"))
        .into_iter()
        .cloned()
        .collect()
}

fn param(name: &str) -> VectorExpr {
    VectorExpr::Parameter(name.to_string())
}

#[test]
fn select_collects_every_primary_vector_whatever_its_position() {
    let vectors = select_vectors(
        "SELECT * FROM d WHERE (NOT similarity(vector, $a) < 0.1 OR x = 1) \
         AND vector NEAR $b ORDER BY similarity(vector, $c) DESC LIMIT 3",
    );
    assert_eq!(vectors, vec![param("a"), param("b"), param("c")]);
}

#[test]
fn select_collects_a_similarity_inside_an_arithmetic_order_key() {
    let vectors = select_vectors(
        "SELECT * FROM d WHERE vector NEAR $a \
         ORDER BY 0.5 * similarity(vector, $b) + 0.5 * similarity(embedding, $c) DESC LIMIT 3",
    );
    assert_eq!(vectors, vec![param("a"), param("b")]);
}

#[test]
fn select_collects_every_fused_vector() {
    let vectors = select_vectors("SELECT * FROM d WHERE vector NEAR_FUSED [$a, $b] LIMIT 3");
    assert_eq!(vectors, vec![param("a"), param("b")]);
}

#[test]
fn select_leaves_named_vectors_to_the_rows() {
    let vectors = select_vectors(
        "SELECT * FROM d WHERE similarity(embedding, $a) > 0.1 \
         ORDER BY similarity(embedding, $b) DESC",
    );
    assert!(vectors.is_empty(), "{vectors:?}");
}

#[test]
fn match_collects_every_similarity_whatever_its_column() {
    let vectors = match_vectors(
        "MATCH (a)-[:R]->(b) WHERE NOT similarity(a.embedding, $a) > 0.1 \
         RETURN b ORDER BY similarity(b.vector, $b) DESC",
    );
    assert_eq!(vectors, vec![param("a"), param("b")]);
}

#[test]
fn a_target_without_vectors_refuses_before_binding() {
    let target = QueryVectorTarget {
        collection: "tags",
        dimension: None,
    };
    let unbound = param("missing");
    let err = target
        .check(&[&unbound], &HashMap::<String, serde_json::Value>::new())
        .expect_err("a metadata collection scores no query vector");
    assert!(
        matches!(err, Error::SearchNotSupported(ref name) if name == "tags"),
        "{err:?}"
    );
}

#[test]
fn a_target_checks_the_dimension_of_every_bound_vector() {
    let target = QueryVectorTarget {
        collection: "docs",
        dimension: Some(2),
    };
    let params = HashMap::from([
        ("ok".to_string(), json!([1.0, 0.0])),
        ("bad".to_string(), json!([1.0, 0.0, 0.0])),
    ]);
    let (ok, bad) = (param("ok"), param("bad"));
    assert!(target.check(&[&ok], &params).is_ok());
    let err = target
        .check(&[&ok, &bad], &params)
        .expect_err("the second vector has another dimension");
    assert!(
        matches!(
            err,
            Error::DimensionMismatch {
                expected: 2,
                actual: 3
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_statement_without_query_vectors_passes_any_target() {
    let target = QueryVectorTarget {
        collection: "tags",
        dimension: None,
    };
    assert!(target
        .check(&[], &HashMap::<String, serde_json::Value>::new())
        .is_ok());
}

/// The statement-level checks both engines call: a SELECT's or a MATCH's
/// vectors, and a MATCH's scoring vector before them.
#[test]
fn check_select_and_check_match_compose_the_statement_check() {
    let target = QueryVectorTarget {
        collection: "docs",
        dimension: Some(2),
    };
    let params = HashMap::from([("q".to_string(), json!([1.0, 0.0, 0.0]))]);
    let select = Parser::parse("SELECT * FROM docs WHERE NOT similarity(vector, $q) > 0.1")
        .expect("test: parse");
    assert_eq!(
        target
            .check_select(&select.select, &params)
            .map_err(|e| e.code()),
        Err("VELES-004")
    );
    let walk = Parser::parse("MATCH (a) RETURN a").expect("test: parse");
    let clause = walk.match_clause.as_ref().expect("test: a MATCH");
    assert!(target.check_match(clause, &params, None).is_ok());
    assert_eq!(
        target
            .check_match(clause, &params, Some(3))
            .map_err(|e| e.code()),
        Err("VELES-004")
    );
    let none = QueryVectorTarget {
        collection: "tags",
        dimension: None,
    };
    assert_eq!(
        none.check_match(clause, &params, Some(2))
            .map_err(|e| e.code()),
        Err("VELES-015")
    );
}
