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
//! The `remember_extracted` half of that pair is gone: its reason named
//! `OllamaExtractor` as the crate's only `Extractor`, and a deterministic,
//! dependency-free one annulled it (issue #1692). Worth recording, because
//! the exemption check never read that prose — it only asks whether the
//! binding started publishing the tool. An exemption whose REASON has become
//! false can therefore sit green indefinitely; only a reader catches it.
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
//!
//! ## The second invariant: return SHAPE, not just presence
//!
//! Name parity alone was blind by construction, and that blindness had a
//! cost. `load_working_context` served a three-field envelope (`found`,
//! `working`, `other_sessions`) from the MCP server since V2a-1 while all
//! three bindings kept handing back a bare `WorkingContext | null` — so a
//! caller going through a binding could not tell "nothing was ever saved"
//! from "a typo in `session` missed a session that exists". This file was
//! green the whole time: the method WAS there, under the right name, and
//! nothing here ever looked at what it returned.
//!
//! So the second guard reads each tool's `output_schema` ROOT KEYS from the
//! live server and requires every one of them to reach the binding. A field
//! the server publishes and no binding ever names is exactly the defect
//! above. Three ways to satisfy it, in the order they are tried:
//!
//! 1. **The binding names the server's own output type** (read out of the
//!    server source's `output_schema = wire_safe_output_schema::<T>()`).
//!    That is the complete answer: relaying `T` itself cannot lose a field,
//!    today or when `T` grows one. Where a binding relays wholesale, the
//!    type is written down as an annotation for exactly this reason —
//!    `let compiled: CompiledContext = …` in the WASM binding is not
//!    decoration, it is the relay made checkable.
//! 2. **The field is named in the method region**, or in the body of a
//!    binding-local struct that region names (ONE hop — the Node idiom of a
//!    mirror DTO, `CompiledContextJs`). The hop is what turns "Node never
//!    names `sections`" into the one statement that is true: Node never
//!    relays `warnings`.
//! 3. **A declared [`ShapeDivergence`]**, with its reason.
//!
//! ### What this guard does NOT prove — read this before trusting it
//!
//! **It is a text search over source, nothing more.** The region it scans is
//! the whole method region: doc comment, attributes (`#[napi(ts_return_type
//! = ...)]`, `#[wasm_bindgen]`, `#[pyo3]`) and body. A field named ONLY in a
//! prose comment therefore PASSES — as does a field named in a
//! `ts_return_type` string while the body returns something else entirely,
//! or a type annotation naming `T` on a value that is then reshaped by hand.
//!
//! It cannot be otherwise here: the bindings serialize whole values in one
//! `serde` call, so the field names legitimately never appear as identifiers
//! in their bodies. Requiring them in the body would force a hand-written
//! field-by-field copy — more code, more places to diverge — to satisfy a
//! test.
//!
//! Consequence, stated plainly: **this guard proves DECLARATION, never
//! MARSHALLING.** That a binding actually emits the field with the right
//! value is proved by the bindings' own round-trip tests
//! (`crates/velesdb-node/__test__/index.spec.mjs`,
//! `crates/velesdb-python/tests/test_context_compiler.py`,
//! `crates/velesdb-wasm/tests/memory_wedge_web.rs`), which call the real
//! binding and read the real result. What this file adds is the thing none
//! of those can do: notice that the SERVER grew a field nobody relayed.
//!
//! ### `SHAPE_DIVERGENCES` holds two different things, on purpose
//!
//! Most entries are deliberate unwraps (`recall` → a bare array, `forget` →
//! a bare bool, an id twin collapsing to one form). A few carried
//! [`KNOWN_GAP`]: fields a binding really does lose. Writing a real gap down
//! is not blessing it — it is the only way the guard can be green on the 19
//! other tools while the gap stays visible in one place instead of being
//! rediscovered by a user. An entry is deleted by the fix, never renewed.
//!
//! As of issues #1690/#1691/#1692 the second kind is EMPTY: every entry left
//! is a deliberate unwrap. That is the state this list is meant to reach, so
//! [`KNOWN_GAP`] now sits unused on purpose — see its own doc comment. Note
//! what it does NOT mean: nothing here caps how many gaps may be declared,
//! and adding a seventh would have been exactly as green as fixing six. The
//! pressure to close them was never mechanical.

#![cfg(all(feature = "mcp", feature = "context", feature = "persistence"))]

use std::collections::{BTreeMap, BTreeSet};
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
        binding: "velesdb-node",
        tool: "memory_status",
        reason: "memory_status reports SERVER-RUNTIME state a binding host already holds in its \
                 own hands: the embedder identity is whatever the host constructed (there is no \
                 env-var resolution to reveal), and the daemon's provenance record lives with the \
                 daemon's store. The counts (facts/edges) would be genuinely useful here — a \
                 language-level stats accessor is its own change, tracked with the reembed work",
    },
    Exemption {
        binding: "velesdb-python",
        tool: "memory_status",
        reason: "same as velesdb-node: the host constructs its embedder explicitly, so the \
                 status tool's central answer (which embedder actually runs, is recall semantic) \
                 is the caller's own constructor argument; a stats accessor for the counts is \
                 tracked as its own change",
    },
    Exemption {
        binding: "velesdb-node",
        tool: "list_memories",
        reason: "the audit walk pages through MemoryStore::list, which only the native \
                 store implements today; the binding surface for it rides the same planned \
                 change as the stats accessor - one language-level inspection API",
    },
    Exemption {
        binding: "velesdb-python",
        tool: "list_memories",
        reason: "same as velesdb-node: one planned language-level inspection API covers \
                 listing and stats together",
    },
    Exemption {
        binding: "velesdb-wasm",
        tool: "list_memories",
        reason: "the WASM backend keeps MemoryStore::list default refusal - its in-memory \
                 store lives and dies with the page, so what-does-my-agent-know is answered \
                 by the caller own state, not by an audit walk",
    },
    Exemption {
        binding: "velesdb-wasm",
        tool: "memory_status",
        reason: "the WASM backend is in-memory and persistence-free: no provenance record, no \
                 store directory, no autograph worker — every block of the status except the \
                 fact count is structurally absent from that build",
    },
];

