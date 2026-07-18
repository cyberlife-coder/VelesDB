//! Unit tests for the context compiler MCP tools (split out of
//! `context_tools.rs`, same `#[cfg(test)]`-via-`#[path]` pattern as
//! `server_tests.rs`).

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorCode;
use tempfile::TempDir;

use super::super::dto::RememberParams;
use super::*;
use crate::context::{
    fragment_id, ContextAction, ContextFact, ContextFragment, MemoryScope, WorkingContext,
};
use crate::embedder::{DynEmbedder, HashEmbedder};
use crate::service::MemoryService;

fn server() -> (TempDir, McpServer) {
    let dir = TempDir::new().expect("create tempdir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let service = MemoryService::open(dir.path(), embedder).expect("open memory store");
    (dir, McpServer::new(service))
}

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
    }
}

fn request(query: &str, fragments: Vec<ContextFragment>, budget: u64) -> CompileRequest {
    CompileRequest {
        query: query.to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: budget,
        memory_scope: None,
        policy: None,
    }
}

/// `compile_context`/`explain_compilation` now return the wire `Value`
/// directly (so `policy.ids_as_strings` can rewrite it before it leaves the
/// process) — deserialize back into the domain type for tests that assert
/// on typed fields, exactly mirroring what a Rust MCP client would do.
fn compiled_context_of(value: serde_json::Value) -> CompiledContext {
    serde_json::from_value(value).expect("valid CompiledContext wire value")
}

fn decision_of(value: serde_json::Value) -> ContextDecision {
    serde_json::from_value(value).expect("valid ContextDecision wire value")
}

#[tokio::test]
async fn test_compile_context_tool_returns_compiled_context_and_insights() {
    // Given a server and a compile request with a duplicate
    let (_dir, srv) = server();
    let req = request(
        "deploy",
        vec![fragment("a fact"), fragment("a fact")],
        10_000,
    );

    // When calling the compile_context tool
    let Json(value) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");
    let out = compiled_context_of(value);

    // Then the compiled context carries content, decisions, and insights
    assert!(out.content.contains("a fact"));
    assert_eq!(out.decisions.len(), 2);
    assert!(out.insights.tokens_saved > 0, "the duplicate saves tokens");
}

#[tokio::test]
async fn test_compile_context_tool_pulls_memory_scope() {
    // Given a remembered fact and a scoped request
    let (_dir, srv) = server();
    srv.remember(Parameters(RememberParams {
        fact: "the deploy pipeline runs clippy before tests".to_owned(),
        links: Vec::new(),
        metadata: None,
        ttl_seconds: None,
    }))
    .await
    .expect("remember");
    let mut req = request("deploy pipeline checks", vec![fragment("note")], 10_000);
    req.memory_scope = Some(MemoryScope {
        k: Some(3),
        ..MemoryScope::default()
    });

    // When compiling through the tool
    let Json(value) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");
    let out = compiled_context_of(value);

    // Then the memory is pulled in with provenance
    assert!(out.content.contains("runs clippy before tests"));
    assert!(out.decisions.iter().any(|d| d.memory_id.is_some()));
}

#[tokio::test]
async fn test_context_savings_tool_aggregates_by_project() {
    // Given two compilations recorded under a project
    let (_dir, srv) = server();
    for _ in 0..2 {
        let mut req = request("deploy", vec![fragment("x"), fragment("x")], 10_000);
        req.project = Some("veles".to_owned());
        srv.compile_context(Parameters(req))
            .await
            .expect("compile_context");
    }

    // When aggregating through the tool
    let Json(savings) = srv
        .context_savings(Parameters(ContextSavingsParams {
            project: Some("veles".to_owned()),
        }))
        .await
        .expect("context_savings");

    // Then both events fold into the aggregate
    assert_eq!(savings.events, 2);
    assert!(savings.tokens_saved > 0);
}

#[tokio::test]
async fn test_explain_compilation_tool_returns_decision_for_fragment() {
    // Given a compiled request and one of its fragments
    let (_dir, srv) = server();
    let req = request(
        "deploy",
        vec![fragment("a fact"), fragment("other")],
        10_000,
    );
    let wanted = fragment_id("a fact");

    // When asking why that fragment was treated the way it was
    let Json(value) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: wanted,
            fragment_index: None,
        }))
        .await
        .expect("explain_compilation");
    let decision = decision_of(value);

    // Then the decision is returned with its rule and reason
    assert_eq!(decision.fragment_id, wanted);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert!(!decision.reason.is_empty());
}

