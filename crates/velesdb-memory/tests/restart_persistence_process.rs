//! A memory written by one server process is explained by the next one.
//!
//! This is what an agent harness actually does between two sessions: the
//! first `velesdb-memory` process stores a decision and its reason, the
//! harness closes stdin, the process exits — and a *new* OS process, started
//! later on the same store, must still answer `why` with that reason.
//!
//! The in-process reopen tests (`reinforce_persistence.rs`,
//! `context_memory_bdd.rs`) drop a `MemoryService` and reopen it inside one
//! test binary; they cannot catch state that only lives in the process — a
//! cache that is never flushed, a lock that is never released, an index that
//! is rebuilt from memory instead of from disk. Two real processes can.
//!
//! The reason deliberately shares no word with the question, so the second
//! process can only reach it by walking the persisted typed link, not by
//! similarity.
#![cfg(all(feature = "mcp", feature = "persistence"))]

mod common;

use common::mcp_stdio::McpStdioProcess;
use serde_json::{json, Value};

const DECISION: &str = "Booked the aisle seat on Robert's flight";
const REASON: &str = "Knee surgery last month limits leg movement";
const QUESTION: &str = "why the aisle seat on Robert's flight?";

#[test]
fn given_a_reason_linked_in_one_process_when_a_new_process_asks_why_then_it_walks_to_the_reason() {
    let root = tempfile::tempdir().expect("root");
    let store = root.path().join("store");

    // Given — session 1 stores the decision, its reason and the link, then ends.
    let mut first = spawn(&store, root.path());
    let first_pid = first.pid();
    let reason = id_of(&first.call("remember", json!({"fact": REASON})));
    let decision = id_of(&first.call("remember", json!({"fact": DECISION})));
    first.call(
        "relate",
        json!({"from": decision, "to": reason, "relation": "because"}),
    );
    first.shutdown();

    // When — a fresh OS process opens the same store and asks why.
    let mut second = spawn(&store, root.path());
    assert_ne!(second.pid(), first_pid, "must be a second OS process");
    let explanation = second.call("why", json!({"decision": QUESTION}));
    second.shutdown();

    // Then — the seed is the decision, and the reason is one persisted hop away.
    let nodes = explanation["nodes"].as_array().expect("nodes");
    let seed = nodes.first().expect("a seed node");
    assert_eq!(seed["content"], DECISION, "seed: {explanation}");
    assert_eq!(seed["hop"], 0);
    let reached = node_with_content(nodes, REASON)
        .unwrap_or_else(|| panic!("reason not reached across restart: {explanation}"));
    assert_eq!(
        reached["hop"], 1,
        "reason must be one link away: {explanation}"
    );
    assert_eq!(reached["id_str"], reason.as_str());
    assert!(
        !explanation["edges"].as_array().expect("edges").is_empty(),
        "the typed link must survive the restart: {explanation}"
    );
}

#[test]
fn given_a_closed_session_when_a_new_process_opens_the_store_then_the_lock_was_released() {
    let root = tempfile::tempdir().expect("root");
    let store = root.path().join("store");

    let mut first = spawn(&store, root.path());
    first.call("remember", json!({"fact": DECISION}));
    first.shutdown();

    // A lock leaked by the first process would fail this `initialize` with
    // `DatabaseLocked` instead of serving the stored fact.
    let mut second = spawn(&store, root.path());
    let recalled = second.call("recall", json!({"query": DECISION}));
    second.shutdown();

    let contents: Vec<&str> = recalled["memories"]
        .as_array()
        .expect("memories")
        .iter()
        .filter_map(|hit| hit["content"].as_str())
        .collect();
    assert!(
        contents.contains(&DECISION),
        "recall after restart: {recalled}"
    );
}

fn spawn(store: &std::path::Path, home: &std::path::Path) -> McpStdioProcess {
    McpStdioProcess::spawn(
        McpStdioProcess::command(store, home),
        "restart-persistence-process",
    )
}

fn id_of(remembered: &Value) -> String {
    remembered["id_str"]
        .as_str()
        .unwrap_or_else(|| panic!("remember returned no id_str: {remembered}"))
        .to_owned()
}

fn node_with_content<'a>(nodes: &'a [Value], content: &str) -> Option<&'a Value> {
    nodes.iter().find(|node| node["content"] == content)
}
