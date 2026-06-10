//! Tests for bare table aliases — `FROM table alias` / `JOIN table alias`
//! without the `AS` keyword.
//!
//! The bare form must be strictly equivalent to the `AS` form (same AST,
//! same validation semantics, V011 anchor check included), and no clause
//! keyword following FROM/JOIN may ever be swallowed as an alias.

use crate::velesql::{JoinType, Parser, QueryValidator};

/// README showcase query #2, verbatim (bare FROM alias, no AS).
const SHOWCASE_QUERY: &str = "SELECT doc.*, similarity() FROM documents doc \
     WHERE vector NEAR $query AND MATCH (doc)-[:CITES]->(ref) \
     ORDER BY similarity() DESC";

// =============================================================================
// Positive: bare alias is equivalent to AS alias
// =============================================================================

#[test]
fn test_parse_from_bare_alias_equals_as_alias() {
    let bare = Parser::parse("SELECT * FROM employees e").expect("bare alias must parse");
    let with_as = Parser::parse("SELECT * FROM employees AS e").expect("AS alias must parse");

    assert_eq!(bare.select.from, "employees");
    assert_eq!(bare.select.from, with_as.select.from);
    assert_eq!(bare.select.from_alias, with_as.select.from_alias);
    assert!(bare.select.from_alias.contains(&"e".to_string()));
}

#[test]
fn test_qualified_wildcard_resolves_bare_alias() {
    let query = Parser::parse("SELECT doc.* FROM documents doc").expect("must parse");
    assert_eq!(query.select.from_alias, vec!["doc".to_string()]);
    QueryValidator::validate(&query).expect("doc.* must resolve against bare alias 'doc'");
}

#[test]
fn test_showcase_query_parses_and_validates() {
    let query = Parser::parse(SHOWCASE_QUERY).expect("showcase query #2 must parse");
    assert_eq!(query.select.from, "documents");
    assert_eq!(query.select.from_alias, vec!["doc".to_string()]);
    assert!(query.select.where_clause.is_some());
    assert!(query.select.order_by.is_some());
    QueryValidator::validate(&query).expect("showcase query #2 must validate (V011 anchor = doc)");
}

#[test]
fn test_explain_with_bare_alias_parses() {
    // EXPLAIN wraps a compound_query, so the bare alias must flow through.
    let query = Parser::parse("EXPLAIN SELECT doc.* FROM documents doc WHERE x = 1")
        .expect("EXPLAIN over a bare-alias query must parse");
    assert!(query.introspection.is_some());
}

#[test]
fn test_bare_alias_anchor_mismatch_rejected_like_as_alias() {
    // V011: MATCH anchor must equal a declared alias — bare form included.
    let sql = "SELECT m.* FROM agent_memory m \
               WHERE vector NEAR $q AND MATCH (ctx)-[:RELATES_TO]->(fact)";
    let query = Parser::parse(sql).expect("must parse");
    let err = QueryValidator::validate(&query).expect_err("anchor 'ctx' must not match alias 'm'");
    let msg = err.to_string();
    assert!(
        msg.contains("ctx"),
        "error must name the anchor, got: {msg}"
    );
}

#[test]
fn test_keyword_prefixed_identifiers_are_valid_bare_aliases() {
    // Word boundary: identifiers merely *starting* with a keyword stay usable.
    let query = Parser::parse("SELECT * FROM docs ordering").expect("must parse");
    assert_eq!(query.select.from_alias, vec!["ordering".to_string()]);

    let query = Parser::parse("SELECT * FROM docs unions WHERE x = 1").expect("must parse");
    assert_eq!(query.select.from_alias, vec!["unions".to_string()]);

    // "asset" starts with AS but is a plain identifier, not AS + "set".
    let query = Parser::parse("SELECT * FROM docs asset WHERE x = 1").expect("must parse");
    assert_eq!(query.select.from_alias, vec!["asset".to_string()]);
}

#[test]
fn test_quoted_identifier_escapes_alias_reservation() {
    let query = Parser::parse("SELECT * FROM docs `limit` WHERE x = 1").expect("must parse");
    assert_eq!(query.select.from_alias, vec!["limit".to_string()]);
}

// =============================================================================
// Negative: clause keywords must never be swallowed as a bare alias
// =============================================================================