#[tokio::test]
async fn test_explain_compilation_tool_unknown_fragment_is_invalid_params() {
    let (_dir, srv) = server();
    let req = request("deploy", vec![fragment("a fact")], 10_000);

    let Err(err) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: 424_242,
            fragment_index: None,
        }))
        .await
    else {
        panic!("no such fragment in the request — the tool must fail");
    };
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

// --- ids_as_strings (wire-compat, EPIC-P-071 wave 5 / 5.1) -----------------

/// A fragment id above 2^53 — the point where a raw JS `number` (IEEE-754
/// double) silently loses precision. `2^53 = 9_007_199_254_740_992`.
const ID_ABOVE_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_993;

#[tokio::test]
async fn test_compile_context_tool_ids_as_strings_stringifies_response_ids() {
    // Given a fragment whose caller-supplied id exceeds 2^53
    let (_dir, srv) = server();
    let mut fragment = fragment("a fact above the safe integer range");
    fragment.id = Some(ID_ABOVE_JS_SAFE_INTEGER);
    let mut req = request("deploy", vec![fragment], 10_000);
    req.policy = Some(CompilePolicy {
        ids_as_strings: true,
        ..CompilePolicy::default()
    });

    // When compiling with the option active
    let Json(value) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");

    // Then every id field on the wire is a decimal string, not a number —
    // a raw JS client parses this losslessly.
    let decision_id = &value["decisions"][0]["fragment_id"];
    assert_eq!(
        decision_id.as_str(),
        Some(ID_ABOVE_JS_SAFE_INTEGER.to_string().as_str()),
        "fragment_id must be a JSON string when ids_as_strings is active: {value}"
    );
    assert!(
        !decision_id.is_number(),
        "fragment_id must not still be a JSON number: {value}"
    );
}

#[tokio::test]
async fn test_compile_context_tool_ids_as_strings_default_false_is_byte_identical() {
    // Given the exact same request compiled with the option left at its
    // default (false) and explicitly set to false
    let (_dir, srv) = server();
    let fragment_a = {
        let mut f = fragment("a fact above the safe integer range");
        f.id = Some(ID_ABOVE_JS_SAFE_INTEGER);
        f
    };
    let fragment_b = fragment_a.clone();
    let req_default = request("deploy", vec![fragment_a], 10_000);
    let mut req_explicit_false = request("deploy", vec![fragment_b], 10_000);
    req_explicit_false.policy = Some(CompilePolicy {
        ids_as_strings: false,
        ..CompilePolicy::default()
    });

    // When compiling both
    let Json(default_value) = srv
        .compile_context(Parameters(req_default))
        .await
        .expect("compile_context (default policy)");
    let Json(explicit_value) = srv
        .compile_context(Parameters(req_explicit_false))
        .await
        .expect("compile_context (ids_as_strings: false)");

    // Then the response keeps ids as JSON numbers, byte-identical either way
    assert!(default_value["decisions"][0]["fragment_id"].is_number());
    assert_eq!(default_value, explicit_value);
}

#[tokio::test]
async fn test_explain_compilation_tool_ids_as_strings_stringifies_response_ids() {
    // Given a request whose policy opts into string ids
    let (_dir, srv) = server();
    let mut fragment = fragment("a fact above the safe integer range");
    fragment.id = Some(ID_ABOVE_JS_SAFE_INTEGER);
    let mut req = request("deploy", vec![fragment], 10_000);
    req.policy = Some(CompilePolicy {
        ids_as_strings: true,
        ..CompilePolicy::default()
    });

    // When explaining that fragment's decision
    let Json(value) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: ID_ABOVE_JS_SAFE_INTEGER,
            fragment_index: None,
        }))
        .await
        .expect("explain_compilation");

    // Then fragment_id and content_hash are decimal strings on the wire
    assert_eq!(
        value["fragment_id"].as_str(),
        Some(ID_ABOVE_JS_SAFE_INTEGER.to_string().as_str())
    );
    assert!(value["content_hash"].is_string());
}

