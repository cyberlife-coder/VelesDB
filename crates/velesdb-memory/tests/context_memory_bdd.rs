//! BDD integration tests for the context compiler's memory bridge
//! (US-002 of EPIC-P-070): memory-backed fragment selection, source
//! round-trips, working contexts, and compilation events.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::service;
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextCompiler, ContextFragment, DeterministicReranker,
    MemoryScope, WorkingContext,
};
use velesdb_memory::{ErrorCategory, FusionOptions, HashEmbedder, MemoryService};

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

// --- Nominal -----------------------------------------------------------------

#[test]
fn test_compile_context_memory_scope_pulls_relevant_memory_with_provenance() {
    // Given a remembered fact relevant to the query
    let (_dir, svc) = service();
    let memory_id = svc
        .remember("the deploy pipeline runs clippy before tests", &[], None)
        .expect("remember");

    // When compiling with a memory scope
    let mut req = request(
        "deploy pipeline checks",
        vec![fragment("Session note: user asked about CI.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        project: None,
        k: Some(3),
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the memory is pulled in with full provenance
    assert!(
        out.content.contains("runs clippy before tests"),
        "the relevant memory must be compiled in, got:\n{}",
        out.content
    );
    let memory_decision = out
        .decisions
        .iter()
        .find(|d| d.memory_id == Some(memory_id))
        .expect("the pulled memory must carry its memory_id in provenance");
    assert!(
        (0.0..=1.0).contains(&memory_decision.relevance),
        "memory relevance must be normalised into [0, 1]"
    );
}

#[test]
fn test_compile_context_without_scope_matches_memoryless_compile() {
    // Given a request with no memory scope
    let (_dir, svc) = service();
    svc.remember("an unrelated remembered fact", &[], None)
        .expect("remember");
    let req = request("deploy", vec![fragment("Only caller content.")], 10_000);
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling through the bridge and through the bare compiler
    let bridged = svc.compile_context(&compiler, &req).expect("bridged");
    let bare = compiler.compile(&req).expect("bare");

    // Then the compiled content is identical (the bridge only adds memories
    // when a scope asks for them)
    assert_eq!(bridged.content, bare.content);
    assert_eq!(bridged.decisions.len(), bare.decisions.len());
}

#[test]
fn test_retrieve_context_source_round_trips_original() {
    // Given a compiled request whose sources were stored
    let (_dir, svc) = service();
    let original = "Never restart the primary node during a rebalance.";
    let req = request("rebalance", vec![fragment(original)], 10_000);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // When retrieving the source behind its handle
    let handle = &out.sources[0].handle;
    let retrieved = svc.retrieve_context_source(handle).expect("retrieve");

    // Then the exact original bytes come back
    assert_eq!(retrieved, original);
}

#[test]
fn test_working_context_round_trips_across_reopen() {
    // Given a saved working context
    let dir = tempfile::TempDir::new().expect("tempdir");
    let wc = WorkingContext {
        goal: Some("ship US-002".to_owned()),
        pending_actions: vec!["open PR2".to_owned()],
        ..WorkingContext::default()
    };
    {
        let svc = MemoryService::open(dir.path(), HashEmbedder::new(common::DIM)).expect("open");
        svc.save_working_context("veles", "session-1", &wc)
            .expect("save");
    }

    // When reopening the store in a new service (a later session)
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(common::DIM)).expect("reopen");
    let loaded = svc
        .load_working_context("veles", "session-1")
        .expect("load")
        .expect("the working context must survive the reopen");

    // Then the working state is intact
    assert_eq!(loaded.goal.as_deref(), Some("ship US-002"));
    assert_eq!(loaded.pending_actions, vec!["open PR2".to_owned()]);
}

#[test]
fn test_compile_context_records_aggregatable_events() {
    // Given two compilations under one project and one under another
    let (_dir, svc) = service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for _ in 0..2 {
        let mut req = request("deploy", vec![fragment("a"), fragment("a")], 10_000);
        req.project = Some("veles".to_owned());
        svc.compile_context(&compiler, &req).expect("compile");
    }
    let mut other = request("deploy", vec![fragment("b")], 10_000);
    other.project = Some("other".to_owned());
    svc.compile_context(&compiler, &other).expect("compile");

    // When aggregating savings per project
    let veles = svc.context_savings(Some("veles")).expect("savings");
    let other_project = svc.context_savings(Some("other")).expect("savings");
    let all = svc.context_savings(None).expect("savings");

    // Then events aggregate by project and across projects
    assert_eq!(veles.events, 2);
    assert_eq!(other_project.events, 1);
    assert_eq!(all.events, 3);
    assert!(veles.tokens_saved > 0, "the duplicate drop saved tokens");
    assert!(!all.truncated);
}

#[test]
fn test_recall_fused_reranked_with_deterministic_reranker_orders_by_overlap() {
    // Given facts of varying lexical overlap with the query
    let (_dir, svc) = service();
    svc.remember("the cat sat on the mat", &[], None)
        .expect("remember");
    svc.remember("deploy pipeline runs clippy", &[], None)
        .expect("remember");
    svc.remember(
        "clippy pedantic gates the deploy pipeline strictly",
        &[],
        None,
    )
    .expect("remember");

    // When recalling with the first shipped deterministic reranker
    let hits = svc
        .recall_fused_reranked(
            "deploy pipeline clippy",
            3,
            None,
            FusionOptions::default(),
            &DeterministicReranker,
        )
        .expect("recall");

    // Then the most lexically overlapping fact leads and nothing is dropped
    assert_eq!(hits.len(), 3);
    assert!(
        hits[0].content.contains("clippy"),
        "the top hit must overlap the query, got: {}",
        hits[0].content
    );
    assert!(
        !hits[0].content.contains("cat sat"),
        "the unrelated fact must not lead"
    );
}

// --- Edge --------------------------------------------------------------------

#[test]
fn test_compile_context_system_facts_never_pollute_recall() {
    // Given a compilation that stored sources and an event
    let (_dir, svc) = service();
    let sensitive = "internal incident postmortem draft for the veles cluster";
    let req = request("incident", vec![fragment(sensitive)], 10_000);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    svc.compile_context(&compiler, &req).expect("compile");

    // When recalling normally for that content
    let hits = svc.recall(sensitive, 10, None).expect("recall");

    // Then neither the stored source nor the event surfaces as a memory
    assert!(
        hits.is_empty(),
        "compiler system facts must stay out of normal recall, got {hits:?}"
    );
}

#[test]
fn test_working_context_load_missing_returns_none() {
    let (_dir, svc) = service();
    let loaded = svc
        .load_working_context("veles", "no-such-session")
        .expect("load");
    assert!(loaded.is_none());
}

// --- Negative ----------------------------------------------------------------

#[test]
fn test_compile_context_event_and_sources_opt_out() {
    // Given a policy that opts out of events and source storage
    let (_dir, svc) = service();
    let policy = CompilePolicy {
        record_events: false,
        store_sources: false,
        ..CompilePolicy::default()
    };
    let mut req = request("deploy", vec![fragment("caller content")], 10_000);
    req.project = Some("veles".to_owned());
    req.policy = Some(policy);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // When aggregating and retrieving afterwards
    let savings = svc.context_savings(Some("veles")).expect("savings");
    let retrieved = svc.retrieve_context_source(&out.sources[0].handle);

    // Then nothing was recorded and the source is not retrievable
    assert_eq!(savings.events, 0, "opt-out must record no event");
    let err = retrieved.expect_err("opt-out must not store sources");
    assert_eq!(err.category(), ErrorCategory::NotFound);
}

#[test]
fn test_retrieve_context_source_unknown_handle_is_not_found() {
    let (_dir, svc) = service();
    let err = svc
        .retrieve_context_source("ctx://source/1234567890")
        .expect_err("nothing was stored under this handle");
    assert_eq!(err.category(), ErrorCategory::NotFound);
}

#[test]
fn test_retrieve_context_source_malformed_handle_is_not_found() {
    let (_dir, svc) = service();
    for bad in ["not-a-handle", "ctx://source/", "ctx://source/xyz", ""] {
        let err = svc
            .retrieve_context_source(bad)
            .expect_err("malformed handles must fail");
        assert_eq!(err.category(), ErrorCategory::NotFound, "handle: {bad}");
    }
}
