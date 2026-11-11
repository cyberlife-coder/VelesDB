//! Tests for the CLI-subcommand alias layer: `collection list` →
//! `.collections`, `query execute` → raw `VelesQL`, unmappable commands →
//! actionable hints, and everything else untouched.

use super::*;

fn assert_dot(input: &str, expected: &str) {
    match translate_cli_line(input) {
        CliLine::DotCommand(cmd) => assert_eq!(cmd, expected, "input: {input}"),
        _ => panic!("expected DotCommand({expected}) for {input:?}"),
    }
}

fn assert_hint(input: &str) {
    match translate_cli_line(input) {
        CliLine::Hint(hint) => assert!(!hint.is_empty(), "empty hint for {input:?}"),
        _ => panic!("expected Hint for {input:?}"),
    }
}

#[test]
fn test_collection_mappings() {
    assert_dot("collection list", ".collections");
    assert_dot("collection list ./db", ".collections");
    assert_dot("collection show docs", ".describe docs");
    assert_dot("collection show ./db docs", ".describe docs");
    assert_dot("collection analyze docs", ".analyze docs");
    assert_hint("collection create ./db docs -d 4");
    assert_hint("collection delete ./db docs");
    assert_hint("collection");
}

#[test]
fn test_data_mappings() {
    assert_dot("data export ./db docs", ".export docs");
    assert_dot("data export docs", ".export docs");
    assert_dot("data scroll ./db docs", ".scroll docs");
    assert_dot("data delete ./db docs 1 2", ".delete docs 1 2");
    assert_hint("data import ./db file.jsonl");
    assert_hint("data get ./db docs 5");
    assert_hint("data upsert ./db docs --id 1");
    assert_hint("data");
}

#[test]
fn test_query_execute_runs_sql() {
    match translate_cli_line("query execute \"SELECT * FROM docs LIMIT 5\"") {
        CliLine::Query(sql) => assert_eq!(sql, "SELECT * FROM docs LIMIT 5"),
        _ => panic!("expected Query, got nothing"),
    }
    // db-path positional of the CLI form is dropped
    match translate_cli_line("query execute ./db \"SELECT 1\"") {
        CliLine::Query(sql) => assert_eq!(sql, "SELECT 1"),
        _ => panic!("expected Query"),
    }
    match translate_cli_line("query execute 'SELECT 2'") {
        CliLine::Query(sql) => assert_eq!(sql, "SELECT 2"),
        _ => panic!("expected Query"),
    }
    assert_hint("query execute");
    assert_hint("query search ./db docs '[[1.0]]'");
    assert_hint("query");
}

#[test]
fn test_query_explain_maps() {
    assert_dot("query explain \"SELECT 1\"", ".explain SELECT 1");
    assert_dot("query explain ./db SELECT 1", ".explain SELECT 1");
}

#[test]
fn test_graph_index_agent_info() {
    assert_dot("graph edges kg", ".graph edges kg");
    assert_dot("graph get-edges ./db kg", ".graph edges kg");
    assert_dot("graph edges ./db kg", ".graph edges kg");
    assert_hint("graph");
    assert_dot("index list ./db docs", ".indexes docs");
    assert_hint("index create ./db docs field");
    assert_dot("info", "\\info");
    assert_dot(
        "agent what collections exist",
        ".agent what collections exist",
    );
    assert_dot("help", ".help");
}

#[test]
fn test_hints_for_shell_only_commands() {
    assert_hint("license show");
    assert_hint("simd info");
    assert_hint("completions bash");
    assert_hint("repl ./db");
}

#[test]
fn test_velesql_passthrough() {
    for sql in [
        "SELECT * FROM docs",
        "select * from docs",
        "MATCH (a)-[:KNOWS]->(b) RETURN a, b",
        "CREATE COLLECTION x (dimension = 4)",
        "SHOW COLLECTIONS",
        "LET x = 1 SELECT * FROM docs",
    ] {
        assert!(
            matches!(translate_cli_line(sql), CliLine::NotCli),
            "{sql} must stay a VelesQL query"
        );
    }
}

#[test]
fn test_case_insensitive_first_word() {
    assert_dot("COLLECTION LIST", ".collections");
    assert_dot("Collection Show docs", ".describe docs");
}

#[test]
fn test_looks_like_path() {
    assert!(looks_like_path("./db"));
    assert!(looks_like_path("/abs/path"));
    assert!(looks_like_path("rel/dir"));
    assert!(looks_like_path("."));
    assert!(looks_like_path(".."));
    assert!(looks_like_path("C:\\db"));
    assert!(!looks_like_path("docs"));
    assert!(!looks_like_path("my_collection"));
}

#[test]
fn test_strip_quotes() {
    assert_eq!(strip_quotes("\"SELECT 1\""), "SELECT 1");
    assert_eq!(strip_quotes("'SELECT 1'"), "SELECT 1");
    assert_eq!(strip_quotes("SELECT 1"), "SELECT 1");
    // mismatched quotes are left alone — better a parse error than a
    // silently mangled query
    assert_eq!(strip_quotes("\"SELECT 1'"), "\"SELECT 1'");
}