#[tokio::test]
async fn test_compile_context_tool_accepts_fragment_id_as_decimal_string_on_input() {
    // Given a fragment whose id is supplied as a decimal string (e.g. a
    // client resubmitting an id it previously received stringified)
    let (_dir, srv) = server();
    let mut req_value = serde_json::to_value(request(
        "deploy",
        vec![fragment("a fact above the safe integer range")],
        10_000,
    ))
    .expect("serialize request");
    req_value["fragments"][0]["id"] =
        serde_json::Value::String(ID_ABOVE_JS_SAFE_INTEGER.to_string());
    let req: CompileRequest =
        serde_json::from_value(req_value).expect("fragment id accepts a decimal string");

    // When compiling
    let Json(value) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");

    // Then the fragment id round-trips exactly (as a number by default)
    assert_eq!(
        value["decisions"][0]["fragment_id"].as_u64(),
        Some(ID_ABOVE_JS_SAFE_INTEGER)
    );
}

// --- advertised schemas match the ids_as_strings wire contract -------------
// The official MCP SDKs (TS/Python, spec 2025-06-18) validate a tool's
// `structuredContent` against its advertised `outputSchema`. If the schema
// typed the id fields `integer` only, every `ids_as_strings: true` response
// would fail validation for exactly the clients the option exists for — so
// the advertised schemas must type each id field `["integer", "string"]`.

/// Recursively collect the type of every property named in `keys` across
/// the whole schema tree (`$defs` included), resolving array-typed
/// properties to their `items` type — so the assertions below cover every
/// occurrence, not just a hand-picked path.
fn collect_id_property_types(
    value: &serde_json::Value,
    keys: &[&str],
    found: &mut Vec<(String, serde_json::Value)>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Object(properties)) = map.get("properties") {
                for (name, subschema) in properties {
                    if keys.contains(&name.as_str()) {
                        let leaf = if subschema.get("type") == Some(&serde_json::json!("array")) {
                            subschema
                                .get("items")
                                .unwrap_or_else(|| panic!("array property {name} declares items"))
                        } else {
                            subschema
                        };
                        found.push((name.clone(), leaf["type"].clone()));
                    }
                }
            }
            for entry in map.values() {
                collect_id_property_types(entry, keys, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_id_property_types(item, keys, found);
            }
        }
        _ => {}
    }
}

/// Every collected id type must advertise BOTH `integer` and `string`.
fn assert_ids_widened(found: &[(String, serde_json::Value)]) {
    for (name, type_value) in found {
        let types = type_value
            .as_array()
            .unwrap_or_else(|| panic!("{name} must type a list of forms, got {type_value}"));
        assert!(
            types.contains(&serde_json::json!("integer"))
                && types.contains(&serde_json::json!("string")),
            "{name} must advertise integer|string on the wire, got {type_value}"
        );
    }
}

#[test]
fn test_compile_context_output_schema_advertises_string_ids() {
    let tool = McpServer::compile_context_tool_attr();
    let schema = serde_json::to_value(
        tool.output_schema
            .expect("compile_context declares an output schema"),
    )
    .expect("schema serializes");

    let mut found = Vec::new();
    collect_id_property_types(&schema, crate::context::wire::ID_KEYS, &mut found);

    let names: std::collections::BTreeSet<&str> =
        found.iter().map(|(name, _)| name.as_str()).collect();
    for expected in ["fragment_id", "content_hash", "memory_id", "fragment_ids"] {
        assert!(
            names.contains(expected),
            "the compile_context output schema must carry {expected}; found {names:?}"
        );
    }
    assert_ids_widened(&found);
}

#[test]
fn test_explain_compilation_output_schema_advertises_string_ids() {
    let tool = McpServer::explain_compilation_tool_attr();
    let schema = serde_json::to_value(
        tool.output_schema
            .expect("explain_compilation declares an output schema"),
    )
    .expect("schema serializes");

    let mut found = Vec::new();
    collect_id_property_types(&schema, crate::context::wire::ID_KEYS, &mut found);

    let names: std::collections::BTreeSet<&str> =
        found.iter().map(|(name, _)| name.as_str()).collect();
    for expected in ["fragment_id", "content_hash", "memory_id"] {
        assert!(
            names.contains(expected),
            "the explain_compilation output schema must carry {expected}; found {names:?}"
        );
    }
    assert_ids_widened(&found);
}

