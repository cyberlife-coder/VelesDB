//! Every MCP tool must be reachable from every language binding — or its
//! absence must be a DECLARED decision, with a reason.
//!
//! Why this file exists: the `entity` tool shipped on the MCP server and was
//! silently missing from all three bindings (`velesdb-node`,
//! `velesdb-python`, `velesdb-wasm`) for its entire life. Nobody noticed
//! because nothing ever compared the surfaces. Two other absences
//! (`feedback`, `remember_extracted` on WASM) were deliberate and documented
//! in the WASM module header — but a reader had no way to tell a documented
//! decision from an oversight, since both looked identical from the outside:
//! a method that isn't there.
//!
//! The invariant is therefore not "every binding implements every tool" — it
//! is "every (tool, binding) pair is either implemented or explicitly
//! exempted with a reason". A new MCP tool added without touching the
//! bindings turns this test red on the spot.
//!
//! ## Where the truth comes from
//!
//! The tool list is read from the REAL [`McpServer`] over an in-memory
//! duplex (the idiom of `mcp_schema_bdd.rs`), so it can never drift from
//! what a harness actually sees. The binding surfaces are read from their
//! SOURCE files rather than from a hand-maintained manifest, deliberately: a
//! manifest is a second copy of the truth and can lie (someone adds a tool,
//! updates the manifest, and forgets the code — and the test stays green on
//! a promise nobody kept). Source is the only copy that cannot.
//!
//! Reading source is precise here because each binding publishes its whole
//! surface from exactly ONE `impl` block whose closing brace sits at column
//! 0 — so the scan is bounded to that block, never to a test module or a
//! private helper elsewhere in the file. Method names are the tool names,
//! verbatim, in all three bindings (JS/`camelCase` renaming happens in the
//! `#[napi]` / `#[wasm_bindgen]` attribute, not in the Rust identifier).

#![cfg(all(feature = "mcp", feature = "context", feature = "persistence"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use tempfile::TempDir;
use velesdb_memory::mcp::McpServer;
use velesdb_memory::{DynEmbedder, HashEmbedder, MemoryService, DEFAULT_DIMENSION};

/// One language binding of `velesdb-memory`, and where its published surface
/// lives.
struct Binding {
    /// Crate name, as it appears in a failure message.
    name: &'static str,
    /// Source file holding the published surface, relative to the workspace root.
    path: &'static str,
    /// The `impl` line that opens that surface. Everything up to the next
    /// column-0 `}` is the published block; nothing outside it counts.
    surface_impl: &'static str,
}

const BINDINGS: &[Binding] = &[
    Binding {
        name: "velesdb-node",
        path: "crates/velesdb-node/src/lib.rs",
        surface_impl: "impl MemoryStore {",
    },
    Binding {
        name: "velesdb-python",
        path: "crates/velesdb-python/src/agent_memory_service.rs",
        surface_impl: "impl PyMemoryService {",
    },
    Binding {
        name: "velesdb-wasm",
        path: "crates/velesdb-wasm/src/memory_service.rs",
        surface_impl: "impl WasmMemoryService {",
    },
];

/// A (tool, binding) pair that is deliberately NOT implemented.
///
/// An entry here is a decision on the record, not a to-do: it says the
/// absence was weighed and kept. The reason is mandatory and is printed
/// back by the stale-exemption check, so an entry that stops being true
/// cannot quietly survive.
struct Exemption {
    binding: &'static str,
    tool: &'static str,
    reason: &'static str,
}

const EXEMPTIONS: &[Exemption] = &[
    Exemption {
        binding: "velesdb-wasm",
        tool: "feedback",
        reason: "a durable learned confidence is meaningless on the in-memory WASM backend: \
                 MemoryService::feedback lives in the `persistence`-gated `reinforce` module and \
                 is not compiled for wasm32 at all — exposing it would mean pulling \
                 NativeStore/filesystem code into the very bundle this binding exists to avoid",
    },
    Exemption {
        binding: "velesdb-wasm",
        tool: "remember_extracted",
        reason: "extraction needs a generative model (OllamaExtractor is the crate's only \
                 Extractor impl), i.e. a network dependency in the WASM bundle by default; a \
                 JS-provided extractor callback is the natural v2 addition",
    },
];

/// Boot the real `McpServer` over an in-memory duplex pipe and complete the
/// MCP handshake — same fixture as `mcp_schema_bdd.rs`, so both files read
/// the surface a real harness is served.
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

