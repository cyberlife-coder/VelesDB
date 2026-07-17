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

// --- Review findings (2026-07-17): system-fact isolation & robustness -------

#[test]
fn test_system_facts_never_pollute_filtered_recall_or_memory_scope() {
    // Given a compilation that recorded an event and a saved working context,
    // both under a project facet
    let (_dir, svc) = service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let mut req = request(
        "incident",
        vec![fragment("caller note about the incident")],
        10_000,
    );
    req.project = Some("acme".to_owned());
    svc.compile_context(&compiler, &req).expect("compile");
    svc.save_working_context("acme", "s1", &WorkingContext::default())
        .expect("save");

    // When recalling with a caller-style project filter
    let mut filter = serde_json::Map::new();
    filter.insert(
        "project".to_owned(),
        serde_json::Value::String("acme".to_owned()),
    );
    let hits = svc
        .recall("compilation event working context", 10, Some(&filter))
        .expect("recall");

    // Then no system fact surfaces (events/working state carry no caller keys)
    assert!(
        hits.is_empty(),
        "system facts must be invisible to filtered recall, got {hits:?}"
    );

    // And a project-scoped memory pull can never compile them into a prompt
    let mut scoped = request("compilation event", vec![fragment("note")], 10_000);
    scoped.memory_scope = Some(MemoryScope {
        project: Some("acme".to_owned()),
        k: Some(10),
    });
    let out = svc.compile_context(&compiler, &scoped).expect("compile");
    assert!(
        !out.content.contains("veles context compilation event")
            && !out.content.contains("active_constraints"),
        "system facts must never be pulled as memories, got:\n{}",
        out.content
    );
}

#[test]
fn test_context_savings_ignores_forged_caller_events_and_never_overflows() {
    // Given ordinary caller facts that try to pose as compilation events
    let (_dir, svc) = service();
    let mut forged = serde_json::Map::new();
    forged.insert("ctx_event".to_owned(), serde_json::Value::Bool(true));
    forged.insert(
        "project".to_owned(),
        serde_json::Value::String("x".to_owned()),
    );
    forged.insert(
        "tokens_saved".to_owned(),
        serde_json::Value::Number(serde_json::Number::from(u64::MAX)),
    );
    forged.insert(
        "cost_saved_micros".to_owned(),
        serde_json::Value::Number(serde_json::Number::from(u64::MAX)),
    );
    forged.insert(
        "currency".to_owned(),
        serde_json::Value::String("USD".to_owned()),
    );
    svc.remember("a perfectly ordinary fact", &[], Some(&forged))
        .expect("remember");
    svc.remember("another ordinary fact", &[], Some(&forged))
        .expect("remember");

    // When aggregating savings
    let savings = svc
        .context_savings(Some("x"))
        .expect("savings must not panic");

    // Then forged facts count for nothing
    assert_eq!(savings.events, 0, "caller facts must never count as events");
    assert_eq!(savings.tokens_saved, 0);
    assert!(savings.cost_saved_micros_by_currency.is_empty());
}