#[test]
fn test_compile_context_input_schema_advertises_string_fragment_id() {
    // fragments[].id accepts a decimal string on input (see
    // wire::deserialize_optional_id) — a client generating requests from the
    // advertised schema must be able to discover that.
    let tool = McpServer::compile_context_tool_attr();
    let schema = serde_json::to_value(&tool.input_schema).expect("schema serializes");

    let id_type = &schema["$defs"]["ContextFragment"]["properties"]["id"]["type"];
    let types = id_type
        .as_array()
        .unwrap_or_else(|| panic!("fragments[].id must type a list of forms, got {id_type}"));
    for expected in ["integer", "string", "null"] {
        assert!(
            types.contains(&serde_json::json!(expected)),
            "fragments[].id must advertise {expected} on input, got {id_type}"
        );
    }
}

#[test]
fn test_explain_compilation_input_schema_keeps_top_level_fragment_id_strict() {
    // The widening is scoped to fragments[].id: the tool's own fragment_id
    // parameter is deserialized as a strict u64 (a string is rejected), so
    // its advertised type must stay integer-only — announcing a string there
    // would over-promise.
    let tool = McpServer::explain_compilation_tool_attr();
    let schema = serde_json::to_value(&tool.input_schema).expect("schema serializes");

    assert_eq!(
        schema["properties"]["fragment_id"]["type"],
        serde_json::json!("integer"),
        "top-level fragment_id stays integer-only"
    );
    // …while the nested fragments[].id is widened, like compile_context's.
    let id_type = &schema["$defs"]["ContextFragment"]["properties"]["id"]["type"];
    let types = id_type
        .as_array()
        .unwrap_or_else(|| panic!("fragments[].id must type a list of forms, got {id_type}"));
    assert!(
        types.contains(&serde_json::json!("string")),
        "request.fragments[].id must advertise string on input, got {id_type}"
    );
}

// --- fragment_index (positional disambiguation, EPIC-P-071 wave 5 / 5.2) ---

#[tokio::test]
async fn test_explain_compilation_tool_fragment_index_disambiguates_byte_identical_twins() {
    // Given two byte-identical fragments (same content ⇒ same
    // content-addressed fragment_id, since neither sets a caller id)
    let (_dir, srv) = server();
    let req = request(
        "deploy",
        vec![fragment("duplicate payload"), fragment("duplicate payload")],
        10_000,
    );
    let shared_id = fragment_id("duplicate payload");

    // When asking for the decision by fragment_id alone (today's behavior)
    let Json(survivor_value) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req.clone(),
            fragment_id: shared_id,
            fragment_index: None,
        }))
        .await
        .expect("explain_compilation (by id)");
    let survivor = decision_of(survivor_value);

    // And when asking for the SECOND fragment's decision by position
    let Json(twin_value) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: shared_id,
            fragment_index: Some(1),
        }))
        .await
        .expect("explain_compilation (by index)");
    let twin = decision_of(twin_value);

    // Then the id-based lookup returns the deduplication survivor (kept,
    // verbatim), while the positional lookup returns the dropped twin's own
    // decision — not the same decision.
    assert!(matches!(survivor.action, ContextAction::Preserve));
    assert!(matches!(twin.action, ContextAction::Drop));
    assert_eq!(twin.rule_id, "drop.duplicate");
    assert_eq!(twin.fragment_id, shared_id);
}

#[tokio::test]
async fn test_explain_compilation_tool_fragment_index_out_of_bounds_is_invalid_params() {
    // Given a request with only one fragment
    let (_dir, srv) = server();
    let req = request("deploy", vec![fragment("a fact")], 10_000);
    let wanted = fragment_id("a fact");

    // When asking for an index beyond the fragment list
    let Err(err) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: wanted,
            fragment_index: Some(5),
        }))
        .await
    else {
        panic!("fragment_index 5 has no fragment — the tool must fail");
    };

    // Then the tool reports an invalid-params error with a clear reason
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("fragment_index"));
}

#[tokio::test]
async fn test_retrieve_context_source_tool_round_trips_original() {
    // Given a compiled fragment whose source was stored
    let (_dir, srv) = server();
    let original = "Never restart the primary node during a rebalance.";
    let req = request("rebalance", vec![fragment(original)], 10_000);
    let Json(value) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");
    let out = compiled_context_of(value);
    let handle = out.sources[0].handle.clone();

    // When retrieving through the tool
    let Json(retrieved) = srv
        .retrieve_context_source(Parameters(RetrieveContextSourceParams {
            handle: handle.clone(),
        }))
        .await
        .expect("retrieve_context_source");

    // Then the original bytes round-trip
    assert_eq!(retrieved.content, original);
    assert_eq!(retrieved.handle, handle);
}