/// The workspace root: this crate is `<root>/crates/velesdb-memory`.
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .expect("velesdb-memory sits two levels under the workspace root")
        .to_path_buf()
}

/// The body of the ONE `impl` block that publishes `binding`'s surface.
///
/// Bounded by the block's own column-0 closing brace, which `rustfmt`
/// guarantees — so a `fn` in a nested test module or a free helper further
/// down the file is never mistaken for a published method.
fn surface_block(binding: &Binding) -> String {
    let path = workspace_root().join(binding.path);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {} ({}): {err}", binding.path, path.display()));
    let mut lines = source
        .lines()
        .skip_while(|l| l.trim() != binding.surface_impl);
    assert!(
        lines.next().is_some(),
        "{} no longer contains `{}` — the parity guard reads that block to know what the \
         binding publishes; point `Binding::surface_impl` at the renamed block",
        binding.path,
        binding.surface_impl,
    );
    lines
        .take_while(|l| *l != "}")
        .collect::<Vec<_>>()
        .join("\n")
}

/// Method names published by `binding`, taken from its surface block.
fn published_methods(binding: &Binding) -> BTreeSet<String> {
    let block = surface_block(binding);
    let methods: BTreeSet<String> = block
        .lines()
        .filter_map(|line| method_name(line.trim()))
        .collect();
    assert!(
        methods.contains("remember"),
        "{}: the surface block parsed to {} method(s) and none of them is `remember` — the \
         scan is broken, not the binding",
        binding.name,
        methods.len(),
    );
    methods
}

/// The identifier of a method declaration line, if the line is one.
fn method_name(trimmed: &str) -> Option<String> {
    let rest = trimmed
        .strip_prefix("pub fn ")
        .or_else(|| trimmed.strip_prefix("fn "))?;
    let (name, _) = rest.split_once('(')?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// The exemption covering `(binding, tool)`, if one is declared.
fn exemption_for(binding: &str, tool: &str) -> Option<&'static Exemption> {
    EXEMPTIONS
        .iter()
        .find(|e| e.binding == binding && e.tool == tool)
}

/// THE guard: no MCP tool may reach a binding without either an
/// implementation or a declared, motivated exemption.
#[tokio::test]
async fn every_mcp_tool_is_implemented_or_exempted_in_every_binding() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    assert!(!tools.is_empty(), "the server advertises at least one tool");

    let mut gaps: Vec<String> = Vec::new();
    for binding in BINDINGS {
        let methods = published_methods(binding);
        for tool in &tools {
            let name = tool.name.as_ref();
            if methods.contains(name) || exemption_for(binding.name, name).is_some() {
                continue;
            }
            gaps.push(format!("  {name} is missing from {}", binding.name));
        }
    }

    assert!(
        gaps.is_empty(),
        "{} MCP tool(s) unreachable from a binding:\n{}\n\nEvery MCP tool must be either \
         implemented in the binding (relay `MemoryService`, add no logic — follow the idiom of \
         a neighbouring tool such as `why`) or declared in `EXEMPTIONS` in this file WITH the \
         reason it does not apply there. Silence is not a decision: the `entity` tool was \
         invisible from all three bindings for its whole life precisely because nothing \
         compared the surfaces.",
        gaps.len(),
        gaps.join("\n"),
    );
    client.cancel().await.expect("close the MCP session");
}

/// An exemption that stopped being true must be deleted, not left to rot:
/// a stale entry is a hole in the guard that looks like a decision.
#[tokio::test]
async fn no_exemption_is_stale() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let advertised: BTreeSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    let mut stale: Vec<String> = Vec::new();
    for exemption in EXEMPTIONS {
        let Some(binding) = BINDINGS.iter().find(|b| b.name == exemption.binding) else {
            stale.push(format!(
                "  unknown binding `{}` (exempting `{}`)",
                exemption.binding, exemption.tool
            ));
            continue;
        };
        if !advertised.contains(exemption.tool) {
            stale.push(format!(
                "  `{}` is no longer an MCP tool (exempted on {}: {})",
                exemption.tool, exemption.binding, exemption.reason
            ));
        } else if published_methods(binding).contains(exemption.tool) {
            stale.push(format!(
                "  {} now implements `{}` — drop the exemption (it claimed: {})",
                exemption.binding, exemption.tool, exemption.reason
            ));
        }
    }

    assert!(
        stale.is_empty(),
        "{} stale exemption(s) in EXEMPTIONS:\n{}",
        stale.len(),
        stale.join("\n"),
    );
    client.cancel().await.expect("close the MCP session");
}
