//! Behaviour: `list_memories` lets a user AUDIT their store — "what does my
//! agent know?" — which `recall` structurally cannot answer: recall ranks by
//! resemblance to a query, so what resembles nothing you thought to ask is
//! invisible, and invisible is precisely what an audit must not have.
//!
//! The contract, each clause with its own case: the walk is exhaustive and
//! cursor-paginated (ids ascending — deterministic, so two audits see the
//! same order); internal entity hubs are EXCLUDED by default (they are the
//! graph's scaffolding, not the user's facts) and included on request;
//! reserved `_veles_*` keys are stripped exactly as `recall` strips them,
//! except the auto-stamped date which the caller may legitimately want; a
//! metadata filter narrows a page WITHOUT breaking the cursor walk.
//!
//! Wire-level like the other BDD suites: the REAL server over an in-memory
//! duplex, raw tool calls, assertions on `structuredContent`.

// `mcp` (which implies `persistence`), not `persistence`: this file boots the
// MCP server itself, and the feature-isolation matrix checks `persistence`
// ALONE with --all-targets — under which `McpServer` does not exist.
#![cfg(feature = "mcp")]

use rmcp::model::CallToolRequestParams;
use rmcp::service::ServiceExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_memory::{DynEmbedder, HashEmbedder, McpServer, MemoryService, DEFAULT_DIMENSION};

async fn connected() -> (
    TempDir,
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
) {
    let store_dir = tempfile::tempdir().expect("create scratch store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(DEFAULT_DIMENSION));
    let service =
        MemoryService::open(store_dir.path(), embedder).expect("open scratch memory store");
    let server = McpServer::new(service);
    let (server_side, client_side) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_side).await {
            let _ = running.waiting().await;
        }
    });
    let client = ().serve(client_side).await.expect("MCP initialize handshake over duplex");
    (store_dir, client)
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    tool: &str,
    args: Value,
) -> Value {
    let mut request = CallToolRequestParams::new(tool.to_owned());
    if let Value::Object(map) = args {
        request = request.with_arguments(map);
    }
    let result = client
        .call_tool(request)
        .await
        .unwrap_or_else(|e| panic!("tool {tool} failed: {e}"));
    result
        .structured_content
        .unwrap_or_else(|| panic!("tool {tool} returned no structuredContent"))
}

/// Walk the whole store through the cursor, page by page, and hand back
/// every listed memory. Bounded (32 pages) so a broken cursor loops into a
/// test failure, never a hang.
async fn list_all(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    mut args: Value,
) -> Vec<Value> {
    let mut out = Vec::new();
    for _ in 0..32 {
        let page = call(client, "list_memories", args.clone()).await;
        let memories = page["memories"].as_array().expect("memories array");
        out.extend(memories.iter().cloned());
        match &page["next_cursor"] {
            Value::String(cursor) => {
                args["cursor"] = Value::String(cursor.clone());
            }
            Value::Null => return out,
            other => panic!("next_cursor is a string or null, got {other:?}"),
        }
    }
    panic!("cursor never terminated across 32 pages");
}

#[tokio::test]
async fn the_walk_is_exhaustive_paginated_and_ordered() {
    let (_dir, client) = connected().await;

    let mut ids = Vec::new();
    for i in 0..5 {
        let stored = call(
            &client,
            "remember",
            json!({"fact": format!("fait numero {i} pour l'audit")}),
        )
        .await;
        ids.push(stored["id_str"].as_str().expect("id_str").to_owned());
    }

    // Page size 2 over 5 facts: the audit still sees all 5, exactly once.
    let listed = list_all(&client, json!({"limit": 2})).await;
    assert_eq!(listed.len(), 5, "every stored fact is listed exactly once");
    let listed_ids: Vec<&str> = listed
        .iter()
        .map(|m| m["id_str"].as_str().expect("id_str on each memory"))
        .collect();
    let mut sorted = listed_ids.clone();
    sorted.sort_by_key(|s| s.parse::<u64>().expect("decimal id"));
    assert_eq!(
        listed_ids, sorted,
        "ids come back ascending — two audits of the same store see the same order"
    );
    for id in &ids {
        assert!(
            listed_ids.contains(&id.as_str()),
            "stored id {id} missing from the audit"
        );
    }
    assert!(
        listed
            .iter()
            .all(|m| m["content"].as_str().is_some_and(|c| !c.is_empty())),
        "each listed memory carries its content"
    );
}

#[tokio::test]
async fn hubs_are_scaffolding_not_facts_unless_asked_for() {
    let (_dir, client) = connected().await;

    // remember + relate builds entity hubs behind the facts when extraction
    // runs; without an extractor, hubs come from `remember`'s entity wiring.
    // Store two facts and link them: the edge's endpoints are user facts, so
    // the default listing must show exactly the two facts — whatever
    // scaffolding the store keeps besides them stays out of the audit.
    let a = call(
        &client,
        "remember",
        json!({"fact": "alice travaille chez wiscale"}),
    )
    .await;
    let b = call(&client, "remember", json!({"fact": "wiscale est a lille"})).await;
    call(
        &client,
        "relate",
        json!({"from": a["id_str"], "to": b["id_str"], "relation": "situe"}),
    )
    .await;

    let default_view = list_all(&client, json!({})).await;
    assert_eq!(
        default_view.len(),
        2,
        "the default audit lists the user's facts and nothing else"
    );

    let full_view = list_all(&client, json!({"include_internal": true})).await;
    assert!(
        full_view.len() >= default_view.len(),
        "the internal view is a superset — it may add scaffolding, never hide facts"
    );
}

#[tokio::test]
async fn reserved_keys_are_stripped_like_recall_strips_them() {
    let (_dir, client) = connected().await;
    call(
        &client,
        "remember",
        json!({"fact": "le budget est de 12000 euros", "metadata": {"project": "acme"}}),
    )
    .await;

    let listed = list_all(&client, json!({})).await;
    assert_eq!(listed.len(), 1);
    let metadata = listed[0]["metadata"].as_object().expect("metadata object");
    assert_eq!(
        metadata.get("project"),
        Some(&json!("acme")),
        "business metadata is listed"
    );
    assert!(
        metadata.get("_veles_date").is_some(),
        "the auto-stamped date survives — an audit legitimately asks WHEN"
    );
    assert!(
        !metadata
            .keys()
            .any(|k| k.starts_with("_veles_") && k != "_veles_date"),
        "every other reserved key is stripped, exactly as recall strips them: {metadata:?}"
    );
}

#[tokio::test]
async fn a_metadata_filter_narrows_without_breaking_the_walk() {
    let (_dir, client) = connected().await;
    for i in 0..3 {
        call(
            &client,
            "remember",
            json!({"fact": format!("fait acme {i}"), "metadata": {"project": "acme"}}),
        )
        .await;
    }
    for i in 0..2 {
        call(
            &client,
            "remember",
            json!({"fact": format!("fait globex {i}"), "metadata": {"project": "globex"}}),
        )
        .await;
    }

    // Page size 2 with a filter: pages may come back sparse (the cursor
    // advances over non-matching facts), but the WALK still finds exactly
    // the three acme facts.
    let acme = list_all(&client, json!({"limit": 2, "filter": {"project": "acme"}})).await;
    assert_eq!(acme.len(), 3, "the filter selects exactly the acme facts");
    assert!(
        acme.iter().all(|m| m["metadata"]["project"] == "acme"),
        "no foreign fact leaks through the filter"
    );
}