/// One `output_schema` root field a binding deliberately does NOT relay.
///
/// Twin of [`Exemption`], for the second invariant. Same contract: an entry
/// is a decision on the record, the reason is mandatory, and
/// [`no_shape_divergence_is_stale`] deletes it the moment it stops being
/// true. Unwrapping an envelope down to its one useful member is a
/// legitimate binding choice — LOSING a member is not, and only a declared
/// list can tell the two apart.
struct ShapeDivergence {
    binding: &'static str,
    tool: &'static str,
    field: &'static str,
    reason: &'static str,
}

/// The `<id>` / `<id>_str` twin pair collapses in a typed binding.
const ID_TWIN: &str = "deliberate unwrap of the id twin: the MCP wire carries an id BOTH as a \
     JSON number and as its decimal-string copy, because a u64 above 2^53 is lossy on a \
     float-lossy JSON client. A typed binding return has no such problem — it hands back one \
     form (a decimal string on JS, a native int on Python) and the twin has nothing to add";

/// A single-member envelope exists for the MCP transport, not for the domain.
const SINGLE_MEMBER: &str = "deliberate unwrap: the envelope carries exactly one useful member \
     and exists only because the MCP spec requires an object at the output-schema root — a \
     constraint of the transport, not of the domain. Nothing is lost";

/// `forget` answers with the one bit its envelope carries.
const FORGET_BOOL: &str = "deliberate unwrap: the binding returns the bare boolean, which IS \
     this envelope's `found` — whether a memory existed under that id and was deleted. The \
     echoed id adds nothing to a call the caller made with that id in hand";

/// The dated half of fused recall is a SEPARATE binding method.
const DATED_SPLIT: &str = "deliberate split, not a drop: the dated half of fused recall is a \
     SECOND binding method (`recallFusedDated`), which returns the timeline and the clock. \
     This method is the undated one, and returns the bare memories array";

/// A field the server publishes and this binding really does lose. NOT a
/// deliberate unwrap — an entry here is an admission, kept honest by
/// [`no_shape_divergence_is_stale`], and it must be deleted by the fix, not
/// renewed.
///
/// **Currently unused, and that is the point.** The six entries that carried
/// it were deleted by their fixes (issues #1690, #1691, #1692): every field
/// the server publishes now reaches every binding, or is a declared,
/// motivated unwrap. The constant stays so the next honest admission has this
/// exact wording to reach for — deleting it would leave the next author to
/// invent a looser reason of their own, which is how a gap stops being
/// visible.
#[allow(
    dead_code,
    reason = "zero known gaps is the target state, not a reason to drop the word"
)]
const KNOWN_GAP: &str = "KNOWN GAP — NOT a deliberate unwrap: this binding really does lose \
     the field, and the loss predates the guard that found it. Declared so the guard can be \
     green on everything else while the gap stays visible in one place; delete this entry \
     with the fix, never renew it";

const SHAPE_DIVERGENCES: &[ShapeDivergence] = &[
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "entity",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "feedback",
        field: "id_str",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "forget",
        field: "found",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "forget",
        field: "id_str",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "recall_fused",
        field: "dated_context",
        reason: DATED_SPLIT,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "recall_fused",
        field: "memories",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "recall_where",
        field: "memories",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "relate",
        field: "edge_id",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "relate",
        field: "edge_id_str",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "remember",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "remember_extracted",
        field: "ids_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-node",
        tool: "save_working_context",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "entity",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "feedback",
        field: "id_str",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "forget",
        field: "found",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "forget",
        field: "id_str",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "recall_where",
        field: "memories",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "relate",
        field: "edge_id",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "relate",
        field: "edge_id_str",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "remember",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "remember_extracted",
        field: "ids_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-python",
        tool: "save_working_context",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "entity",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "remember_extracted",
        field: "ids_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "forget",
        field: "found",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "forget",
        field: "id_str",
        reason: FORGET_BOOL,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "recall_fused",
        field: "dated_context",
        reason: DATED_SPLIT,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "recall_fused",
        field: "memories",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "recall_fused",
        field: "now",
        reason: DATED_SPLIT,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "recall_where",
        field: "memories",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "relate",
        field: "edge_id",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "relate",
        field: "edge_id_str",
        reason: SINGLE_MEMBER,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "remember",
        field: "id_str",
        reason: ID_TWIN,
    },
    ShapeDivergence {
        binding: "velesdb-wasm",
        tool: "save_working_context",
        field: "id_str",
        reason: ID_TWIN,
    },
];

/// The shape divergence covering `(binding, tool, field)`, if one is declared.
fn shape_divergence_for(
    binding: &str,
    tool: &str,
    field: &str,
) -> Option<&'static ShapeDivergence> {
    SHAPE_DIVERGENCES
        .iter()
        .find(|d| d.binding == binding && d.tool == tool && d.field == field)
}

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

// --- The one published surface that is not Rust ------------------------------
//
// The TypeScript SDK wraps the WASM binding and shipped 17 of its methods for
// its whole life, missing `entity` and `unrelate` (issue #1721). Nothing saw
// it: `BINDINGS` holds three Rust crates and the reader above is a Rust
// parser — `surface_impl` looks for `impl X {`, `method_name` for `pub fn`,
// and `published_methods` asserts its way out of a scan that finds nothing.
// Handing it a `.ts` file would PANIC, not report, so "just add a fourth
// BINDINGS entry" is not available.
//
// What IS available is the chain. The SDK cannot expose a tool the WASM
// binding does not — it calls into it. So the invariant is not "the SDK
// implements every MCP tool" (that would duplicate wasm's `feedback`
// exemption into a second list, and a duplicated decision is a decision that
// will drift); it is "the SDK relays every tool that REACHES the WASM
// binding". Composed with the guard above, which holds wasm against the live
// tool list, the two links prove the whole path from server to SDK.

