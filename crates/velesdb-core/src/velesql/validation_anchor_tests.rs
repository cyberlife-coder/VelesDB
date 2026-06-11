//! Tests for the V011 MATCH anchor rule — explicit and implicit binding
//! with guards G1 (declared alias in non-anchor position), G2 (alias shared
//! across graph predicates), and G3 (@collection override on the anchor).

use crate::velesql::{Parser, QueryValidator, ValidationError};

fn validate(sql: &str) -> Result<(), ValidationError> {
    let query = Parser::parse(sql).unwrap_or_else(|e| panic!("{sql} must parse: {e}"));
    QueryValidator::validate(&query)
}

// =============================================================================
// Explicit anchor and bare-FROM precedent (unchanged behavior)
// =============================================================================

#[test]
fn test_explicit_anchor_on_from_alias_accepted() {
    validate("SELECT * FROM docs AS d WHERE MATCH (d)-[:REL]->(x) LIMIT 10")
        .expect("explicit anchor on the FROM alias must validate");
}

#[test]
fn test_bare_from_accepts_any_anchor() {
    validate("SELECT * FROM docs WHERE MATCH (anything)-[:REL]->(x) LIMIT 10")
        .expect("a bare FROM declares no alias and accepts any anchor");
}

#[test]
fn test_missing_anchor_alias_still_rejected() {
    let err = validate("SELECT * FROM docs AS d WHERE MATCH (:Doc)-[:REL]->(x) LIMIT 10")
        .expect_err("an unaliased first node must stay rejected");
    assert!(
        err.to_string().contains("alias on the first node"),
        "error must explain the missing anchor alias, got: {err}"
    );
}

// =============================================================================
// Implicit anchor: no pattern alias matches a declared alias
// =============================================================================

#[test]
fn test_implicit_anchor_accepted_when_no_alias_matches() {
    validate("SELECT * FROM docs AS d WHERE MATCH (z)-[:REL]->(autre) LIMIT 10")
        .expect("implicit anchor must validate when no pattern alias matches FROM");
}

#[test]
fn test_flagship_agent_memory_query_validates() {
    // Q1, verbatim: 'ctx' binds implicitly to the agent_memory rows.
    validate(
        "SELECT memory.*, similarity() FROM agent_memory AS memory \
         WHERE vector NEAR $embedding AND MATCH (ctx)-[:RELATES_TO]->(fact) \
         AND session_id = $current_session ORDER BY similarity() DESC LIMIT 10",
    )
    .expect("flagship agent-memory query must validate verbatim");
}

#[test]
fn test_not_match_implicit_anchor_accepted() {
    // NOT MATCH is the exact dual of the positive case.
    validate("SELECT * FROM docs AS d WHERE NOT MATCH (z)-[:REL]->(x) LIMIT 10")
        .expect("implicit anchor under NOT must validate");
}

// =============================================================================
// G1: a declared alias in a non-anchor position forces that anchor
// =============================================================================

#[test]
fn test_g1_from_alias_in_non_anchor_position_rejected() {
    let err = validate("SELECT * FROM a AS x WHERE MATCH (w)-[:R]->(x) LIMIT 10")
        .expect_err("FROM alias 'x' in non-anchor position must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("V011"), "expected V011, got: {msg}");
    assert!(
        msg.contains("MATCH (x)-[:R]->(w)"),
        "suggestion must rewrite the user's pattern re-anchored on 'x', got: {msg}"
    );
}

#[test]
fn test_g1_rejected_under_not() {
    let err = validate("SELECT * FROM a AS x WHERE NOT MATCH (w)-[:R]->(x) LIMIT 10")
        .expect_err("G1 must apply uniformly under NOT");
    assert!(err.to_string().contains("V011"), "expected V011: {err}");
}

// =============================================================================
// G2: implicit anchor shared with another graph predicate is ambiguous
// =============================================================================

#[test]
fn test_g2_shared_alias_across_predicates_rejected_with_chain_hint() {
    let err = validate(
        "SELECT * FROM a AS x \
         WHERE MATCH (m)-[:R]->(f) AND MATCH (f)-[:S]->(g) LIMIT 10",
    )
    .expect_err("implicit anchor 'f' shared across MATCH predicates must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("V011"), "expected V011, got: {msg}");
    assert!(
        msg.contains("chain into a single pattern"),
        "error must hint at chaining the patterns, got: {msg}"
    );
}

#[test]
fn test_g2_disjoint_aliases_across_predicates_accepted() {
    validate(
        "SELECT * FROM a AS x \
         WHERE MATCH (m)-[:R]->(f) AND MATCH (p)-[:S]->(q) LIMIT 10",
    )
    .expect("disjoint implicit anchors must validate");
}

// =============================================================================
// G3: a @collection override on the anchor cannot bind to the FROM rows
// =============================================================================

#[test]
fn test_g3_collection_override_on_anchor_rejected() {
    let err = validate("SELECT * FROM a AS x WHERE MATCH (p@other)-[:R]->(y) LIMIT 10")
        .expect_err("@collection anchor outside FROM aliases must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("V011"), "expected V011, got: {msg}");
    assert!(
        msg.contains("@collection"),
        "error must name the @collection override, got: {msg}"
    );
}
