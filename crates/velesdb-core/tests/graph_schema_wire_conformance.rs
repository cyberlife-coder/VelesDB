//! `graph_schema` wire contract shared with the TypeScript SDK.
//!
//! `conformance/graph_schema_wire_cases.json` holds the `graph_schema` value
//! the SDK's `createGraphCollection` sends for each `schemaMode`
//! (`sdks/typescript/tests/graph-backend.test.ts` asserts it). velesdb-server
//! deserializes that field as a core [`GraphSchema`]; this test proves every
//! case does, and yields the mode it stands for.

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use velesdb_core::GraphSchema;

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    schema_mode: String,
    graph_schema: Value,
    is_schemaless: bool,
}

#[test]
fn graph_schema_wire_cases_parse_as_core_graph_schema() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/graph_schema_wire_cases.json");
    let content = std::fs::read_to_string(path).expect("read graph schema wire fixture");
    let fixture: Fixture = serde_json::from_str(&content).expect("parse graph schema fixture");
    assert!(!fixture.cases.is_empty(), "fixture has no cases");
    for case in fixture.cases {
        let schema: GraphSchema = serde_json::from_value(case.graph_schema).unwrap_or_else(|e| {
            panic!("{} must parse as a core GraphSchema: {e}", case.schema_mode)
        });
        assert_eq!(
            schema.is_schemaless(),
            case.is_schemaless,
            "{} yields the wrong mode",
            case.schema_mode
        );
    }
}
