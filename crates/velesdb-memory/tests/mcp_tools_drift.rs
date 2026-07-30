//! The tool surface this server actually puts on the wire, materialised as a
//! versioned artifact.
//!
//! ## Why this file exists
//!
//! Four times now, a schema defect has been diagnosed by reading the schema as
//! a CLIENT HARNESS rendered it, rather than as the server emitted it — and
//! twice that rendering lied. `mcp_schema_bdd.rs` carries the retraction of one
//! such mistake in its own comments: a `required` list that looked truncated
//! was complete on the wire, and a campaign was built on the rendering anyway.
//!
//! The root cause is not any single schema bug. It is that the wire had no
//! representation anyone could point at: the truth lived only inside a test
//! run, so every campaign had to re-derive it, and could re-derive it wrongly.
//!
//! This file gives the wire a body. It boots the real [`McpServer`], speaks
//! raw newline-delimited JSON-RPC to it — no client library between the bytes
//! and the assertion, deliberately, since a client library is exactly the kind
//! of intermediary that produced the wrong diagnoses — and commits the answer
//! to `docs/reference/mcp-tools.json`. From here on, "what does the server
//! advertise?" is answered by reading a file in the repository, and any change
//! to it shows up in a diff during review.
//!
//! This is the same shape as the `openapi-drift` job for the REST surface
//! (`.github/workflows/ci.yml`): regenerate from the code, compare with what is
//! committed, fail on drift.
//!
//! ## Why no `#[ignore]`
//!
//! `generate_openapi_spec_files` must be `#[ignore]`d because the workspace
//! test sweep runs it under a feature set that legitimately changes the
//! schema. The equivalent hazard here is real but currently empty, and the
//! distinction is worth writing down rather than rounding off.
//!
//! The `cfg` below is a CONJUNCTION, not an equality: every SUPERSET of
//! `mcp` + `context` + `persistence` satisfies it too — so "under any other
//! feature set this file compiles to nothing" would be false. One such
//! superset already runs in CI: the `lint` job's "Test the velesdb-memory
//! extract feature" step (`.github/workflows/ci.yml`) invokes
//! `cargo test -p velesdb-memory --features extract,ollama,persistence`
//! WITHOUT `--no-default-features`, so this file compiles and executes there
//! as well. Verified by running it, not assumed: it passes, because the
//! twenty tools are registered unconditionally — `remember_extracted` is
//! always advertised and merely answers "no extractor configured" until one
//! is attached, so neither `extract` nor `ollama` adds or removes a tool.
//!
//! The trap is therefore conditional, not present: the day a tool (or a DTO
//! field reachable from a schema) sits behind a non-default feature, this
//! comparison becomes feature-dependent, an artifact regenerated with the
//! `REGENERATE` command below will not match what the `lint` job sees, and
//! the answer is to keep the surface feature-invariant — not to regenerate
//! under one feature set and break the other.
//!
//! It runs inside the required `Tests` job, which builds this crate with its
//! default features (`mcp`, `context`, `persistence`).

#![cfg(all(feature = "mcp", feature = "context", feature = "persistence"))]

use std::path::{Path, PathBuf};

use rmcp::service::ServiceExt;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// Where the captured surface lives, relative to the workspace root. It sits
/// under `docs/reference/` rather than in a test fixture directory on purpose:
/// it is the contract other surfaces (bindings, SDK, guides) are checked
/// against, not a private detail of this test.
const ARTIFACT: &str = "docs/reference/mcp-tools.json";

/// Printed verbatim on failure. A drift message that does not say how to
/// resolve itself sends the reader hunting through CI config.
const REGENERATE: &str =
    "UPDATE_MCP_TOOLS_SNAPSHOT=1 cargo test -p velesdb-memory --test mcp_tools_drift";

/// At most this many differing paths are reported. The point of the message is
/// to name the change, not to reprint the artifact.
const MAX_REPORTED_DIFFERENCES: usize = 40;

/// Values longer than this are elided in the failure message.
const MAX_VALUE_WIDTH: usize = 120;

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test: CARGO_MANIFEST_DIR has a parent (crates/)")
        .parent()
        .expect("test: crates/ has a parent (workspace root)")
        .join(ARTIFACT)
}

fn regeneration_requested() -> bool {
    std::env::var_os("UPDATE_MCP_TOOLS_SNAPSHOT").is_some()
}

