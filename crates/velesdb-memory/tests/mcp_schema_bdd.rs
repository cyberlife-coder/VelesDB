//! The advertised MCP **input** schemas must be usable by a client that does
//! NOT dereference `$defs` — every array parameter has to carry a DIRECT
//! `type` on its `items`, not a bare `$ref`.
//!
//! Field report (2026-07-26): calling `save_working_context` for real took
//! four round trips of deserialization errors, because `working`'s nested
//! arrays (`verified_facts`, `decisions`, `exact_evidence`) advertised
//! `items: {"$ref": "#/$defs/…"}`. Harnesses that do not resolve `$defs`
//! degrade that to "untyped array of anything", so `ContextFact`,
//! `ContextDecisionRef` and `SourceReference` — and their REQUIRED fields
//! (`rule_id`, `handle`) — were invisible to the caller. Same failure class
//! as the `$ref`-only `working` parameter fixed on 2026-07-24, one level
//! deeper.
//!
//! The schemas are read exactly as published: an in-memory MCP client over
//! `tokio::io::duplex` (the idiom of rmcp's own upstream tests) drives the
//! real `McpServer`, so these tests see the same schema bytes a Claude Code /
//! Windsurf harness receives.
//!
//! Output schemas are covered too, since the same defect turned out to be
//! there and to bite harder: the official MCP SDKs validate a tool's
//! `structuredContent` against its advertised `outputSchema`, so an
//! unresolvable `$ref` is a RESULT the client may reject — worse than an
//! unparseable argument, where the server at least got to answer.

#![cfg(all(feature = "mcp", feature = "context", feature = "persistence"))]

use std::collections::BTreeSet;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tempfile::TempDir;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// A `u64` id well past 2^53 — the range every real `fragment_id` lives in
/// (they are FNV-1a 64 content hashes), and exactly the range a float-lossy
/// JSON client must relay as a decimal string.
const BIG_FRAGMENT_ID: &str = "12297829382473034410";

/// Boot the real `McpServer` over an in-memory duplex pipe and complete the
/// MCP handshake. The `TempDir` is returned so the caller keeps the store
/// alive for the test's duration.
async fn connected() -> (TempDir, RunningService<RoleClient, ()>) {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = McpServer::new(service).serve(server_side).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_side).await.expect("MCP initialize handshake over duplex");
    (store_dir, client)
}

fn as_args(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other:?}"),
    }
}

/// Collect every `items` schema reachable from a tool's input schema WITHOUT
/// resolving `$defs` — i.e. exactly what a non-dereferencing harness sees —
/// and report the ones that carry no direct `type` keyword.
///
/// `$defs` itself is deliberately not walked: it is the reference pool kept
/// for spec-correct clients, not an argument path. Any `$ref` still sitting
/// on a reachable `items` shows up here as "untyped", which is the point.
fn untyped_items(schema: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk_schema(schema, "$", &mut found);
    found
}

fn walk_schema(node: &Value, path: &str, found: &mut Vec<String>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(items) = map.get("items") {
        let items_path = format!("{path}.items");
        match items {
            Value::Array(entries) => {
                for (index, entry) in entries.iter().enumerate() {
                    check_items(entry, &format!("{items_path}[{index}]"), found);
                }
            }
            other => check_items(other, &items_path, found),
        }
    }
    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, sub) in properties {
            walk_schema(sub, &format!("{path}.{name}"), found);
        }
    }
    for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(entries)) = map.get(keyword) {
            for (index, entry) in entries.iter().enumerate() {
                walk_schema(entry, &format!("{path}.{keyword}[{index}]"), found);
            }
        }
    }
    if let Some(extra @ Value::Object(_)) = map.get("additionalProperties") {
        walk_schema(extra, &format!("{path}.additionalProperties"), found);
    }
}

/// One `items` slot: it must announce its own `type`, then it is walked like
/// any other schema node.
fn check_items(items: &Value, path: &str, found: &mut Vec<String>) {
    let has_type = matches!(items, Value::Object(map) if map.contains_key("type"));
    if !has_type {
        found.push(format!("{path} = {items}"));
    }
    walk_schema(items, path, found);
}

