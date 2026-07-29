//! `recall_fused`'s `pool` knob, seen from the wire.
//!
//! The three bindings (Node `{pool?}`, Python `{"pool": int}`, WASM
//! `{pool?}`) have all exposed the oversampled-pool depth for a while; the
//! MCP tool never did. That is the drift running the unusual way — the
//! server behind the bindings is the one lagging — and it is invisible to a
//! caller, because an unknown argument is not refused: `RecallFusedParams`
//! ignores what it does not declare, so `{"pool": 1}` returns a full-depth
//! result while looking accepted.
//!
//! So this suite asserts on the EFFECT, not on acceptance. `pool` is the
//! depth of the vector candidate pool fusion re-ranks: at `1` only the top
//! vector hit may enter, so a store holding several facts must come back
//! with a single memory even though `limit` allows ten. A tool that merely
//! swallowed the argument returns all of them, and the assertion names the
//! count.
//!
//! Everything is read through a real `McpServer` over `tokio::io::duplex`
//! (the idiom of `mcp_schema_bdd`), so both the schema and the behaviour
//! are the ones a Claude Code / Windsurf harness actually gets.

#![cfg(all(feature = "mcp", feature = "persistence"))]

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Map, Value};
use tempfile::TempDir;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// Facts with no `relate` edge between them, so nothing is graph-reached and
/// the returned count is exactly the vector pool's depth capped by `limit` —
/// the property under test, with no fusion promotion blurring it.
const FACTS: [&str; 6] = [
    "we chose parking_lot to avoid lock poisoning",
    "the on-call rotation moved to Tuesdays",
    "EPIC-317 xyzzy quux frobnicate",
    "PR #42 swaps the mutex implementation",
    "the release train leaves on Thursday",
    "the staging cluster runs three replicas",
];

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

/// Store [`FACTS`] through the `remember` tool, unlinked.
async fn seed(client: &RunningService<RoleClient, ()>) {
    for fact in FACTS {
        let stored = client
            .call_tool(
                CallToolRequestParams::new("remember")
                    .with_arguments(as_args(json!({ "fact": fact }))),
            )
            .await
            .expect("remember call");
        assert_ne!(
            stored.is_error,
            Some(true),
            "remember reported a tool error: {:?}",
            stored.content
        );
    }
}

/// Call `recall_fused` with `arguments` and return how many memories came
/// back.
async fn fused_count(client: &RunningService<RoleClient, ()>, arguments: Value) -> usize {
    let result = client
        .call_tool(
            CallToolRequestParams::new("recall_fused").with_arguments(as_args(arguments.clone())),
        )
        .await
        .unwrap_or_else(|e| panic!("recall_fused({arguments}) call: {e}"));
    assert_ne!(
        result.is_error,
        Some(true),
        "recall_fused({arguments}) reported a tool error: {:?}",
        result.content
    );
    result
        .structured_content
        .expect("recall_fused returns structured content")["memories"]
        .as_array()
        .expect("`memories` is an array")
        .len()
}

/// The knob has to be advertised before anyone can use it — and advertised
/// with a direct `type`, the invariant `mcp_schema_bdd` pins for every slot.
#[tokio::test]
async fn recall_fused_advertises_pool_as_a_typed_slot() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "recall_fused")
        .expect("recall_fused is advertised");
    let schema = Value::Object((*tool.input_schema).clone());
    let slot = schema["properties"].get("pool").unwrap_or_else(|| {
        panic!(
            "recall_fused advertises no `pool` parameter, though all three bindings expose \
             one; properties: {}",
            schema["properties"]
        )
    });
    assert!(
        slot.get("type").is_some(),
        "`pool` must advertise a direct `type` keyword (a $defs-blind harness stringifies \
         anything else); got: {slot}"
    );
    client.cancel().await.expect("close the MCP session");
}

/// The effect, not the acceptance: a narrowed pool admits fewer candidates,
/// so fewer memories come back than the same call left at the default.
#[tokio::test]
async fn a_narrowed_pool_admits_fewer_candidates_than_the_default() {
    let (_store, client) = connected().await;
    seed(&client).await;

    let default_depth = fused_count(&client, json!({ "query": FACTS[0], "limit": 10 })).await;
    assert_eq!(
        default_depth,
        FACTS.len(),
        "with the proven default pool, `limit: 10` returns every stored fact"
    );

    let narrowed = fused_count(
        &client,
        json!({ "query": FACTS[0], "limit": 10, "pool": 1 }),
    )
    .await;
    assert_eq!(
        narrowed, 1,
        "`pool: 1` admits only the top vector hit, so one memory comes back — \
         {default_depth} means the tool took the argument and threw it away"
    );

    client.cancel().await.expect("close the MCP session");
}

/// A harness that stringifies a schema-degraded scalar (`"2"`, not `2`) is
/// served identically — the tolerance every other `recall_fused` knob
/// already carries, extended to this one rather than left as a gap.
#[tokio::test]
async fn a_stringified_pool_is_honoured_like_a_numeric_one() {
    let (_store, client) = connected().await;
    seed(&client).await;

    let numeric = fused_count(
        &client,
        json!({ "query": FACTS[0], "limit": 10, "pool": 2 }),
    )
    .await;
    let stringified = fused_count(
        &client,
        json!({ "query": FACTS[0], "limit": 10, "pool": "2" }),
    )
    .await;
    assert_eq!(numeric, 2, "`pool: 2` admits two candidates");
    assert_eq!(
        stringified, numeric,
        "a stringified `pool` must reach the same depth as the numeric form"
    );

    client.cancel().await.expect("close the MCP session");
}

/// The two degenerate ends of an untrusted knob, which the engine — not the
/// transport — is responsible for absorbing: `0` must not oversample an
/// empty candidate set (it is floored at 1), and a colossal value must be
/// clamped rather than turned into an unbounded scan.
#[tokio::test]
async fn a_degenerate_pool_is_floored_and_a_colossal_one_clamped() {
    let (_store, client) = connected().await;
    seed(&client).await;

    let zero = fused_count(
        &client,
        json!({ "query": FACTS[0], "limit": 10, "pool": 0 }),
    )
    .await;
    assert_eq!(
        zero, 1,
        "`pool: 0` is floored at one candidate, never an empty result"
    );

    let colossal = fused_count(
        &client,
        json!({ "query": FACTS[0], "limit": 10, "pool": u64::MAX }),
    )
    .await;
    assert_eq!(
        colossal,
        FACTS.len(),
        "a colossal `pool` is clamped to the recall ceiling and still answers"
    );

    client.cancel().await.expect("close the MCP session");
}
