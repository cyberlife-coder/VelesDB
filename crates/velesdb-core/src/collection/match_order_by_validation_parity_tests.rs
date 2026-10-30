//! Parity between the V015 validation rule and the MATCH executor.
//!
//! `velesql/validation_match_order.rs` refuses, before execution, the MATCH
//! `ORDER BY` forms `Collection::order_match_results` refuses with VELES-018
//! after traversal. Each form below goes through both — the executor through
//! `match_query_ordered`, which takes a `MatchClause` and never meets the
//! validator — and the two verdicts must agree.

use crate::collection::types::Collection;
use crate::point::Point;
use crate::velesql::{Parser, QueryValidator};
use std::collections::HashMap;
use tempfile::TempDir;

/// Every MATCH `ORDER BY` shape the grammar accepts for a `Doc` pattern.
const FORMS: &[&str] = &[
    "MATCH (d:Doc) RETURN d.year ORDER BY COUNT(*) DESC LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY MAX(year) LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY d LIMIT 3",
    "MATCH (d:Doc) RETURN d.year AS y ORDER BY y LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY year LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY similarity() DESC LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY similarity(d.embedding, $q) DESC LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY d.year DESC LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY year - 2000 DESC LIMIT 3",
    "MATCH (d:Doc) RETURN d.year ORDER BY depth LIMIT 3",
];

#[test]
fn validator_refuses_exactly_what_the_match_executor_refuses() {
    let dir = TempDir::new().expect("test: tempdir");
    let col = Collection::create(
        dir.path().join("docs"),
        4,
        crate::distance::DistanceMetric::Cosine,
    )
    .expect("test: create collection");
    col.upsert(vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({ "_labels": ["Doc"], "year": 2001 })),
    )])
    .expect("test: upsert");
    let params = HashMap::from([("q".to_string(), serde_json::json!([1.0, 0.0, 0.0, 0.0]))]);

    let mut refused = 0;
    for sql in FORMS {
        let query = Parser::parse(sql).unwrap_or_else(|e| panic!("{sql} must parse: {e}"));
        let validator_refuses = QueryValidator::validate(&query).is_err();
        let clause = query.match_clause.expect("test: MATCH clause");
        let executor_refuses = match col.match_query_ordered(&clause, &params) {
            Ok(_) => false,
            Err(e) if e.code() == "VELES-018" => true,
            Err(e) => panic!("{sql}: unexpected executor error {e}"),
        };
        assert_eq!(
            validator_refuses, executor_refuses,
            "{sql}: validator refuses = {validator_refuses}, executor refuses = {executor_refuses}"
        );
        refused += usize::from(validator_refuses);
    }
    // Both arms are exercised: some forms are refused, some are run.
    assert_eq!(refused, 5, "the five unsupported forms must be refused");
}