#[test]
fn test_clause_keywords_not_swallowed_as_bare_alias() {
    let cases = [
        "SELECT * FROM docs WHERE x = 1",
        "SELECT * FROM docs LIMIT 5",
        "SELECT * FROM docs ORDER BY x ASC",
        "SELECT category, COUNT(*) FROM docs GROUP BY category",
        "SELECT category, COUNT(*) FROM docs GROUP BY category HAVING COUNT(*) > 1",
        "SELECT * FROM docs OFFSET 10",
        "SELECT * FROM docs WITH (max_groups = 100)",
        "SELECT * FROM docs USING FUSION",
    ];
    for sql in cases {
        let query = Parser::parse(sql).unwrap_or_else(|e| panic!("{sql} must parse: {e}"));
        assert!(
            query.select.from_alias.is_empty(),
            "clause keyword swallowed as alias in: {sql}"
        );
        assert_eq!(query.select.from, "docs");
    }
}

#[test]
fn test_clauses_still_bound_after_unaliased_from() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 5").expect("must parse");
    assert_eq!(query.select.limit, Some(5));

    let query = Parser::parse("SELECT * FROM docs WHERE x = 1").expect("must parse");
    assert!(query.select.where_clause.is_some());

    let query = Parser::parse("SELECT * FROM docs ORDER BY x ASC").expect("must parse");
    assert!(query.select.order_by.is_some());
}

#[test]
fn test_join_keywords_not_swallowed_as_bare_alias() {
    let query = Parser::parse("SELECT * FROM docs JOIN tags AS t ON docs.tag_id = t.id")
        .expect("must parse");
    assert_eq!(query.select.from_alias, vec!["t".to_string()]);
    assert_eq!(query.select.joins.len(), 1);

    let query = Parser::parse("SELECT * FROM docs LEFT JOIN tags AS t ON docs.tag_id = t.id")
        .expect("must parse");
    assert_eq!(query.select.joins[0].join_type, JoinType::Left);

    let query = Parser::parse("SELECT * FROM docs INNER JOIN tags AS t ON docs.tag_id = t.id")
        .expect("must parse");
    assert_eq!(query.select.joins[0].join_type, JoinType::Inner);
}

#[test]
fn test_set_operators_not_swallowed_as_bare_alias() {
    for op in ["UNION", "UNION ALL", "INTERSECT", "EXCEPT"] {
        let sql = format!("SELECT * FROM a {op} SELECT * FROM b");
        let query = Parser::parse(&sql).unwrap_or_else(|e| panic!("{sql} must parse: {e}"));
        assert!(
            query.select.from_alias.is_empty(),
            "set operator swallowed as alias in: {sql}"
        );
        assert!(query.compound.is_some(), "compound missing in: {sql}");
    }
}

#[test]
fn test_dangling_as_is_not_a_bare_alias() {
    assert!(
        Parser::parse("SELECT * FROM docs AS").is_err(),
        "dangling AS must not be parsed as a bare alias"
    );
}

// =============================================================================
// JOIN: bare alias parity with AS
// =============================================================================

#[test]
fn test_join_bare_alias_equals_as_alias() {
    let bare = Parser::parse("SELECT d.name, t.tag FROM docs d JOIN tags t ON d.tag_id = t.id")
        .expect("bare JOIN alias must parse");
    let with_as =
        Parser::parse("SELECT d.name, t.tag FROM docs AS d JOIN tags AS t ON d.tag_id = t.id")
            .expect("AS JOIN alias must parse");

    assert_eq!(bare.select.joins[0].alias, Some("t".to_string()));
    assert_eq!(bare.select.joins[0].table, "tags");
    assert_eq!(bare.select.from_alias, with_as.select.from_alias);
}

#[test]
fn test_join_spec_keywords_not_swallowed_as_bare_alias() {
    let query =
        Parser::parse("SELECT * FROM docs JOIN tags ON docs.tag_id = tags.id").expect("must parse");
    assert_eq!(query.select.joins[0].alias, None, "ON swallowed as alias");

    let query = Parser::parse("SELECT * FROM docs JOIN tags USING (id)").expect("must parse");
    assert_eq!(
        query.select.joins[0].alias, None,
        "USING swallowed as alias"
    );
}
