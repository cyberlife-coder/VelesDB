//! Tests for the V015 MATCH `ORDER BY` rule: each form the MATCH executor
//! refuses is refused at validation, and each form it sorts by is accepted.

use crate::velesql::{Parser, QueryValidator, ValidationError, ValidationErrorKind};

fn validate(sql: &str) -> Result<(), ValidationError> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("{sql} must parse: {e}"));
    QueryValidator::validate(&query)
}

fn assert_refused(sql: &str, fragment: &str) {
    let err = validate(sql).expect_err(sql);
    assert_eq!(
        err.kind,
        ValidationErrorKind::UnsupportedMatchOrderBy,
        "{sql}"
    );
    assert_eq!(err.kind.code(), "V015");
    assert_eq!(err.fragment, fragment, "{sql}");
    assert!(err.to_string().contains("[V015]"), "{err}");
    assert!(err.suggestion.contains("alias.property"), "{err}");
}

// =============================================================================
// Refused: the forms `order_match_results` answers with VELES-018
// =============================================================================

#[test]
fn test_count_star_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.name ORDER BY COUNT(*) DESC LIMIT 3",
        "ORDER BY COUNT(...)",
    );
}

#[test]
fn test_aggregate_over_a_property_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.name ORDER BY MAX(year) LIMIT 3",
        "ORDER BY MAX(...)",
    );
}

#[test]
fn test_bare_pattern_alias_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.name ORDER BY d LIMIT 3",
        "ORDER BY d",
    );
}

#[test]
fn test_return_alias_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.year AS y ORDER BY y DESC LIMIT 3",
        "ORDER BY y",
    );
}

#[test]
fn test_unqualified_property_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.name ORDER BY year LIMIT 3",
        "ORDER BY year",
    );
}

#[test]
fn test_one_unsupported_key_among_supported_ones_refused() {
    assert_refused(
        "MATCH (d:Doc) RETURN d.name ORDER BY d.year DESC, d LIMIT 3",
        "ORDER BY d",
    );
}

// =============================================================================
// Accepted: the forms the MATCH executor sorts by
// =============================================================================

#[test]
fn test_supported_forms_accepted() {
    for sql in [
        "MATCH (d:Doc) WHERE similarity(d.embedding, $q) > 0.5 RETURN d ORDER BY similarity() DESC LIMIT 3",
        "MATCH (d:Doc) RETURN d ORDER BY similarity(d.embedding, $q) DESC LIMIT 3",
        "MATCH (d:Doc) RETURN d.name ORDER BY d.year DESC LIMIT 3",
        "MATCH (d:Doc) RETURN d.name ORDER BY year - 2000 DESC LIMIT 3",
        "MATCH (a:Doc)-[:CITES*1..3]->(b) RETURN b.name ORDER BY depth ASC LIMIT 3",
        "MATCH (d:Doc) RETURN d.name ORDER BY d.year DESC, depth LIMIT 3",
        "MATCH (d:Doc) RETURN d.name LIMIT 3",
    ] {
        validate(sql).unwrap_or_else(|e| panic!("{sql} must validate: {e}"));
    }
}

#[test]
fn test_select_order_by_is_not_subject_to_the_match_rule() {
    // A bare name and an aggregate are SELECT ORDER BY forms; V015 is MATCH-only.
    validate("SELECT category, COUNT(*) FROM docs GROUP BY category ORDER BY COUNT(*) DESC")
        .expect("SELECT ORDER BY an aggregate must validate");
    validate("SELECT * FROM docs ORDER BY year DESC LIMIT 3")
        .expect("SELECT ORDER BY a bare field must validate");
}