/// The regression that cost four round trips: `save_working_context`'s
/// nested arrays must be self-describing.
#[tokio::test]
async fn save_working_context_input_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "save_working_context")
        .expect("save_working_context is advertised");
    let schema = Value::Object((*tool.input_schema).clone());
    let untyped = untyped_items(&schema);
    assert!(
        untyped.is_empty(),
        "save_working_context advertises {} untyped `items` (a $defs-blind harness sees an \
         array of anything): {untyped:#?}",
        untyped.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// The generic form of the same rule, over EVERY advertised tool — so the
/// next tool that takes an array of objects is covered by construction, not
/// by someone remembering to add a case here.
#[tokio::test]
async fn every_tool_input_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let schema = Value::Object((*tool.input_schema).clone());
        for finding in untyped_items(&schema) {
            offenders.insert(format!("{}: {finding}", tool.name));
        }
    }
    assert!(
        offenders.is_empty(),
        "{} untyped `items` across the advertised input schemas: {offenders:#?}",
        offenders.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// Same rule on the OUTPUT side, which the input fix left behind.
///
/// It is not cosmetic: the official MCP SDKs validate a tool's
/// `structuredContent` against its advertised `outputSchema`. A `$ref` a
/// client cannot resolve is a result it may reject outright — a harsher
/// failure than the input case, where the server at least got to answer.
#[tokio::test]
async fn every_tool_output_schema_types_every_reachable_items() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut checked = 0usize;
    let mut offenders: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        let Some(output) = tool.output_schema.as_ref() else {
            continue;
        };
        checked += 1;
        let schema = Value::Object((**output).clone());
        for finding in untyped_items(&schema) {
            offenders.insert(format!("{}: {finding}", tool.name));
        }
    }
    assert!(checked > 0, "at least one tool advertises an output schema");
    assert!(
        offenders.is_empty(),
        "{} untyped `items` across {checked} advertised output schemas: {offenders:#?}",
        offenders.len()
    );
    client.cancel().await.expect("close the MCP session");
}

/// A full external payload — the shape a client would build from the schema
/// alone — including the two collections no existing fixture ever fills:
/// `decisions` (a `ContextDecisionRef`, `rule_id` required) and
/// `exact_evidence` (a `SourceReference`, `handle` required). `fragment_id`
/// is sent as a decimal STRING past 2^53, the form a float-lossy client must
/// use to relay an id without rounding it.
/// The payload an external client sends: every array field populated, so the
/// round-trip actually exercises `ContextDecisionRef` and `SourceReference`
/// rather than skipping them the way the in-crate fixtures do.
fn external_working_payload() -> serde_json::Value {
    json!({
        "goal": "ship the schema fix",
        "active_constraints": [
            { "text": "never break the published wire shape" }
        ],
        "verified_facts": [
            {
                "text": "the inliner only went one level deep",
                "source": {
                    "fragment_id": BIG_FRAGMENT_ID,
                    "handle": "ctx://source/schema-rs",
                    "memory_id": 42
                }
            }
        ],
        "open_hypotheses": [
            { "text": "output schemas have the same latent hole" }
        ],
        "decisions": [
            { "fragment_id": BIG_FRAGMENT_ID, "rule_id": "preserve.code_fence" }
        ],
        "exact_evidence": [
            { "fragment_id": BIG_FRAGMENT_ID, "handle": "ctx://source/evidence" }
        ],
        "pending_actions": ["re-run the gate"]
    })
}

#[tokio::test]
async fn save_working_context_round_trips_external_payload_with_decisions_and_evidence() {
    let (_store, client) = connected().await;
    let working = external_working_payload();
    let saved = client
        .call_tool(
            CallToolRequestParams::new("save_working_context").with_arguments(as_args(json!({
                "project": "velesdb",
                "session": "b6-schema",
                "working": working,
            }))),
        )
        .await
        .expect("save_working_context accepts a complete external payload");
    assert_ne!(
        saved.is_error,
        Some(true),
        "save_working_context reported a tool error: {:?}",
        saved.content
    );

    let loaded = client
        .call_tool(
            CallToolRequestParams::new("load_working_context").with_arguments(as_args(json!({
                "project": "velesdb",
                "session": "b6-schema",
            }))),
        )
        .await
        .expect("load_working_context call");
    let structured = loaded
        .structured_content
        .expect("load_working_context returns structured content");
    assert_eq!(structured["found"], json!(true));
    let round_tripped = &structured["working"];

    assert_payload_survived_the_round_trip(round_tripped);

    client.cancel().await.expect("close the MCP session");
}

