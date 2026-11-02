//! `graph_schema` wire contract shared with the TypeScript SDK.
//!
//! `conformance/graph_schema_wire_cases.json` holds the `graph_schema` value
//! the SDK's `createGraphCollection` sends for a given config
//! (`sdks/typescript/tests/graph-backend.test.ts` asserts it). velesdb-server
//! deserializes that field as a core [`GraphSchema`]; this test proves every
//! case does, yields the mode it stands for, and accepts and refuses the node
//! types and edges the fixture states.

use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use velesdb_core::GraphSchema;

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

/// An edge as `[label, from_type, to_type]`.
type Edge = (String, String, String);

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    graph_schema: Value,
    is_schemaless: bool,
    accepts_node_types: Vec<String>,
    refuses_node_types: Vec<String>,
    accepts_edges: Vec<Edge>,
    refuses_edges: Vec<Edge>,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/graph_schema_wire_cases.json");
    let content = std::fs::read_to_string(path).expect("read graph schema wire fixture");
    serde_json::from_str(&content).expect("parse graph schema fixture")
}

fn assert_types(case: &Case, schema: &GraphSchema) {
    for name in &case.accepts_node_types {
        assert!(
            schema.validate_node_type(name).is_ok(),
            "{}: {name} refused",
            case.id
        );
    }
    for name in &case.refuses_node_types {
        assert!(
            schema.validate_node_type(name).is_err(),
            "{}: {name} accepted",
            case.id
        );
    }
    for (label, from, to) in &case.accepts_edges {
        assert!(
            schema.validate_edge_type(label, from, to).is_ok(),
            "{}: edge {label} {from}->{to} refused",
            case.id
        );
    }
    for (label, from, to) in &case.refuses_edges {
        assert!(
            schema.validate_edge_type(label, from, to).is_err(),
            "{}: edge {label} {from}->{to} accepted",
            case.id
        );
    }
}

#[test]
fn graph_schema_wire_cases_parse_as_core_graph_schema() {
    let fixture = fixture();
    assert!(!fixture.cases.is_empty(), "fixture has no cases");
    for case in &fixture.cases {
        let schema: GraphSchema = serde_json::from_value(case.graph_schema.clone())
            .unwrap_or_else(|e| panic!("{} must parse as a core GraphSchema: {e}", case.id));
        assert_eq!(
            schema.is_schemaless(),
            case.is_schemaless,
            "{} yields the wrong mode",
            case.id
        );
        assert_types(case, &schema);
    }
}

#[test]
fn graph_schema_wire_cases_round_trip_through_core() {
    // What core serializes back is what the SDK sends: no field the SDK writes
    // is dropped or renamed on the way in.
    for case in fixture().cases {
        let schema: GraphSchema =
            serde_json::from_value(case.graph_schema.clone()).expect("parses");
        let back = serde_json::to_value(&schema).expect("serializes");
        assert_eq!(back, case.graph_schema, "{} does not round-trip", case.id);
    }
}
