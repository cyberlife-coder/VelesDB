#![allow(clippy::doc_markdown)]

use serde::Deserialize;
use std::path::PathBuf;
use velesdb_core::velesql::Parser;

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<ParserCase>,
}

#[derive(Debug, Deserialize)]
struct ParserCase {
    id: String,
    query: String,
    should_parse: bool,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/velesql_parser_cases.json")
}

#[test]
fn test_cli_velesql_parser_conformance_fixture_cases() {
    let content = std::fs::read_to_string(fixture_path()).expect("read parser fixture");
    let fixture: Fixture = serde_json::from_str(&content).expect("parse parser fixture");

    for case in &fixture.cases {
        let parsed = Parser::parse(&case.query);
        assert_eq!(
            parsed.is_ok(),
            case.should_parse,
            "cli parser conformance failed for case {}",
            case.id
        );
    }
}