/// Where the TypeScript SDK declares what it publishes.
///
/// Two blocks in one file, and BOTH matter: `ensureCapability` is typed
/// `keyof WasmMemoryServiceInstance`, so a method added to the class without
/// the interface does not compile, and one added to the interface alone is a
/// promise no caller can reach.
const SDK_SOURCE: &str = "sdks/typescript/src/memory.ts";
const SDK_CLASS: &str = "export class MemoryService {";
const SDK_INTERFACE: &str = "interface WasmMemoryServiceInstance {";

/// The identifier of a TypeScript method declaration, if the line is one.
///
/// Deliberately shallow: exactly one indent level (so a call inside a body
/// never counts) and not `private`. It over-collects — `init`, `close` and
/// `constructor` are not tools — which is harmless, because every caller
/// below iterates over TOOL names and asks whether the SDK has them, never
/// the reverse.
fn typescript_method(line: &str) -> Option<&str> {
    let declaration = line.strip_prefix("  ")?;
    if declaration.starts_with(' ') || declaration.starts_with("private ") {
        return None;
    }
    let (name, _) = declaration.split_once('(')?;
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

/// The methods declared between `opening` and the next column-0 `}`.
fn sdk_methods(opening: &str) -> BTreeSet<String> {
    let path = workspace_root().join(SDK_SOURCE);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {SDK_SOURCE} ({}): {err}", path.display()));
    let mut lines = source.lines().skip_while(|l| l.trim() != opening);
    assert!(
        lines.next().is_some(),
        "{SDK_SOURCE} no longer contains `{opening}` — the SDK parity check reads that block to \
         know what the SDK publishes; point the constant at the renamed block",
    );
    let methods: BTreeSet<String> = lines
        .take_while(|l| *l != "}")
        .filter_map(typescript_method)
        .map(str::to_owned)
        .collect();
    assert!(
        methods.contains("remember"),
        "{SDK_SOURCE} `{opening}` parsed to {} method(s) and none of them is `remember` — the \
         scan is broken, not the SDK",
        methods.len(),
    );
    methods
}

/// A `snake_case` tool name as every JavaScript surface spells it.
///
/// The Rust bindings name their methods after the tool verbatim and rename in
/// an attribute (`#[napi(js_name = ...)]`, `#[wasm_bindgen(js_name = ...)]`);
/// the SDK is written in the target language, so the `camelCase` happens in
/// the identifier itself and the comparison has to cross that gap.
fn js_name(tool: &str) -> String {
    let mut out = String::with_capacity(tool.len());
    let mut capitalize = false;
    for c in tool.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            out.extend(c.to_uppercase());
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Tools the WASM binding publishes that the SDK never declares, named.
///
/// Pure, so the refusal itself is testable without a server: see
/// [`the_sdk_check_names_the_method_a_stripped_sdk_lost`].
fn sdk_gaps(
    tools: &BTreeSet<String>,
    upstream: &BTreeSet<String>,
    class: &BTreeSet<String>,
    interface: &BTreeSet<String>,
) -> Vec<String> {
    let mut gaps = Vec::new();
    for tool in tools {
        if !upstream.contains(tool) {
            // The tool does not reach the WASM binding either. That is the
            // first guard's business, and wasm's exemption governs both.
            continue;
        }
        let published = js_name(tool);
        let in_class = class.contains(&published);
        let in_interface = interface.contains(&published);
        if in_class && in_interface {
            continue;
        }
        let where_missing = match (in_class, in_interface) {
            (false, false) => "neither the class nor the interface",
            (true, false) => "the class but NOT the interface (`ensureCapability` keys off it)",
            (false, true) => "the interface but NOT the class (no caller can reach it)",
            (true, true) => unreachable!("handled above"),
        };
        gaps.push(format!(
            "  `{tool}` reaches velesdb-wasm and the SDK declares `{published}` in \
             {where_missing}"
        ));
    }
    gaps
}

/// The source region publishing each method of `binding`, keyed by method
/// name: everything from the end of the previous method through this one's
/// closing brace — so the doc comment and the `#[napi]` / `#[wasm_bindgen]`
/// / `#[pyo3]` attributes that carry the declared return type are included,
/// not just the body.
///
/// Bounded by the 4-space-indented `}` that `rustfmt` puts at the end of a
/// method inside an `impl` block.
fn method_regions(binding: &Binding) -> BTreeMap<String, String> {
    let block = surface_block(binding);
    let lines: Vec<&str> = block.lines().collect();
    let mut regions = BTreeMap::new();
    let mut start = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = method_name(line.trim()) else {
            continue;
        };
        if index < start {
            // A nested `fn` inside the method already consumed — a closure
            // or a local helper, not a published method.
            continue;
        }
        let end = lines[index..]
            .iter()
            .position(|l| *l == "    }")
            .map_or(lines.len(), |offset| index + offset + 1);
        regions.insert(name, lines[start..end].join("\n"));
        start = end;
    }
    assert!(
        regions.contains_key("remember"),
        "{}: the region scan produced {} method(s) and none of them is `remember` — the \
         scan is broken, not the binding",
        binding.name,
        regions.len(),
    );
    regions
}

/// `region` with the declared method's PARAMETER LIST cut out — the window
/// route 2 must read.
///
/// A region runs from the end of the previous method to this one's closing
/// brace, so it carries the signature, and the signature carries the
/// parameter NAMES. Route 2 is a text search: an output field called `handle`
/// was therefore satisfied by an INPUT parameter called `handle`, which
/// proves the binding accepts that name, not that it relays it (#1704). The
/// return type sits outside the parentheses and is deliberately kept.
///
/// Falls back to the whole region when the declaration cannot be located —
/// a parser that silently narrows its own window would be the same class of
/// defect one level up.
fn output_window(region: &str, method: &str) -> String {
    cut_parameter_list(&cut_signature_attribute(region), method)
}

/// `region` without the parameter echo of a `#[pyo3(signature = (...))]`
/// attribute.
///
/// The same defect by another road: the attribute sits BEFORE the `fn` line,
/// so cutting the parentheses of the declaration alone leaves every parameter
/// name spelled a second time, and route 2 reads it just as happily.
fn cut_signature_attribute(region: &str) -> String {
    let mut text = region.to_owned();
    while let Some(at) = text.find("signature = (") {
        let open = at + "signature = ".len();
        let Some(close) = matching_paren(&text[open..]).map(|offset| open + offset) else {
            break;
        };
        text.replace_range(open..=close, "");
    }
    text
}

/// `region` with the parameter list of `method`'s declaration cut out.
fn cut_parameter_list(region: &str, method: &str) -> String {
    let mut at = 0usize;
    let mut declaration = None;
    for line in region.split_inclusive('\n') {
        // The same parser that built the regions, not a text search: a doc
        // comment mentioning `fn recall(` must not be taken for the header.
        if method_name(line.trim()).as_deref() == Some(method) {
            declaration = Some(at);
            break;
        }
        at += line.len();
    }
    let Some(header) = declaration else {
        return region.to_owned();
    };
    let Some(open) = region[header..].find('(').map(|offset| header + offset) else {
        return region.to_owned();
    };
    let Some(close) = matching_paren(&region[open..]).map(|offset| open + offset) else {
        return region.to_owned();
    };
    let mut window = region[..open].to_owned();
    window.push_str(&region[close + 1..]);
    window
}

/// `region` with every `///` doc-comment line removed.
///
/// Route 2's window keeps doc comments — deliberately, per the module
/// header's "What this guard does NOT prove": a field named only in prose is
/// an accepted weak pass for "is the field DECLARED". The stale check (below)
/// asks a different question, "did the RENDERED form change", which prose
/// cannot answer either way — so it reads this stricter window instead
/// (#1760: documenting `recall_where`'s contract as "Returns caller memories
/// ONLY" made a still-true [`ShapeDivergence`] for the field `memories` look
/// stale, because the sentence, not the return value, named it).
fn without_doc_comments(region: &str) -> String {
    region
        .lines()
        .filter(|line| !line.trim_start().starts_with("///"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The offset, within `text`, of the `)` closing the `(` that `text` opens
/// with — `None` when it never closes.
///
/// Depth-counted, not `find(')')`: a parameter typed `Wrapped<(u8, u8)>`
/// closes an inner pair first, and cutting there would leave half a
/// parameter list in the window.
fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// The ROOT property names of `tool`'s advertised `output_schema`, empty
/// when the tool advertises none.
fn output_root_fields(tool: &rmcp::model::Tool) -> BTreeSet<String> {
    tool.output_schema
        .as_ref()
        .and_then(|schema| schema.get("properties"))
        .and_then(serde_json::Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

/// The server source files declaring the tools, scanned for the Rust type
/// each tool's `output_schema` is derived from.
const SERVER_TOOL_SOURCES: &[&str] = &[
    "crates/velesdb-memory/src/mcp.rs",
    "crates/velesdb-memory/src/mcp/context_tools.rs",
];

/// Tool name → the Rust type the server publishes as that tool's output
/// shape, read from its `output_schema = wire_safe_output_schema::<T>()`
/// declaration in the server source.
///
/// Why the TYPE and not only the field list: a binding that hands back the
/// server's own type cannot lose a field — not now, and not when the type
/// grows one. Naming the type is therefore a complete answer to "do you
/// relay this shape?", where naming today's fields would only be an answer
/// for today.
fn server_output_types() -> BTreeMap<String, String> {
    let mut types = BTreeMap::new();
    for relative in SERVER_TOOL_SOURCES {
        let path = workspace_root().join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {} ({}): {err}", relative, path.display()));
        collect_output_types(&source, &mut types);
    }
    assert!(
        types.len() >= 20,
        "only {} tool output type(s) parsed out of the server source — the scan is broken, \
         not the server (it publishes 22 tools)",
        types.len(),
    );
    types
}

/// Pair each `name = "<tool>"` line of a server source file with the
/// `wire_safe_output_schema::<T>()` that follows it inside the same `#[tool(
/// ... )]` attribute.
fn collect_output_types(source: &str, types: &mut BTreeMap<String, String>) {
    let mut pending: Option<String> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(tool) = between(trimmed, "name = \"", "\"") {
            pending = Some(tool.to_owned());
        } else if let Some(ty) = between(trimmed, "wire_safe_output_schema::<", ">") {
            if let Some(tool) = pending.take() {
                types.insert(tool, ty.to_owned());
            }
        }
    }
}

/// The text between the first `open` and the next `close` after it.
fn between<'a>(haystack: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let rest = haystack.split_once(open)?.1;
    let (inner, _) = rest.split_once(close)?;
    (!inner.is_empty()).then_some(inner)
}

/// Whether `region` names `needle` as a WHOLE identifier — never as a
/// prefix, a suffix or an inner fragment.
///
/// Whole-word matters both ways here: `CompiledContext` must NOT be
/// satisfied by the Node binding's own mirror DTO `CompiledContextJs`, which
/// is a different type that really does drop a field (`warnings`); and
/// `found` must not be satisfied by the word `founded` in a comment.
fn names_identifier(region: &str, needle: &str) -> bool {
    region.match_indices(needle).any(|(at, _)| {
        !bounded_by_ident_char(&region[..at], true)
            && !bounded_by_ident_char(&region[at + needle.len()..], false)
    })
}

/// Whether `text` touches the match on an identifier character — looking
/// backwards from its end when `before`, forwards from its start otherwise.
fn bounded_by_ident_char(text: &str, before: bool) -> bool {
    let adjacent = if before {
        text.chars().next_back()
    } else {
        text.chars().next()
    };
    adjacent.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Every `struct <Name> { ... }` declared in `binding`'s crate, name → body.
///
/// Used for ONE hop out of the method region: a binding that returns its own
/// mirror DTO (the Node idiom — `CompiledContextJs`, `EntityProfileJs`)
/// declares the relayed shape in that struct, not at the method. Following
/// that hop is what turns "Node never names `sections`" into the one finding
/// that is actually true: Node never relays `warnings`.
fn binding_structs(binding: &Binding) -> BTreeMap<String, String> {
    let crate_src = workspace_root()
        .join(binding.path)
        .parent()
        .expect("a binding surface file lives in the crate's src/")
        .to_path_buf();
    let mut structs = BTreeMap::new();
    for entry in std::fs::read_dir(&crate_src).expect("read the binding crate's src/") {
        let path = entry.expect("read a src/ entry").path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).expect("read a binding source file");
            collect_structs(&source, &mut structs);
        }
    }
    structs
}

/// Pair each `struct X {` line with the lines up to its column-0 closing
/// brace — `rustfmt` guarantees that brace for a top-level item.
fn collect_structs(source: &str, structs: &mut BTreeMap<String, String>) {
    let lines: Vec<&str> = source.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = between(line, "struct ", " {") else {
            continue;
        };
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue; // A generic or `where`-bounded declaration; not a wire DTO.
        }
        let end = lines[index..]
            .iter()
            .position(|l| *l == "}")
            .map_or(lines.len(), |offset| index + offset);
        structs.insert(name.to_owned(), lines[index..end].join("\n"));
    }
}

/// `region`, plus the body of every binding-local struct it names — the one
/// hop described on [`binding_structs`].
fn region_with_named_structs(region: &str, structs: &BTreeMap<String, String>) -> String {
    let mut text = region.to_owned();
    for (name, body) in structs {
        if names_identifier(region, name) {
            text.push('\n');
            text.push_str(body);
        }
    }
    text
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

/// THE second guard: every ROOT field of a tool's advertised
/// `output_schema` must reach the binding — either because the binding
/// names the SERVER'S OWN output type (then it cannot lose a field, today
/// or after the type grows one), or because it names that field, or because
/// the drop is a declared, motivated [`ShapeDivergence`].
///
/// Read the module header's "What this guard does NOT prove" before relying
/// on a green run: this is a text search, and a field named only in a
/// comment satisfies it.
#[tokio::test]
async fn every_output_field_is_relayed_or_divergence_is_declared_in_every_binding() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let server_types = server_output_types();

    let mut gaps: Vec<String> = Vec::new();
    for binding in BINDINGS {
        let regions = method_regions(binding);
        let structs = binding_structs(binding);
        for tool in &tools {
            let name = tool.name.as_ref();
            let Some(region) = regions.get(name) else {
                continue; // Absence is the FIRST guard's business, not this one.
            };
            if server_types
                .get(name)
                .is_some_and(|ty| names_identifier(region, ty))
            {
                continue; // Relays the server's own type: shape-complete by construction.
            }
            // Route 2 reads the region MINUS the parameter list: an input
            // named like an output field is not a relay of it (#1704).
            // Route 1 above keeps the whole region — it matches a type name,
            // and a type named only in the parameters is an input type.
            let window = output_window(region, name);
            let declared = region_with_named_structs(&window, &structs);
            for field in output_root_fields(tool) {
                if names_identifier(&declared, &field)
                    || shape_divergence_for(binding.name, name, &field).is_some()
                {
                    continue;
                }
                gaps.push(format!(
                    "  {}.{field} is not relayed by {}",
                    name, binding.name
                ));
            }
        }
    }

    assert!(
        gaps.is_empty(),
        "{} output field(s) the server publishes but a binding never names:\n{}\n\nEvery \
         root field of a tool's output_schema must reach the binding: name the server's \
         output type, or name the field in the method region (doc comment, attributes or \
         body), or declare the drop in SHAPE_DIVERGENCES in this file WITH its reason. This \
         is the invariant that was missing when `load_working_context` served \
         `{{found, working, other_sessions}}` for months while all three bindings returned a \
         bare `WorkingContext | null`: the name guard was green, and nobody was looking at \
         the shape.",
        gaps.len(),
        gaps.join("\n"),
    );
    client.cancel().await.expect("close the MCP session");
}

/// Tools whose bindings must satisfy the shape guard by ROUTE 1 — naming the
/// server's own output type — and never by the text-search fallback.
///
/// Route 2 is a text search: prose in a doc comment satisfies it, and so does
/// a `ts_return_type` string whose body returns something else. That is an
/// accepted limit for the file at large (see the module header), but it is
/// NOT acceptable for the tool whose silent shape drift is the entire reason
/// this guard exists. `load_working_context` served `{found, working,
/// other_sessions}` for months while all three bindings handed back a bare
/// `WorkingContext | null`; every one of those bindings carried a doc comment
/// describing the tool, so a text search would have been green throughout.
///
/// Route 1 is different in kind: the annotation `let loaded:
/// LoadedWorkingContext = …` does not describe the relay, it IS the relay,
/// and the compiler rejects it the moment the binding stops returning that
/// type. Prose cannot lie its way past a type check.
const RELAY_BY_TYPE_ONLY: &[&str] = &["load_working_context"];

/// Route 1 is applicable — and applied — for every tool in
/// [`RELAY_BY_TYPE_ONLY`], in every binding that implements it.
///
/// Fails when a binding relays the envelope without naming its type, even
/// though the field-by-field fallback would have passed it on the strength of
/// its doc comment alone.
#[tokio::test]
async fn envelope_tools_are_relayed_by_type_never_by_prose_alone() {
    let server_types = server_output_types();

    let mut weak: Vec<String> = Vec::new();
    for binding in BINDINGS {
        let regions = method_regions(binding);
        for tool in RELAY_BY_TYPE_ONLY {
            let Some(region) = regions.get(*tool) else {
                continue; // Absence is the first guard's business, not this one.
            };
            let ty = server_types
                .get(*tool)
                .unwrap_or_else(|| panic!("no server output type parsed for `{tool}`"));
            if !names_identifier(region, ty) {
                weak.push(format!(
                    "  {}.{tool} never names `{ty}` — it relays the envelope without \
                     declaring its type",
                    binding.name
                ));
            }
        }
    }

    assert!(
        weak.is_empty(),
        "{} binding(s) satisfy the shape guard only by text search:\n{}\n\nThese tools must \
         name the server's own output type (route 1), because a doc comment describing the \
         envelope is enough to satisfy route 2 while the body returns the bare form — which \
         is exactly the drift that went unnoticed for months. Bind the value with an explicit \
         annotation, e.g. `let loaded: LoadedWorkingContext = \
         svc.resume_working_context(..)?;`, so the compiler enforces what this test reads.",
        weak.len(),
        weak.join("\n"),
    );
}

/// A shape divergence that stopped being true must be deleted, not left to
/// rot — same reason as [`no_exemption_is_stale`]: a stale entry is a hole
/// in the guard that looks like a decision.
#[tokio::test]
async fn no_shape_divergence_is_stale() {
    let (_store, client) = connected().await;
    let tools = client.list_all_tools().await.expect("list tools");

    let mut stale: Vec<String> = Vec::new();
    for divergence in SHAPE_DIVERGENCES {
        if let Some(reason) = stale_shape_reason(&tools, divergence) {
            stale.push(format!(
                "  {} / {} / {}: {reason} (it claimed: {})",
                divergence.binding, divergence.tool, divergence.field, divergence.reason
            ));
        }
    }

    assert!(
        stale.is_empty(),
        "{} stale entry(ies) in SHAPE_DIVERGENCES:\n{}",
        stale.len(),
        stale.join("\n"),
    );
    client.cancel().await.expect("close the MCP session");
}

/// Why `divergence` no longer describes reality, or `None` when it still
/// does. Split out of [`no_shape_divergence_is_stale`] to keep both under
/// the repository's cyclomatic-complexity gate.
fn stale_shape_reason(tools: &[rmcp::model::Tool], divergence: &ShapeDivergence) -> Option<String> {
    if !BINDINGS.iter().any(|b| b.name == divergence.binding) {
        return Some(format!("unknown binding `{}`", divergence.binding));
    }
    let Some(tool) = tools.iter().find(|t| t.name.as_ref() == divergence.tool) else {
        return Some(format!("`{}` is no longer an MCP tool", divergence.tool));
    };
    if !output_root_fields(tool).contains(divergence.field) {
        return Some(format!(
            "`{}` no longer has a root output field `{}`",
            divergence.tool, divergence.field
        ));
    }
    let binding = BINDINGS.iter().find(|b| b.name == divergence.binding)?;
    let regions = method_regions(binding);
    let region = regions.get(divergence.tool)?;
    let server_type = server_output_types().get(divergence.tool).cloned();
    if server_type
        .as_ref()
        .is_some_and(|ty| names_identifier(region, ty))
    {
        return Some(format!(
            "the binding now relays the server type `{}` wholesale",
            server_type.unwrap_or_default()
        ));
    }
    // Route 2's window, minus doc comments (#1760) — the parameter-list cut
    // is still shared with route 2 for the #1704 reason (a divergence must
    // not go stale because a PARAMETER names the field), but doc comments
    // are stripped ON TOP of that here, not in route 2: see
    // `without_doc_comments`.
    let window = without_doc_comments(&output_window(region, divergence.tool));
    let declared = region_with_named_structs(&window, &binding_structs(binding));
    names_identifier(&declared, divergence.field)
        .then(|| format!("the binding now names `{}`", divergence.field))
}

// ===========================================================================
// The window of #1704, pinned on synthetic text.
//
// Measured on develop and again after the cut: NO binding is green today on
// the strength of a parameter name alone, so the bindings cannot demonstrate
// this refusal. That is a reason to pin it here, not a reason to skip it —
// the channel is open until something closes it, and a guard nobody has seen
// refuse is the defect this whole campaign is about.
// ===========================================================================

/// A region shaped like the real ones: doc comment, `#[pyo3(signature)]`
/// echo, declaration with a nested-paren parameter type, body, closing brace.
const SYNTHETIC_REGION: &str = "\
    /// Fetch the source back.
    #[pyo3(signature = (handle, media = None))]
    fn retrieve_context_source(&self, py: Python<'_>, handle: &str, media: Option<(u8, u8)>) -> PyResult<Py<PyAny>> {
        let source: ContextSource = fetch();
        Ok(source)
    }
";

#[test]
fn the_raw_region_is_satisfied_by_a_parameter_name() {
    // The false green of #1704, stated before it is fixed: `handle` is a root
    // field of `retrieve_context_source`'s output_schema, and the region
    // names it three times without relaying it once.
    assert!(
        names_identifier(SYNTHETIC_REGION, "handle"),
        "fixture precondition: the raw region names `handle`",
    );
}

#[test]
fn the_output_window_refuses_a_field_named_only_by_an_input() {
    let window = output_window(SYNTHETIC_REGION, "retrieve_context_source");
    assert!(
        !names_identifier(&window, "handle"),
        "an input named `handle` still satisfies the window:\n{window}",
    );
    assert!(
        !names_identifier(&window, "media"),
        "the `signature` attribute still spells the parameters:\n{window}",
    );
}

#[test]
fn the_output_window_keeps_everything_that_describes_the_return() {
    let window = output_window(SYNTHETIC_REGION, "retrieve_context_source");
    for kept in ["PyResult", "ContextSource", "Fetch"] {
        assert!(
            names_identifier(&window, kept),
            "the cut swallowed `{kept}`, which describes the return:\n{window}",
        );
    }
}

#[test]
fn the_cut_falls_back_to_the_whole_region_when_the_declaration_is_absent() {
    // A cut that narrowed silently on a parse miss would be the defect one
    // level up: the guard would stop seeing relays it used to see. Only the
    // parameter-list cut can miss — the `signature` echo is cut either way,
    // since finding it does not depend on finding the declaration.
    let missed = cut_parameter_list(SYNTHETIC_REGION, "a_method_that_is_not_here");
    assert_eq!(missed, SYNTHETIC_REGION);
}

#[test]
fn the_cut_stops_at_the_paren_that_closes_the_parameter_list() {
    // `Option<(u8, u8)>` closes an inner pair first; a `find(')')` would cut
    // there and leave `-> PyResult<…>` amputated of everything before it.
    let window = output_window(SYNTHETIC_REGION, "retrieve_context_source");
    assert!(
        window.contains("fn retrieve_context_source -> PyResult<Py<PyAny>> {"),
        "the declaration did not survive the cut intact:\n{window}",
    );
}

// ===========================================================================
// The false stale of #1760, pinned on synthetic text.
//
// Reproduced on develop before the fix: documenting `recall_where`'s
// contract as "Returns caller memories ONLY" in the three bindings' doc
// comments made `no_shape_divergence_is_stale` report their real,
// still-needed `memories` `ShapeDivergence` entries as stale — the sentence
// explaining what the binding does NOT return was read as proof that it now
// does.
// ===========================================================================

/// A region shaped like the real regression: a doc comment that spells a
/// field name in prose, on a method whose body never returns it.
const PROSE_ONLY_REGION: &str = "\
    /// Returns caller memories ONLY: entity hubs and the context compiler's\n\
    /// artefacts are internal scaffolding and never come back through this call.\n\
    fn recall_where(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {\n\
        Ok(vec![])\n\
    }\n\
";

#[test]
fn the_raw_window_is_satisfied_by_prose_alone() {
    // The false green this fixture exists to state before it is fixed:
    // `memories` never appears outside the doc comment, and route 2's window
    // (which keeps doc comments, by design) matches it anyway.
    let window = output_window(PROSE_ONLY_REGION, "recall_where");
    assert!(
        names_identifier(&window, "memories"),
        "fixture precondition: the raw window names `memories` only in prose",
    );
}

#[test]
fn stripping_doc_comments_refuses_a_field_named_only_in_prose() {
    let window = without_doc_comments(&output_window(PROSE_ONLY_REGION, "recall_where"));
    assert!(
        !names_identifier(&window, "memories"),
        "a field named only in a doc comment must not survive the strip:\n{window}",
    );
}

#[test]
fn stripping_doc_comments_keeps_everything_that_describes_the_return() {
    let window = without_doc_comments(&output_window(PROSE_ONLY_REGION, "recall_where"));
    for kept in ["PyResult", "PyObject", "recall_where"] {
        assert!(
            names_identifier(&window, kept),
            "the strip swallowed `{kept}`, which is not a doc comment:\n{window}",
        );
    }
}

// --- The SDK link of the chain (issue #1721) ---------------------------------

/// THE guard for the one surface written in the target language: every tool
/// that reaches the WASM binding must reach the SDK that wraps it.
///
/// The SDK shipped 17 of 19 for its whole life. The reason nothing caught it
/// is the same one that let `entity` go missing from three bindings: the list
/// of what is checked did not include it. Widening a guard's declared
/// perimeter is the fix; writing a second, parallel one would only move the
/// blind spot.
#[tokio::test]
async fn the_typescript_sdk_relays_every_tool_that_reaches_the_wasm_binding() {
    let (_store, client) = connected().await;
    let tools: BTreeSet<String> = client
        .list_all_tools()
        .await
        .expect("list tools")
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    let wasm = BINDINGS
        .iter()
        .find(|b| b.name == "velesdb-wasm")
        .expect("velesdb-wasm is a declared binding");

    let gaps = sdk_gaps(
        &tools,
        &published_methods(wasm),
        &sdk_methods(SDK_CLASS),
        &sdk_methods(SDK_INTERFACE),
    );

    assert!(
        gaps.is_empty(),
        "{} tool(s) reach velesdb-wasm but stop at the TypeScript SDK:\n{}\n\nThe SDK calls \
         into the WASM binding, so it can never publish more than that binding does — but it \
         can publish LESS, silently, and it did: `entity` and `unrelate` were absent for the \
         SDK's whole life while wasm exposed both. Add the method to BOTH the class and the \
         `WasmMemoryServiceInstance` interface in {}.",
        gaps.len(),
        gaps.join("\n"),
        SDK_SOURCE,
    );
    client.cancel().await.expect("close the MCP session");
}

/// The refusal vector issue #1721 asks for, on a synthetic SDK so it can be
/// replayed without editing the real one: a method the wasm binding publishes
/// and the SDK dropped must be REPORTED BY NAME, never merely counted.
#[test]
fn the_sdk_check_names_the_method_a_stripped_sdk_lost() {
    let tools = ["remember", "unrelate", "recall_fused_dated"]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let upstream = tools.clone();
    let stripped = ["remember", "recallFusedDated"]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();

    let gaps = sdk_gaps(&tools, &upstream, &stripped, &stripped);

    assert_eq!(
        gaps.len(),
        1,
        "exactly the dropped tool is reported: {gaps:?}"
    );
    assert!(
        gaps[0].contains("`unrelate`"),
        "the refusal names the missing tool, got: {}",
        gaps[0],
    );
}

/// The positive control the refusal above is worthless without: the SAME
/// inputs, complete, must produce nothing. Without it, a `sdk_gaps` that
/// returned a finding for every tool would pass the test above.
#[test]
fn the_sdk_check_is_silent_when_every_tool_is_relayed() {
    let tools = ["remember", "unrelate", "recall_fused_dated"]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let complete = ["remember", "unrelate", "recallFusedDated"]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();

    assert!(
        sdk_gaps(&tools, &tools, &complete, &complete).is_empty(),
        "a complete SDK must produce no finding",
    );
}

/// A tool the WASM binding does not publish is NOT the SDK's problem: wasm's
/// own exemption governs, and duplicating it into a second list is how two
/// records of one decision start disagreeing.
#[test]
fn the_sdk_check_ignores_a_tool_that_never_reaches_wasm() {
    let tools = ["remember", "feedback"]
        .map(str::to_owned)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let upstream = ["remember"].map(str::to_owned).into_iter().collect();
    let sdk = ["remember"].map(str::to_owned).into_iter().collect();

    assert!(
        sdk_gaps(&tools, &upstream, &sdk, &sdk).is_empty(),
        "`feedback` is exempted on wasm; the SDK must not be asked for it",
    );
}

/// Declaring a method in one of the two blocks and not the other is its own
/// failure, and the report must say WHICH — the two have different
/// consequences (`ensureCapability` keys off the interface; a caller reaches
/// the class).
#[test]
fn the_sdk_check_distinguishes_the_class_from_the_interface() {
    let tools = ["entity"].map(str::to_owned).into_iter().collect();
    let upstream = ["entity"].map(str::to_owned).into_iter().collect();
    let present: BTreeSet<String> = ["entity"].map(str::to_owned).into_iter().collect();
    let absent = BTreeSet::new();

    let gaps = sdk_gaps(&tools, &upstream, &present, &absent);
    assert!(gaps[0].contains("NOT the interface"), "got: {}", gaps[0]);
    let gaps = sdk_gaps(&tools, &upstream, &absent, &present);
    assert!(gaps[0].contains("NOT the class"), "got: {}", gaps[0]);
}

// --- Field parity: the context compiler's fragment ---------------------------
//
// Method parity, above, proves a tool is REACHABLE from the SDK. It says
// nothing about whether the SDK can express the tool's input — and that is a
// second way to publish a promise nobody can use.
//
// Measured on 2026-08-02: `CompileContextFragment` in the TypeScript SDK was
// missing `path` (file ingestion, V2b-1 — advertised by the server and by both
// SKILL.md copies) and `priority` ("higher packs first", the knob that decides
// what survives a token budget). A TypeScript caller could reach
// `compileContext` and still not use half of what it accepts.
//
// Both lists are read from source, never written down here: adding a field to
// the canonical fragment without adding it to the SDK turns this red on the
// spot, which a hard-coded expectation could not do.

/// Where the canonical wire shape of a context fragment is declared.
const FRAGMENT_SOURCE: &str = "crates/velesdb-memory/src/context/model.rs";
const FRAGMENT_STRUCT: &str = "pub struct ContextFragment {";

/// Where the TypeScript SDK declares the same fragment.
const SDK_FRAGMENT: &str = "export interface CompileContextFragment {";

/// Field names of the `struct` opened by `opening`, up to the next column-0 `}`.
fn rust_struct_fields(source_path: &str, opening: &str) -> BTreeSet<String> {
    let path = workspace_root().join(source_path);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {source_path} ({}): {err}", path.display()));
    let mut lines = source.lines().skip_while(|l| l.trim() != opening);
    assert!(
        lines.next().is_some(),
        "{source_path} no longer contains `{opening}` — point the constant at the renamed struct",
    );
    lines
        .take_while(|l| *l != "}")
        .filter_map(|line| {
            let declaration = line.trim().strip_prefix("pub ")?;
            let (name, _) = declaration.split_once(':')?;
            name.chars()
                .all(|c| c.is_alphanumeric() || c == '_')
                .then(|| name.to_owned())
        })
        .collect()
}

/// Field names of the TypeScript interface opened by `opening`.
fn typescript_interface_fields(opening: &str) -> BTreeSet<String> {
    let path = workspace_root().join(SDK_SOURCE);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {SDK_SOURCE} ({}): {err}", path.display()));
    let mut lines = source.lines().skip_while(|l| l.trim() != opening);
    assert!(
        lines.next().is_some(),
        "{SDK_SOURCE} no longer contains `{opening}` — point the constant at the renamed interface",
    );
    lines
        .take_while(|l| *l != "}")
        .filter_map(|line| {
            let declaration = line.trim();
            let (name, _) = declaration.split_once(':')?;
            let name = name.trim_end_matches('?');
            name.chars()
                .all(|c| c.is_alphanumeric() || c == '_')
                .then(|| name.to_owned())
        })
        .collect()
}

#[test]
fn the_fragment_scans_read_something_on_both_sides() {
    // Guard the guard: an empty scan on either side would make the parity
    // assertion below pass (or fail) for reasons that have nothing to do with
    // the SDK.
    let canonical = rust_struct_fields(FRAGMENT_SOURCE, FRAGMENT_STRUCT);
    assert!(
        canonical.contains("content") && canonical.contains("id"),
        "the canonical fragment scan parsed {canonical:?} — the scan is broken, not the struct",
    );
    let sdk = typescript_interface_fields(SDK_FRAGMENT);
    assert!(
        sdk.contains("content") && sdk.contains("id"),
        "the SDK fragment scan parsed {sdk:?} — the scan is broken, not the SDK",
    );
}

/// Fields the SDK deliberately does NOT declare, each with the reason.
///
/// Same shape as this file's tool-level exemptions, and for the same reason: a
/// field that is absent by decision and a field that is absent by oversight
/// look identical from the outside, and only a written reason tells them
/// apart.
const SDK_FRAGMENT_EXEMPTIONS: &[(&str, &str)] = &[(
    "path",
    "resolving a `path` fragment is a server-side I/O pre-pass gated on \
     VELESDB_MEMORY_INGEST_ROOTS, an operator-configured allowlist of \
     directories. This SDK runs on the WASM binding, which has neither a \
     filesystem nor that setting. NO binding declares it — velesdb-node and \
     velesdb-python do not resolve paths either; the MCP daemon is the only \
     surface that can honour one, so declaring it here would be a field that \
     always fails.",
)];

#[test]
fn every_fragment_exemption_names_a_field_that_actually_exists() {
    // An exemption for a field the canonical fragment no longer has would sit
    // here forever, silently excusing nothing — and would hide the day a real
    // field takes that name.
    let canonical = rust_struct_fields(FRAGMENT_SOURCE, FRAGMENT_STRUCT);
    for (field, _) in SDK_FRAGMENT_EXEMPTIONS {
        assert!(
            canonical.contains(*field),
            "`{field}` is exempted from the SDK fragment but is not a field of the canonical \
             fragment any more — delete the exemption",
        );
    }
}

#[test]
fn the_typescript_fragment_declares_every_field_the_wire_accepts() {
    let canonical = rust_struct_fields(FRAGMENT_SOURCE, FRAGMENT_STRUCT);
    let mut sdk = typescript_interface_fields(SDK_FRAGMENT);
    sdk.extend(
        SDK_FRAGMENT_EXEMPTIONS
            .iter()
            .map(|(field, _)| (*field).to_owned()),
    );
    let missing: Vec<&String> = canonical.difference(&sdk).collect();
    assert!(
        missing.is_empty(),
        "the TypeScript SDK's `CompileContextFragment` is missing {missing:?}, so a TypeScript \
         caller cannot express input the server accepts. The tool is reachable and half of its \
         contract is not — declare the field in {SDK_SOURCE}, or, if it is deliberately withheld, \
         say so where a reader will see it rather than leaving the absence to look like an \
         oversight.",
    );
}
