//! The `_collection` rule: what it addresses, what it refuses, and how a
//! request's own collection fills it.

use super::*;

use serde_json::json;

use crate::velesql::Parser;

fn parse(sql: &str) -> Query {
    Parser::parse(sql).expect("test: valid VelesQL")
}

/// Params holding `_collection` bound to `value`.
fn collection(value: serde_json::Value) -> Params {
    Params::from([(COLLECTION_PARAM.to_string(), value)])
}

fn match_query() -> Query {
    parse("MATCH (n) RETURN n LIMIT 10")
}

#[test]
fn a_top_level_match_is_addressed_by_its_collection_param() {
    let p = collection(json!("g"));
    assert_eq!(collection_target(&match_query(), &p).ok(), Some(Some("g")));
    assert_eq!(
        collection_target(&match_query(), &Params::new()).ok(),
        Some(None)
    );
}

#[test]
fn a_from_decides_and_the_collection_param_is_not_read() {
    for sql in ["SELECT * FROM docs LIMIT 1", "SELECT COUNT(*) FROM docs"] {
        // Not even an invalid one: only a top-level MATCH reads the key.
        let p = collection(json!(7));
        assert_eq!(
            collection_target(&parse(sql), &p).ok(),
            Some(Some("docs")),
            "{sql}"
        );
    }
}

#[test]
fn a_statement_other_than_a_match_without_from_is_addressed_by_nothing() {
    let insert = parse("INSERT INTO docs (id, a) VALUES (1, 2)");
    let p = collection(json!("g"));
    assert_eq!(collection_target(&insert, &p).ok(), Some(None));
    let mut aggregate = parse("SELECT COUNT(*) FROM docs");
    aggregate.select.from.clear();
    assert_eq!(collection_target(&aggregate, &p).ok(), Some(None));
}

#[test]
fn an_empty_or_non_string_collection_param_is_a_parameter_error() {
    for bad in [json!(""), json!(7), json!(null), json!(["g"])] {
        let p = collection(bad);
        let err = collection_target(&match_query(), &p).expect_err("test: refused");
        assert_eq!(err.code(), "VELES-038", "{err}");
        assert!(
            matches!(&err, Error::QueryParameter { name, .. } if name == COLLECTION_PARAM),
            "{err}"
        );
        let direct = collection_param(&p).expect_err("test: refused");
        assert_eq!(direct.to_string(), err.to_string());
    }
}

#[test]
fn a_request_collection_fills_an_absent_param_of_a_top_level_match() {
    let empty = Params::new();
    let filled = with_collection_param(&match_query(), &empty, Some("body"));
    assert_eq!(filled.get(COLLECTION_PARAM), Some(&json!("body")));
}

#[test]
fn an_explicit_param_wins_and_nothing_else_is_filled() {
    let explicit = collection(json!("explicit"));
    assert!(matches!(
        with_collection_param(&match_query(), &explicit, Some("body")),
        Cow::Borrowed(p) if p.get(COLLECTION_PARAM) == Some(&json!("explicit"))
    ));
    let empty = Params::new();
    for (query, name) in [
        (match_query(), None),
        (match_query(), Some("")),
        (parse("SELECT * FROM docs LIMIT 1"), Some("body")),
        (
            parse("INSERT INTO docs (id, a) VALUES (1, 2)"),
            Some("body"),
        ),
    ] {
        assert!(matches!(
            with_collection_param(&query, &empty, name),
            Cow::Borrowed(p) if p.is_empty()
        ));
    }
}

/// The guard behind a rule concerning a top-level `MATCH` alone: the
/// grammar's `select_stmt` requires `FROM`, so no parsed `SELECT` —
/// aggregate or not — is without one.
#[test]
fn a_select_without_from_does_not_parse() {
    for sql in [
        "SELECT COUNT(*)",
        "SELECT COUNT(*) GROUP BY category",
        "SELECT *",
    ] {
        assert!(Parser::parse(sql).is_err(), "{sql} must not parse");
    }
}