/// Every field of [`external_working_payload`] must come back byte-identical.
/// The `fragment_id` assertions are the point of the test: sent as a decimal
/// string past 2^53, they must return as the exact integer, not rounded.
fn assert_payload_survived_the_round_trip(round_tripped: &serde_json::Value) {
    let expected_id: u64 = BIG_FRAGMENT_ID.parse().expect("the fixture id parses");
    assert_eq!(round_tripped["goal"], json!("ship the schema fix"));
    assert_eq!(
        round_tripped["decisions"][0]["fragment_id"].as_u64(),
        Some(expected_id),
        "the decision's fragment_id must survive exactly, not rounded"
    );
    assert_eq!(
        round_tripped["decisions"][0]["rule_id"],
        json!("preserve.code_fence")
    );
    assert_eq!(
        round_tripped["exact_evidence"][0]["fragment_id"].as_u64(),
        Some(expected_id)
    );
    assert_eq!(
        round_tripped["exact_evidence"][0]["handle"],
        json!("ctx://source/evidence")
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["fragment_id"].as_u64(),
        Some(expected_id)
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["handle"],
        json!("ctx://source/schema-rs")
    );
    assert_eq!(
        round_tripped["verified_facts"][0]["source"]["memory_id"].as_u64(),
        Some(42)
    );
    assert_eq!(
        round_tripped["active_constraints"][0]["text"],
        json!("never break the published wire shape")
    );
    assert_eq!(
        round_tripped["open_hypotheses"][0]["text"],
        json!("output schemas have the same latent hole")
    );
    assert_eq!(round_tripped["pending_actions"], json!(["re-run the gate"]));
}

/// Pruning unreferenced `$defs` must never orphan a surviving `$ref`.
///
/// Inlining copies definitions to their use sites, leaving most `$defs`
/// entries dead — 55 % of the published bytes, measured. Dropping them is
/// worth 40 KB across the 18 tools, but a definition still reachable through
/// a cycle guard's leftover `$ref`, or through another definition, MUST
/// survive. This walks every advertised schema and resolves every `$ref`
/// against what actually shipped.
#[tokio::test]
async fn no_advertised_ref_points_at_a_pruned_definition() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let mut dangling: BTreeSet<String> = BTreeSet::new();
    for tool in &tools {
        for (kind, schema) in [
            ("input", Some(Value::Object((*tool.input_schema).clone()))),
            (
                "output",
                tool.output_schema
                    .as_ref()
                    .map(|s| Value::Object((**s).clone())),
            ),
        ] {
            let Some(schema) = schema else { continue };
            let available: BTreeSet<String> = schema
                .get("$defs")
                .and_then(Value::as_object)
                .map(|defs| defs.keys().cloned().collect())
                .unwrap_or_default();
            let mut wanted = BTreeSet::new();
            collect_ref_targets(&schema, &mut wanted);
            for name in wanted.difference(&available) {
                dangling.insert(format!("{} ({kind}): #/$defs/{name}", tool.name));
            }
        }
    }
    assert!(
        dangling.is_empty(),
        "{} `$ref`(s) point at a definition that was pruned away: {dangling:#?}",
        dangling.len()
    );
    client.cancel().await.expect("close the MCP session");
}

fn collect_ref_targets(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    out.insert(name.to_owned());
                }
            }
            for sub in map.values() {
                collect_ref_targets(sub, out);
            }
        }
        Value::Array(entries) => {
            for sub in entries {
                collect_ref_targets(sub, out);
            }
        }
        _ => {}
    }
}

/// Every tool must ADVERTISE an output schema, not merely have a typed one.
///
/// Ten of the nineteen tools declared none at all, so rmcp derived one that
/// escaped every post-processing pass — the inliner included. A client
/// validating `structuredContent` against a `$ref` it cannot resolve may
/// reject a perfectly good result.
#[tokio::test]
async fn every_tool_advertises_an_output_schema() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let missing: BTreeSet<String> = tools
        .iter()
        .filter(|tool| tool.output_schema.is_none())
        .map(|tool| tool.name.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "{} tool(s) advertise no output schema: {missing:#?}",
        missing.len()
    );
    client.cancel().await.expect("close the MCP session");
}