#[test]
fn test_compile_context_memory_scope_respects_the_fragment_cap() {
    // Given a request already at the fragment cap and a scope asking for more
    let (_dir, svc) = service();
    svc.remember("the deploy pipeline runs clippy", &[], None)
        .expect("remember");
    let fragments: Vec<ContextFragment> = (0..velesdb_memory::limits::MAX_FRAGMENTS)
        .map(|i| fragment(&format!("note {i}")))
        .collect();
    let mut req = request("deploy pipeline", fragments, 100_000);
    req.memory_scope = Some(MemoryScope {
        project: None,
        k: Some(5),
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling — the bridge must not push the request over the cap
    let out = svc
        .compile_context(&compiler, &req)
        .expect("a valid request must stay valid with a memory scope");

    // Then exactly the caller's fragments were compiled (no room for pulls)
    assert_eq!(out.decisions.len(), velesdb_memory::limits::MAX_FRAGMENTS);
}

#[test]
fn test_retrieve_context_source_refuses_a_squatting_caller_fact() {
    // Given a compiled source and a caller fact remembered at the literal
    // salt-preimage of another handle's storage slot
    let (_dir, svc) = service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(
            &compiler,
            &request("q", vec![fragment("legit source")], 10_000),
        )
        .expect("compile");
    let legit_handle = out.sources[0].handle.clone();

    let squatted_hash: u64 = 424_242;
    svc.remember(&format!("veles-ctx-source:{squatted_hash}"), &[], None)
        .expect("remember");

    // When retrieving both handles
    let legit = svc.retrieve_context_source(&legit_handle).expect("legit");
    let squatted = svc.retrieve_context_source(&format!("ctx://source/{squatted_hash}"));

    // Then the real source round-trips and the squatter is never served back
    assert_eq!(legit, "legit source");
    let err = squatted.expect_err("a caller fact must never masquerade as a stored source");
    assert_eq!(err.category(), ErrorCategory::NotFound);
}

#[test]
fn test_source_ttl_zero_stores_permanently_like_remember() {
    // Given the crate-wide TTL convention: Some(0) means "no expiry"
    let (_dir, svc) = service();
    let policy = CompilePolicy {
        source_ttl_seconds: Some(0),
        ..CompilePolicy::default()
    };
    let mut req = request("q", vec![fragment("must stay retrievable")], 10_000);
    req.policy = Some(policy);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // When retrieving right away (an expired-at-once fact would already fail)
    let retrieved = svc
        .retrieve_context_source(&out.sources[0].handle)
        .expect("Some(0) must mean permanent, exactly like remember_with_ttl");

    // Then the source is there
    assert_eq!(retrieved, "must stay retrievable");
}

// --- Coverage round (2026-07-17): pricing trail, writer guard, provenance ----

#[test]
fn test_context_savings_aggregates_cost_by_currency_when_pricing_injected() {
    // Given a service compiling twice with a pricing table and a project
    let (_dir, svc) = service();
    let mut models = std::collections::BTreeMap::new();
    models.insert(
        "claude-sonnet-5".to_owned(),
        velesdb_memory::context::ModelPricing {
            input_micros_per_million_tokens: 3_000_000,
        },
    );
    let pricing = velesdb_memory::context::PricingTable {
        version: "2026-07".to_owned(),
        currency: "EUR".to_owned(),
        models,
    };
    let compiler = ContextCompiler::new(CompilePolicy::default()).with_pricing(pricing);
    let dup = "The rebalance pauses ingestion on the affected shard.";
    let mut expected_micros = 0_u64;
    for _ in 0..2 {
        let mut req = request("rebalance", vec![fragment(dup), fragment(dup)], 10_000);
        req.project = Some("acme".to_owned());
        req.target_model = Some("claude-sonnet-5".to_owned());
        let out = svc.compile_context(&compiler, &req).expect("compile");
        expected_micros += out
            .insights
            .estimated_cost_saved_micros
            .expect("priced model must yield a cost figure");
    }

    // When aggregating the project's savings
    let savings = svc.context_savings(Some("acme")).expect("savings");

    // Then the cost trail sums per currency, exactly
    assert_eq!(savings.events, 2);
    assert!(expected_micros > 0);
    assert_eq!(
        savings.cost_saved_micros_by_currency.get("EUR").copied(),
        Some(expected_micros),
        "the recorded events must carry and aggregate the cost figures"
    );
}

#[test]
fn test_store_context_sources_never_clobbers_a_caller_fact_squatting_the_slot() {
    // Given a caller fact remembered at the literal salt-preimage of the
    // slot where a future compile would store its source
    let (_dir, svc) = service();
    let content = "a fragment whose source slot is already squatted";
    let hash = velesdb_memory::context::fragment_id(content);
    let squat = format!("veles-ctx-source:{hash}");
    let squat_id = svc.remember(&squat, &[], None).expect("remember");

    // When compiling that content (store_sources defaults to true)
    let compiler = ContextCompiler::new(CompilePolicy::default());
    svc.compile_context(&compiler, &request("q", vec![fragment(content)], 10_000))
        .expect("compile");

    // Then the caller's fact is intact (never overwritten by the writer) ...
    let hits = svc.recall(&squat, 3, None).expect("recall");
    assert!(
        hits.iter().any(|h| h.id == squat_id && h.content == squat),
        "the squatting caller fact must survive a compile of the colliding content"
    );
    // ... and the handle stays unresolvable rather than serving wrong bytes
    let err = svc
        .retrieve_context_source(&format!("ctx://source/{hash}"))
        .expect_err("a squatted slot must not resolve");
    assert_eq!(err.category(), ErrorCategory::NotFound);
}

#[test]
fn test_pulled_memory_source_reference_carries_its_memory_id() {
    // Given a remembered fact pulled into a compilation via memory scope
    let (_dir, svc) = service();
    let memory_id = svc
        .remember("the canary stage rolls to five percent first", &[], None)
        .expect("remember");
    let mut req = request("canary rollout", vec![fragment("Session note.")], 10_000);
    req.memory_scope = Some(MemoryScope {
        project: None,
        k: Some(3),
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the pulled memory's source reference links back to the memory id
    let hash = velesdb_memory::context::fragment_id("the canary stage rolls to five percent first");
    let source = out
        .sources
        .iter()
        .find(|s| s.handle.ends_with(&hash.to_string()))
        .expect("the pulled memory must have a source reference");
    assert_eq!(
        source.memory_id,
        Some(memory_id),
        "provenance must link the source back to the memory it came from"
    );
}
