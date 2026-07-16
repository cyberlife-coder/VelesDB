//! Unit tests for the context compiler MCP tools (split out of
//! context_tools.rs, same `#[cfg(test)]`-via-`#[path]` pattern as
//! server_tests.rs).

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorCode;
use tempfile::TempDir;

use super::super::dto::RememberParams;
use super::*;
use crate::context::{fragment_id, ContextAction, ContextFragment, MemoryScope};
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
    let Json(out) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");

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
        project: None,
        k: Some(3),
    });

    // When compiling through the tool
    let Json(out) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");

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
    let Json(decision) = srv
        .explain_compilation(Parameters(ExplainCompilationParams {
            request: req,
            fragment_id: wanted,
        }))
        .await
        .expect("explain_compilation");

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
        }))
        .await
    else {
        panic!("no such fragment in the request — the tool must fail");
    };
    assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
}

#[tokio::test]
async fn test_retrieve_context_source_tool_round_trips_original() {
    // Given a compiled fragment whose source was stored
    let (_dir, srv) = server();
    let original = "Never restart the primary node during a rebalance.";
    let req = request("rebalance", vec![fragment(original)], 10_000);
    let Json(out) = srv
        .compile_context(Parameters(req))
        .await
        .expect("compile_context");
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