async fn send<W>(writer: &mut W, frame: &Value)
where
    W: AsyncWrite + Unpin,
{
    let mut line = serde_json::to_string(frame).expect("serialize a JSON-RPC frame");
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .expect("write a JSON-RPC frame to the duplex");
    writer.flush().await.expect("flush the duplex");
}

async fn receive<R>(lines: &mut Lines<BufReader<R>>) -> Value
where
    R: tokio::io::AsyncRead + Unpin,
{
    let line = lines
        .next_line()
        .await
        .expect("read a line from the duplex")
        .expect("the server closed the duplex before answering");
    serde_json::from_str(&line).expect("the server wrote a line that is not JSON")
}

/// Drive a real server through the handshake and return its `tools/list`
/// result, with the tool array sorted by name so the artifact does not depend
/// on registration order.
async fn capture_tool_surface() -> Value {
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

    let (reader, mut writer) = tokio::io::split(client_side);
    let mut lines = BufReader::new(reader).lines();

    send(
        &mut writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "mcp-tools-drift", "version": "1"},
            },
        }),
    )
    .await;
    let handshake = receive(&mut lines).await;
    assert!(
        handshake.get("result").is_some(),
        "the server refused the handshake: {handshake}"
    );

    send(
        &mut writer,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    send(
        &mut writer,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;

    let response = receive(&mut lines).await;
    let mut tools = response
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("tools/list did not return a tool array: {response}"))
        .clone();
    tools.sort_by_key(tool_name);

    let mut surface = Map::new();
    surface.insert("tools".to_string(), Value::Array(tools));
    Value::Object(surface)
}

fn tool_name(tool: &Value) -> String {
    tool.get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn elide(value: &Value) -> String {
    let rendered = value.to_string();
    if rendered.chars().count() <= MAX_VALUE_WIDTH {
        return rendered;
    }
    let head: String = rendered.chars().take(MAX_VALUE_WIDTH).collect();
    format!("{head}…")
}

/// Walk both trees together and record every path where they disagree, so a
/// failure names the slot that moved instead of printing two blobs.
fn collect_differences(committed: &Value, current: &Value, path: &str, out: &mut Vec<String>) {
    if out.len() >= MAX_REPORTED_DIFFERENCES || committed == current {
        return;
    }
    match (committed, current) {
        (Value::Object(was), Value::Object(now)) => {
            let mut keys: Vec<&String> = was.keys().chain(now.keys()).collect();
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                descend(was.get(key), now.get(key), &format!("{path}/{key}"), out);
            }
        }
        (Value::Array(was), Value::Array(now)) => {
            for index in 0..was.len().max(now.len()) {
                descend(
                    was.get(index),
                    now.get(index),
                    &format!("{path}/{index}"),
                    out,
                );
            }
        }
        _ => out.push(format!(
            "{path}: committed {} → now {}",
            elide(committed),
            elide(current)
        )),
    }
}

fn descend(committed: Option<&Value>, current: Option<&Value>, path: &str, out: &mut Vec<String>) {
    match (committed, current) {
        (Some(was), Some(now)) => collect_differences(was, now, path, out),
        (Some(was), None) => out.push(format!("{path}: REMOVED (was {})", elide(was))),
        (None, Some(now)) => out.push(format!("{path}: ADDED ({})", elide(now))),
        (None, None) => {}
    }
}

#[tokio::test]
async fn the_published_tool_surface_matches_the_committed_artifact() {
    let surface = capture_tool_surface().await;
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&surface).expect("serialize the captured tool surface")
    );
    let path = artifact_path();

    if regeneration_requested() {
        std::fs::create_dir_all(path.parent().expect("the artifact path has a parent"))
            .expect("create docs/reference/");
        std::fs::write(&path, &rendered).expect("write the tool-surface artifact");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("cannot read {ARTIFACT} ({error}) — create it with:\n  {REGENERATE}")
    });
    if committed == rendered {
        return;
    }

    let committed_json: Value = serde_json::from_str(&committed).unwrap_or_else(|error| {
        panic!("{ARTIFACT} is not valid JSON ({error}) — regenerate it with:\n  {REGENERATE}")
    });
    let mut differences = Vec::new();
    collect_differences(&committed_json, &surface, "", &mut differences);
    if differences.is_empty() {
        differences.push("(only formatting differs)".to_string());
    }

    panic!(
        "the advertised MCP tool surface no longer matches {ARTIFACT}.\n\
         This is the wire, not a client's rendering of it: if the change below is\n\
         intended, regenerate the artifact so it lands in the diff for review.\n\n\
         {}\n\n  {REGENERATE}",
        differences.join("\n")
    );
}