#[tokio::test]
async fn test_compile_context_tool_zero_budget_is_invalid_params() {
    let (_dir, srv) = server();
    let req = request("deploy", vec![fragment("anything")], 0);

    let Err(err) = srv.compile_context(Parameters(req)).await else {
        panic!("a zero budget cannot compile — the tool must fail");
    };
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn test_retrieve_context_source_tool_unknown_handle_is_invalid_params() {
    let (_dir, srv) = server();
    let Err(err) = srv
        .retrieve_context_source(Parameters(RetrieveContextSourceParams {
            handle: "ctx://source/999999".to_owned(),
        }))
        .await
    else {
        panic!("nothing stored under this handle — the tool must fail");
    };
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

// --- save_working_context / load_working_context ---------------------------

fn working() -> WorkingContext {
    WorkingContext {
        goal: Some("ship EPIC-P-071 PR3".to_owned()),
        active_constraints: vec![ContextFact {
            text: "never merge without green gates".to_owned(),
            source: None,
        }],
        verified_facts: vec![ContextFact {
            text: "compile_context already ships on MCP+Node".to_owned(),
            source: None,
        }],
        open_hypotheses: Vec::new(),
        decisions: Vec::new(),
        exact_evidence: Vec::new(),
        pending_actions: vec!["wire save/load working-context tools".to_owned()],
    }
}

#[tokio::test]
async fn test_save_working_context_tool_then_load_round_trips() {
    // Given a server and a working context to persist
    let (_dir, srv) = server();
    let saved = working();

    // When saving through the tool
    let Json(save_result) = srv
        .save_working_context(Parameters(SaveWorkingContextParams {
            project: "veles".to_owned(),
            session: "session-1".to_owned(),
            working: saved.clone(),
        }))
        .await
        .expect("save_working_context");
    assert!(save_result.id > 0);

    // Then a later load (a fresh "session") recovers the exact same state —
    // this is the inter-session resumption the tool exists for.
    let Json(loaded) = srv
        .load_working_context(Parameters(LoadWorkingContextParams {
            project: "veles".to_owned(),
            session: "session-1".to_owned(),
        }))
        .await
        .expect("load_working_context");
    let recovered = loaded
        .working
        .expect("a previously saved working context must load back");
    assert_eq!(recovered.goal, saved.goal);
    assert_eq!(recovered.pending_actions, saved.pending_actions);
    assert_eq!(recovered.active_constraints.len(), 1);
}

#[tokio::test]
async fn test_load_working_context_tool_none_when_never_saved() {
    // Given a server with nothing saved under this project/session pair
    let (_dir, srv) = server();

    // When loading through the tool
    let Json(loaded) = srv
        .load_working_context(Parameters(LoadWorkingContextParams {
            project: "veles".to_owned(),
            session: "never-saved".to_owned(),
        }))
        .await
        .expect("load_working_context");

    // Then there is nothing to resume
    assert!(loaded.working.is_none());
}

#[tokio::test]
async fn test_save_working_context_tool_is_idempotent_upsert() {
    // Given an already-saved working context
    let (_dir, srv) = server();
    let mut state = working();
    srv.save_working_context(Parameters(SaveWorkingContextParams {
        project: "veles".to_owned(),
        session: "session-2".to_owned(),
        working: state.clone(),
    }))
    .await
    .expect("save_working_context");

    // When saving again under the same project+session with a new goal
    state.goal = Some("ship a follow-up PR".to_owned());
    srv.save_working_context(Parameters(SaveWorkingContextParams {
        project: "veles".to_owned(),
        session: "session-2".to_owned(),
        working: state.clone(),
    }))
    .await
    .expect("save_working_context (replace)");

    // Then loading returns the latest state, not the first
    let Json(loaded) = srv
        .load_working_context(Parameters(LoadWorkingContextParams {
            project: "veles".to_owned(),
            session: "session-2".to_owned(),
        }))
        .await
        .expect("load_working_context");
    assert_eq!(loaded.working.expect("saved").goal, state.goal);
}
