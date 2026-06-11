//! Anti-drift guard for the `docs/VELESQL_SPEC.md` examples corrected during
//! the 2026-06 documentation alignment sweep.
//!
//! `velesql_cheatsheet_docs.rs` parses the cheat sheet wholesale; the spec is
//! too large and contains deliberate pseudo-grammar fragments, so this guard
//! pins the *corrected* statements verbatim instead. If the grammar regresses
//! on one of them — or starts accepting a form the spec documents as invalid —
//! the spec must be revisited.

use velesdb_core::velesql::{Parser, QueryValidator, ValidationConfig};

/// Statements that the spec now shows and that must keep parsing AND
/// passing semantic validation with the default configuration.
const CORRECTED_EXAMPLES: &[&str] = &[
    // LET + SELECT * (spec "LET Bindings" / "Hybrid Search" examples; the
    // former `SELECT *, binding AS alias` form is not valid syntax).
    "LET relevance = 0.7 * vector_score + 0.3 * bm25_score\n\
     SELECT * FROM documents\n\
     WHERE vector NEAR $query AND content MATCH 'machine learning'\n\
     ORDER BY relevance DESC LIMIT 10",
    // Window ORDER BY on a column (not on an aggregate expression).
    "SELECT author, year, COUNT(*) AS papers,\n\
     DENSE_RANK() OVER (PARTITION BY author ORDER BY year DESC) AS recency_rank\n\
     FROM publications GROUP BY author, year",
    // Graph predicate anchored on the FROM alias (rule V011).
    "SELECT * FROM docs AS d\n\
     WHERE category = 'tech' AND MATCH (d)-[:REL]->(x)\n\
     LIMIT 10",
    // Bounded variable-length range at the documented cap.
    "MATCH (src)-[:LINKS*1..32]->(dst) RETURN dst LIMIT 10",
    // Plain EXPLAIN remains a parsed statement.
    "EXPLAIN SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10",
];

/// Forms the spec now documents as invalid; they must keep failing to parse.
const DOCUMENTED_PARSE_REJECTS: &[&str] = &[
    // `SELECT *, expr` is not in the grammar (select_list is `*` XOR a list).
    "SELECT *, relevance AS score FROM documents LIMIT 10",
    // EXPLAIN ANALYZE is an API-level mode, not a parsed statement.
    "EXPLAIN ANALYZE SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
    // Window ORDER BY only accepts columns or similarity().
    "SELECT author, COUNT(*) AS papers,\n\
     DENSE_RANK() OVER (PARTITION BY author ORDER BY COUNT(*) DESC) AS r\n\
     FROM publications GROUP BY author",
];

#[test]
fn corrected_spec_examples_parse_and_validate() {
    for statement in CORRECTED_EXAMPLES {
        let query = Parser::parse(statement)
            .unwrap_or_else(|e| panic!("spec example failed to parse:\n  {statement}\n  {e:?}"));
        QueryValidator::validate(&query)
            .unwrap_or_else(|e| panic!("spec example failed validation:\n  {statement}\n  {e:?}"));
        QueryValidator::enforce_query_complexity(&query, statement, &ValidationConfig::default())
            .unwrap_or_else(|e| {
                panic!("spec example exceeded complexity budget:\n  {statement}\n  {e:?}")
            });
    }
}

#[test]
fn documented_invalid_forms_still_fail_to_parse() {
    for statement in DOCUMENTED_PARSE_REJECTS {
        assert!(
            Parser::parse(statement).is_err(),
            "spec documents this form as invalid but it now parses:\n  {statement}"
        );
    }
}

#[test]
fn unbounded_ranges_are_rejected_at_validation() {
    // `*` and `*2..` map to an open upper bound; the complexity budget
    // (DEFAULT_MAX_GRAPH_EXPANSION = 32) rejects them inside Parser::parse.
    for statement in [
        "MATCH (src)-[:LINKS*]->(dst) RETURN dst LIMIT 10",
        "MATCH (src)-[:LINKS*2..]->(dst) RETURN dst LIMIT 10",
    ] {
        let err = Parser::parse(statement)
            .expect_err("spec documents open-ended ranges as rejected, but parse accepted");
        assert!(
            format!("{err:?}").contains("Graph expansion"),
            "expected a 'Graph expansion' complexity error for:\n  {statement}\n  got: {err:?}"
        );
    }
}

#[test]
fn anchor_alias_mismatch_is_rejected_with_v011() {
    let statement = "SELECT * FROM docs AS d\n\
         WHERE category = 'tech' AND MATCH (ctx)-[:REL]->(x)\n\
         LIMIT 10";
    let query = Parser::parse(statement).expect("anchor-mismatch example must parse");
    let err = QueryValidator::validate(&query)
        .expect_err("spec documents anchor mismatch as a V011 validation error");
    assert!(
        format!("{err:?}").contains("V011") || format!("{err}").contains("V011"),
        "expected a V011 error, got: {err:?}"
    );
}
