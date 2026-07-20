//! BDD integration tests for the context compiler's memory bridge
//! (US-002 of EPIC-P-070): memory-backed fragment selection, source
//! round-trips, working contexts, and compilation events.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::service;
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextAction, ContextCompiler, ContextFragment,
    DeterministicReranker, MediaRef, MemoryScope, WorkingContext,
};
use velesdb_memory::{ErrorCategory, FusionOptions, HashEmbedder, MemoryService};

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        path: None,
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

/// A syntactically valid (well-formed base64), tiny PNG header — its exact
/// bytes don't matter to these tests, only that `media::decode_base64`
/// accepts them.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwCAYAAAAAAAAA";

/// Build a fragment carrying an inline PNG media payload.
fn media_fragment(caption: &str, bytes_b64: &str) -> ContextFragment {
    ContextFragment {
        media: Some(MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: bytes_b64.to_owned(),
        }),
        ..fragment(caption)
    }
}

/// A well-formed base64 payload distinct per `seed` — media dedup keys on
/// raw bytes alone, so two fragments needing DIFFERENT bytes must pass
/// different seeds. The seed is an explicit, caption-INDEPENDENT parameter
/// on purpose (review finding on PR2's first cut): deriving bytes from the
/// caption would make "same caption, different bytes" — the exact scenario
/// of the media handle-collision bug — inexpressible in these tests. Only
/// the final base64 character is varied (still a valid unpadded quad).
fn distinct_media_b64(seed: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // A multiplicative fold, not a plain byte sum: same-length seeds like
    // "stale-A"/"fresh-B" must not collide.
    let hash = seed
        .bytes()
        .fold(0_u64, |h, b| h.wrapping_mul(131).wrapping_add(u64::from(b)));
    let mut b64 = PNG_B64[..PNG_B64.len() - 2].to_owned();
    b64.push(ALPHABET[usize::try_from(hash % 64).unwrap_or(0)] as char);
    b64.push(ALPHABET[usize::try_from((hash / 64) % 64).unwrap_or(0)] as char);
    b64
}

/// Build a `kind: "screenshot"` media fragment naming its `metadata.target`,
/// with bytes derived from `seed` (independent of `caption` — see
/// [`distinct_media_b64`]).
fn screenshot_with_seed(caption: &str, target: &str, seed: &str) -> ContextFragment {
    let mut meta = serde_json::Map::new();
    meta.insert(
        "target".to_owned(),
        serde_json::Value::String(target.to_owned()),
    );
    ContextFragment {
        kind: Some("screenshot".to_owned()),
        metadata: Some(meta),
        ..media_fragment(caption, &distinct_media_b64(seed))
    }
}

/// [`screenshot_with_seed`] with the caption reused as the byte seed — for
/// tests where the two need not be independent.
fn screenshot(caption: &str, target: &str) -> ContextFragment {
    screenshot_with_seed(caption, target, caption)
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
        k: Some(3),
        ..MemoryScope::default()
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

    // Then the exact original bytes come back, with no media (text-only)
    assert_eq!(retrieved.content, original);
    assert!(retrieved.media.is_none());
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
        ..MemoryScope::default()
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
        k: Some(5),
        ..MemoryScope::default()
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
    assert_eq!(legit.content, "legit source");
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
    assert_eq!(retrieved.content, "must stay retrievable");
}

// The never-downgrade TTL upgrade rule (permanent-upgrades-TTL,
// TTL-never-downgrades-permanent, TTL-extension-only) is covered by unit
// tests in `src/context/memory_bridge_tests.rs`, not here: those assertions
// read the reserved `_veles_expires_at` metadata directly (unreachable from
// this integration binary — reserved keys are always stripped from the
// public API), which is also what keeps them from being a sleep-past-a-real-TTL
// test, the flaky alternative under this suite's parallel test load.

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
        k: Some(3),
        ..MemoryScope::default()
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

#[test]
fn test_memory_scope_graph_boost_pulls_evidence_sharing_no_words_with_the_query() {
    // Given a cause-chain in memory: a symptom fact (lexically close to the
    // query) linked to a fix fact that shares NO vocabulary with the query,
    // plus a distractor that out-scores the fix in the lexical vector space
    let (_dir, svc) = service();
    let symptom = svc
        .remember(
            "the payments checkout flow returns five hundred and two errors under peak load",
            &[],
            None,
        )
        .expect("remember");
    let fix = svc
        .remember(
            "raising the pool acquisition timeout to forty-five seconds stopped the cascade",
            &[],
            None,
        )
        .expect("remember");
    svc.relate(symptom, fix, "fixed_by").expect("relate");
    svc.remember(
        "the release notifications are posted to the payments channel under the weekly load report",
        &[],
        None,
    )
    .expect("remember distractor");

    // When compiling with a memory scope that raises the graph boost —
    // built from the exact wire JSON an MCP/Node caller sends
    let raw = r#"{
        "query": "why does the payments checkout flow fail under peak load",
        "token_budget": 4000,
        "fragments": [{"content": "Session note."}],
        "memory_scope": {"k": 2, "graph_boost": 0.8}
    }"#;
    let req: CompileRequest = serde_json::from_str(raw).expect("wire shape");
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the graph-reached fix — invisible to lexical/vector matching —
    // is compiled into the context with memory provenance
    assert!(
        out.content
            .contains("forty-five seconds stopped the cascade"),
        "the boosted graph walk must pull the zero-overlap evidence, got:\n{}",
        out.content
    );
    let fix_decision = out
        .decisions
        .iter()
        .find(|d| d.memory_id == Some(fix))
        .expect("the fix must carry its memory id in provenance");
    assert!(fix_decision.relevance > 0.0);
}

// --- Reranker seam (2026-07-17): the last engine capability wired in -------

/// A stand-in for a caller's cross-encoder: promotes the memory containing
/// its marker to the front, keeps every other candidate in place.
struct MarkerReranker(&'static str);

impl velesdb_memory::Reranker for MarkerReranker {
    fn rerank(
        &self,
        _query: &str,
        mut candidates: Vec<velesdb_memory::Recollection>,
    ) -> Result<Vec<velesdb_memory::Recollection>, velesdb_memory::RerankError> {
        candidates.sort_by_key(|c| usize::from(!c.content.contains(self.0)));
        Ok(candidates)
    }
}

#[test]
fn test_compile_context_reranked_lets_a_cross_encoder_drive_memory_selection() {
    // Given two memories where the fused (lexical-vector) ranking prefers
    // the wordy near-miss, while a semantic reranker knows the terse one is
    // the real answer
    let (_dir, svc) = service();
    svc.remember(
        "the deploy pipeline deploy checks deploy the canary deploy stage",
        &[],
        None,
    )
    .expect("remember wordy near-miss");
    let answer = svc
        .remember("promotion is gated on checksum verification", &[], None)
        .expect("remember terse answer");

    let mut req = request(
        "deploy pipeline checks",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(1),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling once with the fused default and once with the reranker
    let fused_only = svc.compile_context(&compiler, &req).expect("compile");
    let reranked = svc
        .compile_context_reranked(&compiler, &req, &MarkerReranker("checksum"))
        .expect("compile reranked");

    // Then the reranker changed which memory was selected (k=1), and the
    // selected memory carries its provenance
    assert!(
        !fused_only.content.contains("checksum verification"),
        "precondition: the fused default must prefer the wordy near-miss"
    );
    assert!(
        reranked
            .content
            .contains("promotion is gated on checksum verification"),
        "the reranker must drive selection, got:\n{}",
        reranked.content
    );
    let picked = reranked
        .decisions
        .iter()
        .find(|d| d.memory_id == Some(answer))
        .expect("the reranked pull must carry its memory id");
    assert!(picked.relevance > 0.0);
}

#[test]
fn test_compile_context_reranked_with_a_lexical_reranker_demotes_graph_rescues() {
    // Given a symptom -> fix chain whose fix shares no vocabulary with the
    // query (the tri-engine rescue case)
    let (_dir, svc) = service();
    let symptom = svc
        .remember("checkout requests fail under peak load", &[], None)
        .expect("remember");
    let fix = svc
        .remember(
            "raising the acquisition timeout stopped the cascade",
            &[],
            None,
        )
        .expect("remember");
    svc.relate(symptom, fix, "fixed_by").expect("relate");

    let raw = r#"{
        "query": "why do checkout requests fail under peak load",
        "token_budget": 4000,
        "fragments": [{"content": "Session note."}],
        "memory_scope": {"k": 2, "graph_boost": 0.8}
    }"#;
    let req: CompileRequest = serde_json::from_str(raw).expect("wire shape");
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When selecting with the boosted fusion vs re-ranking that same pool
    // with the shipped LEXICAL reranker
    let fused = svc.compile_context(&compiler, &req).expect("compile");
    let lexical = velesdb_memory::context::DeterministicReranker;
    let reranked = svc
        .compile_context_reranked(&compiler, &req, &lexical)
        .expect("compile reranked");

    // Then fusion surfaces the zero-overlap fix — and the lexical reranker
    // (scoring by word overlap alone) demotes it out of k=2's front, which
    // is exactly why no reranker runs by default: a lexical second stage
    // would undo the graph rescue. The seam exists for SEMANTIC rerankers.
    assert!(
        fused.content.contains("stopped the cascade"),
        "precondition: the boosted fusion must rescue the fix"
    );
    assert!(
        reranked.decisions.iter().any(|d| d.memory_id == Some(fix)),
        "rerank reorders but never drops: the fix stays in the pulled set"
    );
}

// --- Usage-driven importance blend (EPIC-P-071/US-002) ----------------------
//
// The blend composes the whole retrieval stack — HNSW seed, BFS `relate`
// reach, `graph_boost` fusion, the reranker seam, RL confidence, recency —
// into ONE ranking: `fused_norm + w_c·(confidence−0.5)·2 + w_r·recency_norm`,
// applied strictly AFTER the fused similarity selected the pool.

use velesdb_memory::context::ImportanceWeights;

fn importance_policy(confidence: f64, recency: f64, field: Option<&str>) -> CompilePolicy {
    CompilePolicy {
        importance: ImportanceWeights {
            confidence,
            recency,
            recency_field: field.map(str::to_owned),
        },
        ..CompilePolicy::default()
    }
}

/// Position of `needle` in the compiled content, panicking when absent.
fn pos(out: &velesdb_memory::context::CompiledContext, needle: &str) -> usize {
    out.content
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` must be compiled in, got:\n{}", out.content))
}

#[test]
fn test_importance_confidence_reinforced_memory_leads_at_equal_similarity() {
    // Given two equally on-topic memories about the same runbook topic
    let (_dir, svc) = service();
    let alpha = "postgres pool sizing guidance from runbook alpha";
    let beta = "postgres pool sizing guidance from runbook beta";
    let alpha_id = svc.remember(alpha, &[], None).expect("remember");
    let beta_id = svc.remember(beta, &[], None).expect("remember");

    let mut req = request(
        "postgres pool sizing",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling with the blend OFF, one of the two trails
    req.policy = Some(importance_policy(0.0, 0.0, None));
    let baseline = svc.compile_context(&compiler, &req).expect("baseline");
    let (trailing_text, trailing_id) = if pos(&baseline, alpha) < pos(&baseline, beta) {
        (beta, beta_id)
    } else {
        (alpha, alpha_id)
    };

    // And the team keeps marking the trailing fact useful, session after
    // session, then compiles with the confidence blend ON
    for _ in 0..15 {
        svc.feedback(trailing_id, true).expect("feedback");
    }
    req.policy = Some(importance_policy(1.0, 0.0, None));
    let blended = svc.compile_context(&compiler, &req).expect("blended");

    // Then the reinforced memory now leads the compiled context
    let other = if trailing_text == alpha { beta } else { alpha };
    assert!(
        pos(&blended, trailing_text) < pos(&blended, other),
        "the reinforced memory must out-rank its equally-similar twin, got:\n{}",
        blended.content
    );
}

#[test]
fn test_importance_recency_field_recent_memory_leads_over_older_one() {
    // Given an OLD memory lexically closer to the query and a NEWER one,
    // both dated with the YYYYMMDD convention of dated recall
    let (_dir, svc) = service();
    let old_text = "database connection tuning notes: keep the pool small";
    let new_text = "database defaults changed after the platform upgrade";
    svc.remember(
        old_text,
        &[],
        Some(&common::meta(&[("day", serde_json::json!(20_240_101))])),
    )
    .expect("remember");
    svc.remember(
        new_text,
        &[],
        Some(&common::meta(&[("day", serde_json::json!(20_260_715))])),
    )
    .expect("remember");

    let mut req = request(
        "database connection tuning",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling with the blend OFF the older, wordier fact leads
    req.policy = Some(importance_policy(0.0, 0.0, None));
    let baseline = svc.compile_context(&compiler, &req).expect("baseline");
    assert!(
        pos(&baseline, old_text) < pos(&baseline, new_text),
        "precondition: similarity alone must prefer the older fact"
    );

    // And compiling again with the recency term active
    req.policy = Some(importance_policy(0.0, 1.0, Some("day")));
    let blended = svc.compile_context(&compiler, &req).expect("blended");

    // Then the recent memory leads — recency is batch-relative, no clock
    assert!(
        pos(&blended, new_text) < pos(&blended, old_text),
        "the recent memory must lead once recency weighs in, got:\n{}",
        blended.content
    );
}

#[test]
fn test_importance_zero_weights_output_is_byte_identical_to_0_8_0_golden() {
    // Given the exact scenario the committed 0.8.0 golden was captured on:
    // dated memories, a relate chain, and a heavily reinforced fact
    let (_dir, svc) = service();
    let old_id = svc
        .remember(
            "the deploy pipeline ran mandatory clippy gates last winter",
            &[],
            Some(&common::meta(&[("day", serde_json::json!(20_260_101))])),
        )
        .expect("remember");
    let new_id = svc
        .remember(
            "the deploy pipeline now runs clippy pedantic before tests",
            &[],
            Some(&common::meta(&[("day", serde_json::json!(20_260_715))])),
        )
        .expect("remember");
    let fix = svc
        .remember(
            "switching the runner image stopped the flaky gate",
            &[],
            None,
        )
        .expect("remember");
    svc.relate(new_id, fix, "fixed_by").expect("relate");
    for _ in 0..10 {
        svc.feedback(old_id, true).expect("feedback");
    }

    // When compiling with BOTH importance weights at zero (the recency
    // field may even be named — a zero weight keeps it inert)
    let mut req = request(
        "deploy pipeline clippy checks",
        vec![fragment("Session note: user asked about CI.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(3),
        ..MemoryScope::default()
    });
    req.policy = Some(importance_policy(0.0, 0.0, Some("day")));
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the serialized output matches the pre-blend golden byte for byte
    let actual = serde_json::to_value(&out).expect("serialize");
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("golden/context/compile_importance_zero.json"))
            .expect("parse committed golden");
    assert_eq!(
        actual,
        golden,
        "zero importance weights must reproduce the 0.8.0 output exactly — actual:\n{}",
        serde_json::to_string_pretty(&actual).expect("pretty")
    );
}

#[test]
fn test_importance_confidence_never_admits_off_topic_memory_into_pool() {
    // Given on-topic memories and an OFF-topic fact the team over-reinforced
    let (_dir, svc) = service();
    svc.remember("the deploy pipeline runs clippy before tests", &[], None)
        .expect("remember");
    svc.remember("the deploy pipeline gates on cargo deny", &[], None)
        .expect("remember");
    let coffee = svc
        .remember(
            "the office coffee machine descaling schedule is pinned in the kitchen",
            &[],
            None,
        )
        .expect("remember");
    for _ in 0..20 {
        svc.feedback(coffee, true).expect("feedback");
    }

    // When compiling with the confidence blend at full strength
    let mut req = request(
        "deploy pipeline checks",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    req.policy = Some(importance_policy(1.0, 0.0, None));
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the off-topic fact is NOT in the pool: confidence is not
    // relevance — the blend only re-ranks what similarity already selected
    assert!(
        !out.content.contains("coffee"),
        "an over-reinforced off-topic fact must never enter the pool, got:\n{}",
        out.content
    );
    assert!(
        out.decisions.iter().all(|d| d.memory_id != Some(coffee)),
        "no decision may be backed by the off-topic memory"
    );
}

#[test]
fn test_importance_reason_ventilates_vector_graph_confidence_and_recency() {
    // Given a reinforced, dated memory pulled under an active blend
    let (_dir, svc) = service();
    let id = svc
        .remember(
            "the canary stage rolls to five percent first",
            &[],
            Some(&common::meta(&[("day", serde_json::json!(20_260_701))])),
        )
        .expect("remember");
    svc.feedback(id, true).expect("feedback");

    let mut req = request("canary rollout", vec![fragment("Session note.")], 10_000);
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    req.policy = Some(importance_policy(0.2, 0.1, Some("day")));
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the decision's reason ventilates all four ranking signals
    let decision = out
        .decisions
        .iter()
        .find(|d| d.memory_id == Some(id))
        .expect("the pulled memory must carry provenance");
    for signal in ["vector ", "graph ", "confidence ", "recency "] {
        assert!(
            decision.reason.contains(signal),
            "reason must ventilate `{signal}`, got: {}",
            decision.reason
        );
    }
}

#[test]
fn test_importance_recency_missing_key_and_degenerate_batch_stay_neutral() {
    // Given one dated pair plus one memory WITHOUT the recency key
    let (_dir, svc) = service();
    svc.remember(
        "the rollout freeze applies to the payments cluster",
        &[],
        Some(&common::meta(&[("day", serde_json::json!(20_250_101))])),
    )
    .expect("remember");
    svc.remember(
        "the rollout freeze applies to the search cluster",
        &[],
        Some(&common::meta(&[("day", serde_json::json!(20_260_601))])),
    )
    .expect("remember");
    let keyless = svc
        .remember("the rollout freeze applies to the auth cluster", &[], None)
        .expect("remember");

    let mut req = request("rollout freeze", vec![fragment("Session note.")], 10_000);
    req.memory_scope = Some(MemoryScope {
        k: Some(3),
        ..MemoryScope::default()
    });
    req.policy = Some(importance_policy(0.0, 1.0, Some("day")));
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling with the recency term active
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then the keyless memory contributes 0 — present, never penalised
    let keyless_decision = out
        .decisions
        .iter()
        .find(|d| d.memory_id == Some(keyless))
        .expect("the keyless memory must still be pulled");
    assert!(
        keyless_decision.reason.contains("recency 0.00"),
        "a memory without the key must read recency 0, got: {}",
        keyless_decision.reason
    );
    // And min/max normalisation is batch-relative: newest 1, oldest 0
    let reasons: Vec<&str> = out
        .decisions
        .iter()
        .filter(|d| d.memory_id.is_some())
        .map(|d| d.reason.as_str())
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("recency 1.00")),
        "the newest dated memory must read recency 1.00, got: {reasons:?}"
    );

    // And given a degenerate batch (every date equal), all contributions are 0
    let (_dir2, svc2) = service();
    for cluster in ["payments", "search"] {
        svc2.remember(
            &format!("the rollout freeze applies to the {cluster} cluster"),
            &[],
            Some(&common::meta(&[("day", serde_json::json!(20_260_601))])),
        )
        .expect("remember");
    }
    let mut req2 = request("rollout freeze", vec![fragment("Session note.")], 10_000);
    req2.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    req2.policy = Some(importance_policy(0.0, 1.0, Some("day")));
    let out2 = svc2.compile_context(&compiler, &req2).expect("compile");
    for decision in out2.decisions.iter().filter(|d| d.memory_id.is_some()) {
        assert!(
            decision.reason.contains("recency 0.00"),
            "max == min must zero every recency contribution, got: {}",
            decision.reason
        );
    }
}

#[test]
fn test_importance_blend_composes_with_reranked_memory_selection() {
    // Given two memories where a semantic reranker (the seam) puts the
    // marker fact first, but the OTHER fact is the one the team reinforced
    let (_dir, svc) = service();
    svc.remember("promotion is gated on checksum verification", &[], None)
        .expect("remember");
    let reinforced = svc
        .remember("promotion is gated on the canary error budget", &[], None)
        .expect("remember");
    for _ in 0..15 {
        svc.feedback(reinforced, true).expect("feedback");
    }

    let mut req = request(
        "what gates the promotion",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When the reranker drives selection with the blend OFF, then ON
    req.policy = Some(importance_policy(0.0, 0.0, None));
    let baseline = svc
        .compile_context_reranked(&compiler, &req, &MarkerReranker("checksum"))
        .expect("compile reranked");
    assert!(
        pos(&baseline, "checksum verification") < pos(&baseline, "canary error budget"),
        "precondition: the reranker alone must lead with its marker pick"
    );
    req.policy = Some(importance_policy(1.0, 0.0, None));
    let blended = svc
        .compile_context_reranked(&compiler, &req, &MarkerReranker("checksum"))
        .expect("compile reranked");

    // Then the blend composes with the seam: same pool, learned confidence
    // re-ranks inside it — one coherent ranking across every engine
    assert!(
        pos(&blended, "canary error budget") < pos(&blended, "checksum verification"),
        "the reinforced memory must lead inside the reranked pool, got:\n{}",
        blended.content
    );
}

#[test]
fn test_importance_out_of_range_weight_is_accepted_verbatim_not_clamped() {
    // Given the reinforced-twin scenario, but with a NEGATIVE confidence
    // weight — the documented (unclamped) inversion: demote reinforced facts
    let (_dir, svc) = service();
    let alpha = "postgres pool sizing guidance from runbook alpha";
    let beta = "postgres pool sizing guidance from runbook beta";
    let alpha_id = svc.remember(alpha, &[], None).expect("remember");
    let beta_id = svc.remember(beta, &[], None).expect("remember");

    let mut req = request(
        "postgres pool sizing",
        vec![fragment("Session note.")],
        10_000,
    );
    req.memory_scope = Some(MemoryScope {
        k: Some(2),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When the LEADING fact is the one the team reinforced
    req.policy = Some(importance_policy(0.0, 0.0, None));
    let baseline = svc.compile_context(&compiler, &req).expect("baseline");
    let (leading_text, leading_id, other) = if pos(&baseline, alpha) < pos(&baseline, beta) {
        (alpha, alpha_id, beta)
    } else {
        (beta, beta_id, alpha)
    };
    for _ in 0..15 {
        svc.feedback(leading_id, true).expect("feedback");
    }

    // And compiling with confidence weight -1.0 (outside the recommended
    // [0, 1] range — accepted verbatim, per the documented contract)
    req.policy = Some(importance_policy(-1.0, 0.0, None));
    let blended = svc.compile_context(&compiler, &req).expect("blended");

    // Then the term is inverted, not clamped to zero: the reinforced fact
    // is demoted behind its twin (a clamp to 0 would keep it leading)
    assert!(
        pos(&blended, other) < pos(&blended, leading_text),
        "a negative weight must invert the confidence term, got:\n{}",
        blended.content
    );
}

// --- Media source storage & screenshot supersession (US-009, PR2) ----------

#[test]
fn test_media_fragment_dropped_by_budget_round_trips_byte_identical_via_its_handle() {
    // Given a media fragment far too large for the budget, so it never
    // packs inline
    let (_dir, svc) = service();
    let frag = media_fragment("a huge screenshot", PNG_B64);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", vec![frag], 10))
        .expect("compile");
    let decision = &out.decisions[0];
    assert_eq!(decision.action, ContextAction::Retrieve);
    let handle = decision
        .handle
        .clone()
        .expect("externalized media gets a handle");

    // When retrieving the source behind its handle
    let source = svc.retrieve_context_source(&handle).expect("retrieve");

    // Then the caption AND the media (mime + bytes_b64) round-trip exactly
    assert_eq!(source.content, "a huge screenshot");
    let media = source
        .media
        .expect("a media source must carry its media back");
    assert_eq!(media.mime, "image/png");
    assert_eq!(media.bytes_b64, PNG_B64);
}

#[test]
fn test_media_fragment_source_round_trips_via_out_sources_handle_too() {
    // Given a media fragment that fits inline (still gets a source entry)
    let (_dir, svc) = service();
    let frag = media_fragment("caption", PNG_B64);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", vec![frag], 10_000))
        .expect("compile");

    // When retrieving via `out.sources[0].handle` (not just a drop/retrieve
    // decision's handle)
    let handle = &out.sources[0].handle;
    let source = svc.retrieve_context_source(handle).expect("retrieve");

    // Then the media round-trips byte for byte
    assert_eq!(source.content, "caption");
    assert_eq!(
        source
            .media
            .expect("media must be stored for every non-duplicate source")
            .bytes_b64,
        PNG_B64
    );
}

#[test]
fn test_text_only_source_still_carries_no_media_after_pr2() {
    // Byte-compat guard: a plain text fragment's stored source must still
    // carry no media field at all (never Some(default)).
    let (_dir, svc) = service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(
            &compiler,
            &request("q", vec![fragment("plain text, no media here")], 10_000),
        )
        .expect("compile");
    let source = svc
        .retrieve_context_source(&out.sources[0].handle)
        .expect("retrieve");
    assert_eq!(source.content, "plain text, no media here");
    assert!(source.media.is_none());
}

#[test]
fn test_three_screenshots_same_target_first_two_externalize_with_working_handles() {
    // Given three screenshots of the same target, compiled through the
    // bridge so store_sources actually persists them
    let (_dir, svc) = service();
    let fragments = vec![
        screenshot("v1", "login-page"),
        screenshot("v2", "login-page"),
        screenshot("v3", "login-page"),
    ];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then the first two are superseded, and each is genuinely retrievable
    // (not merely handed a dangling handle) with its OWN distinct bytes
    for (seq, caption) in [(0, "v1"), (1, "v2")] {
        let decision = &out.decisions[seq];
        assert_eq!(decision.action, ContextAction::Retrieve);
        assert_eq!(decision.rule_id, "retrieve.screenshot_superseded");
        let handle = decision
            .handle
            .clone()
            .expect("superseded screenshot gets a handle");
        let source = svc.retrieve_context_source(&handle).expect("retrieve");
        assert_eq!(
            source.media.expect("media must round-trip").bytes_b64,
            distinct_media_b64(caption)
        );
    }

    // And the last stays inline, never externalized
    assert_eq!(out.decisions[2].action, ContextAction::Preserve);
}

#[test]
fn test_screenshots_of_different_targets_all_round_trip_independently() {
    // Given screenshots of two different targets, both fitting the budget
    let (_dir, svc) = service();
    let fragments = vec![
        screenshot("login shot", "login-page"),
        screenshot("checkout shot", "checkout-page"),
    ];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then neither is superseded, and both stay inline
    assert!(out
        .decisions
        .iter()
        .all(|d| d.action == ContextAction::Preserve));
    assert!(out.content.contains("login shot"));
    assert!(out.content.contains("checkout shot"));
}

// --- Review probes (PR2 cross-review): media identity must be the BYTES ----
//
// Media dedup already keys on the raw decoded bytes (PR1); handles and
// storage slots must key on the same identity, or two media fragments with
// identical (typically blank) captions but different bytes collide onto one
// handle and one slot — and "externalized behind a resolvable handle"
// silently serves the wrong image in the nominal captionless case.

#[test]
fn test_two_blank_caption_media_fragments_with_different_bytes_get_distinct_resolving_handles() {
    // P1 — same compile: two media fragments, both captions BLANK, bytes
    // A != B, budget too small for either (the 64x48 fixture image costs 5
    // tokens), so both externalize
    let (_dir, svc) = service();
    let bytes_a = distinct_media_b64("bytes-A");
    let bytes_b = distinct_media_b64("bytes-B");
    assert_ne!(bytes_a, bytes_b, "fixture must yield distinct bytes");
    let fragments = vec![media_fragment("", &bytes_a), media_fragment("", &bytes_b)];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 4))
        .expect("compile");

    // Then the two handles are DISTINCT ...
    let handle_a = out.decisions[0].handle.clone().expect("A externalized");
    let handle_b = out.decisions[1].handle.clone().expect("B externalized");
    assert_ne!(
        handle_a, handle_b,
        "different bytes must never share a handle, blank captions or not"
    );

    // ... and each resolves ITS OWN bytes
    let source_a = svc.retrieve_context_source(&handle_a).expect("retrieve A");
    let source_b = svc.retrieve_context_source(&handle_b).expect("retrieve B");
    assert_eq!(source_a.media.expect("A media").bytes_b64, bytes_a);
    assert_eq!(source_b.media.expect("B media").bytes_b64, bytes_b);
}

#[test]
fn test_cross_compile_blank_caption_media_sources_never_serve_stale_bytes() {
    // P2 — cross-compile: compile blank-caption image A first (its source is
    // stored), then blank-caption image B in a SECOND compile
    let (_dir, svc) = service();
    let bytes_a = distinct_media_b64("stale-A");
    let bytes_b = distinct_media_b64("fresh-B");
    assert_ne!(bytes_a, bytes_b, "fixture must yield distinct bytes");
    let compiler = ContextCompiler::new(CompilePolicy::default());
    svc.compile_context(
        &compiler,
        &request("q", vec![media_fragment("", &bytes_a)], 10_000),
    )
    .expect("compile A");
    let out_b = svc
        .compile_context(
            &compiler,
            &request("q", vec![media_fragment("", &bytes_b)], 10_000),
        )
        .expect("compile B");

    // When resolving B's handle from the second compile
    let handle_b = out_b.sources[0].handle.clone();
    let source_b = svc.retrieve_context_source(&handle_b).expect("retrieve B");

    // Then B's own bytes come back — never A's, stored earlier at what must
    // NOT be the same slot
    assert_eq!(
        source_b.media.expect("B media").bytes_b64,
        bytes_b,
        "an occupied slot from an earlier compile must never serve stale bytes for a new handle"
    );
}

#[test]
fn test_superseded_blank_caption_screenshots_resolve_their_own_bytes_not_the_survivors() {
    // P3 — supersession: three screenshots of the same target, ALL captions
    // blank, three different byte payloads; the first two are superseded
    let (_dir, svc) = service();
    let seeds = ["shot-1", "shot-2", "shot-3"];
    let fragments: Vec<ContextFragment> = seeds
        .iter()
        .map(|seed| screenshot_with_seed("", "login-page", seed))
        .collect();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then the two superseded screenshots carry DISTINCT handles, each
    // resolving its own bytes — never the surviving third's
    let handle_0 = out.decisions[0].handle.clone().expect("superseded 0");
    let handle_1 = out.decisions[1].handle.clone().expect("superseded 1");
    assert_ne!(handle_0, handle_1, "distinct bytes, distinct handles");
    for (handle, seed) in [(handle_0, seeds[0]), (handle_1, seeds[1])] {
        let source = svc.retrieve_context_source(&handle).expect("retrieve");
        assert_eq!(
            source.media.expect("media").bytes_b64,
            distinct_media_b64(seed),
            "a superseded screenshot's handle must resolve ITS OWN bytes, not the survivor's"
        );
    }
    assert_eq!(out.decisions[2].action, ContextAction::Preserve);
}

// --- Review minors: dedup × supersession, and recall invisibility ----------

#[test]
fn test_byte_identical_screenshot_duplicate_wins_over_supersession_and_resolves() {
    // Given two BYTE-IDENTICAL screenshots of a target plus a newer,
    // different one — dedup and supersession both apply to fragment #1;
    // the dup arm must win (it is checked first in `decision`)
    let (_dir, svc) = service();
    let same = distinct_media_b64("same-bytes");
    let fragments = vec![
        screenshot_with_seed("", "login-page", "same-bytes"),
        screenshot_with_seed("", "login-page", "same-bytes"),
        screenshot_with_seed("", "login-page", "newer"),
    ];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then #0 is superseded (older of the series), #1 is its exact
    // duplicate (dup verdict wins over the supersession flag), #2 survives
    assert_eq!(out.decisions[0].rule_id, "retrieve.screenshot_superseded");
    assert_eq!(out.decisions[1].rule_id, "drop.duplicate");
    assert_eq!(out.decisions[1].action, ContextAction::Drop);
    assert_eq!(out.decisions[2].action, ContextAction::Preserve);

    // And the duplicate's handle (same bytes as #0) still resolves those bytes
    let dup_handle = out.decisions[1]
        .handle
        .clone()
        .expect("media duplicate keeps a handle");
    let source = svc.retrieve_context_source(&dup_handle).expect("retrieve");
    assert_eq!(source.media.expect("media").bytes_b64, same);
}

#[test]
fn test_two_identical_screenshots_same_target_emits_latest() {
    // Given two BYTE-IDENTICAL screenshots of the same target — dedup
    // (anchored on #0, the first occurrence) and supersession (which keeps
    // only the LAST occurrence, #1) disagree on which one survives. Without
    // re-anchoring, #0 is superseded (dropped) and #1 is "just a duplicate
    // of #0" (also dropped) — the image vanishes from the compiled output
    // entirely, contradicting US-009's "the freshest copy stays inline".
    let (_dir, svc) = service();
    let fragments = vec![
        screenshot("dup shot", "login-page"),
        screenshot("dup shot", "login-page"),
    ];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then the image survives, exactly once, through the LATEST occurrence
    assert_eq!(
        out.content.matches("dup shot").count(),
        1,
        "the image must appear exactly once in the compiled content"
    );
    assert_eq!(out.decisions[0].action, ContextAction::Drop);
    assert_eq!(out.decisions[0].rule_id, "drop.duplicate");
    assert!(
        out.decisions[0]
            .reason
            .contains("of fragment #1"),
        "the stale fragment #0 must be recorded as a duplicate of the LATEST occurrence (#1), got: {}",
        out.decisions[0].reason
    );
    assert_eq!(out.decisions[1].action, ContextAction::Preserve);

    // And dropping the stale twin never registers as a High-fidelity-risk
    // loss — the content it carried fully survives through #1
    assert_ne!(
        out.risk,
        velesdb_memory::context::FidelityRisk::High,
        "the surviving twin carries the same bytes, so risk must not reach High"
    );
}

#[test]
fn test_three_identical_screenshots_reanchors_on_last() {
    // Given three BYTE-IDENTICAL screenshots of the same target — the media
    // dedup namespace must chain #0 and #1 as duplicates of the final
    // survivor (#2), never anchor on a superseded fragment.
    let (_dir, svc) = service();
    let fragments = vec![
        screenshot("dup shot", "login-page"),
        screenshot("dup shot", "login-page"),
        screenshot("dup shot", "login-page"),
    ];
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request("q", fragments, 10_000))
        .expect("compile");

    // Then the image survives exactly once, and only the last fragment
    // stays inline
    assert_eq!(
        out.content.matches("dup shot").count(),
        1,
        "the image must appear exactly once in the compiled content"
    );
    assert_eq!(out.decisions[0].action, ContextAction::Drop);
    assert_eq!(out.decisions[1].action, ContextAction::Drop);
    assert_eq!(out.decisions[2].action, ContextAction::Preserve);
}

#[test]
fn test_stored_media_sources_are_invisible_to_normal_recall() {
    // Given a compiled (and stored) media source with a distinctive caption
    let (_dir, svc) = service();
    let caption = "zebra quantum flamingo caption";
    let frag = media_fragment(caption, &distinct_media_b64("invisible"));
    let compiler = ContextCompiler::new(CompilePolicy::default());
    svc.compile_context(&compiler, &request("q", vec![frag], 10_000))
        .expect("compile");

    // When recalling with the caption itself as the query
    let hits = svc.recall(caption, 10, None).expect("recall");

    // Then the media source system fact (hub-marked, placeholder-embedded)
    // never surfaces in caller-facing recall
    assert!(
        hits.is_empty(),
        "a stored media source must be invisible to normal recall, got {hits:?}"
    );
}

// --- V2a-2: warnings[] and slim_response ------------------------------------

#[test]
fn test_compile_context_externalized_relevant_fragment_produces_a_warning() {
    // Given a fragment that matches the query but cannot fit the budget at
    // all (a tiny cache-marked filler eats the whole tiny budget first)
    let (_dir, svc) = service();
    let filler = fragment("filler");
    let relevant = fragment(
        "the deploy pipeline runs clippy before every merge across the whole fleet nightly",
    );
    let req = request("deploy pipeline", vec![filler, relevant], 3);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Sanity: the relevant fragment was indeed externalized, not packed.
    let retrieve_decision = out
        .decisions
        .iter()
        .find(|d| d.action == ContextAction::Retrieve)
        .expect("the relevant fragment must not fit this tiny budget");
    assert!(retrieve_decision.relevance >= 0.35);

    // Then it shows up in `warnings` — a mechanical signal the skill can
    // check without scanning every decision by hand.
    assert!(
        out.warnings
            .iter()
            .any(|w| w.fragment_id == retrieve_decision.fragment_id),
        "an externalized, relevant fragment must produce a warning: {:?}",
        out.warnings
    );
}

#[test]
fn test_compile_context_externalized_irrelevant_fragment_has_no_warning() {
    // Given a fragment externalized by budget but sharing NO query terms
    let (_dir, svc) = service();
    let filler = fragment("filler");
    let irrelevant = fragment(
        "unrelated prose about houseplants and weekend gardening that goes on for a while",
    );
    let req = request("deploy pipeline", vec![filler, irrelevant], 3);
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    let retrieve_decision = out
        .decisions
        .iter()
        .find(|d| d.action == ContextAction::Retrieve)
        .expect("the irrelevant fragment must not fit this tiny budget");
    assert!(retrieve_decision.relevance < 0.35);

    // Then it does NOT warn — below the relevance threshold, warnings would
    // just be noise.
    assert!(
        out.warnings.is_empty(),
        "a below-threshold externalized fragment must not warn: {:?}",
        out.warnings
    );
}

#[test]
fn test_compile_context_duplicate_drop_never_warns() {
    // Given two byte-identical fragments (a Drop decision, but the content
    // survives through the kept twin — nothing is actually lost)
    let (_dir, svc) = service();
    let content = "the deploy pipeline runs clippy before tests";
    let req = request(
        "deploy pipeline",
        vec![fragment(content), fragment(content)],
        10_000,
    );
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc.compile_context(&compiler, &req).expect("compile");

    assert!(out
        .decisions
        .iter()
        .any(|d| d.action == ContextAction::Drop));
    // Then `warnings` stays empty: a duplicate whose content survives
    // elsewhere in the output is not a loss worth flagging.
    assert!(
        out.warnings.is_empty(),
        "a safe duplicate drop must never warn: {:?}",
        out.warnings
    );
}

#[test]
fn test_compile_context_slim_response_clears_sections_and_decisions_keeps_content() {
    // Given a normal (non-slim) compile of a request
    let (_dir, svc) = service();
    let req = request(
        "deploy pipeline",
        vec![fragment("the deploy pipeline runs clippy before tests")],
        10_000,
    );
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let full = svc.compile_context(&compiler, &req).expect("full compile");

    // When compiling the same request with slim_response
    let mut slim_req = req.clone();
    slim_req.policy = Some(CompilePolicy {
        slim_response: true,
        ..CompilePolicy::default()
    });
    let slim = svc
        .compile_context(&compiler, &slim_req)
        .expect("slim compile");

    // Then content/insights/risk/warnings/handles are preserved byte-for-byte...
    assert_eq!(slim.content, full.content);
    assert_eq!(slim.insights.tokens_in, full.insights.tokens_in);
    assert_eq!(slim.risk, full.risk);
    assert_eq!(slim.warnings.len(), full.warnings.len());
    // ...but sections and decisions are emptied out.
    assert!(slim.sections.is_empty(), "{:?}", slim.sections);
    assert!(slim.decisions.is_empty(), "{:?}", slim.decisions);
    assert!(!full.sections.is_empty());
    assert!(!full.decisions.is_empty());
}

#[test]
fn test_compile_context_slim_response_defaults_to_false() {
    // Given a request with no explicit slim_response
    let (_dir, svc) = service();
    let req = request(
        "deploy pipeline",
        vec![fragment("the deploy pipeline runs clippy before tests")],
        10_000,
    );
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then sections/decisions are present — the default is unchanged.
    assert!(!out.sections.is_empty());
    assert!(!out.decisions.is_empty());
}

#[test]
fn test_compile_context_memory_pulled_warning_reflects_post_annotation_relevance() {
    // Given a remembered fact that will be pulled in via memory_scope AND
    // externalized (tiny budget), whose relevance only becomes final after
    // the bridge's memory-provenance annotation runs (it can rewrite
    // `relevance` — see `annotate_memory_provenance`)
    let (_dir, svc) = service();
    svc.remember(
        "the deploy pipeline runs clippy before every merge across the fleet",
        &[],
        None,
    )
    .expect("remember");
    let mut req = request("deploy pipeline", vec![fragment("filler")], 3);
    req.memory_scope = Some(MemoryScope {
        k: Some(1),
        ..MemoryScope::default()
    });
    let compiler = ContextCompiler::new(CompilePolicy::default());

    // When compiling through the bridge
    let out = svc.compile_context(&compiler, &req).expect("compile");

    // Then any warning's relevance/reason matches what's actually in
    // `decisions` (post-annotation), never the pre-annotation snapshot.
    for warning in &out.warnings {
        let decision = out
            .decisions
            .iter()
            .find(|d| d.fragment_id == warning.fragment_id)
            .expect("every warning must point at a real decision");
        // Bit-exact: a warning copies its decision's relevance verbatim, so
        // compare bits (satisfies clippy::float_cmp, tests the real invariant).
        assert_eq!(warning.relevance.to_bits(), decision.relevance.to_bits());
        assert_eq!(warning.reason, decision.reason);
    }
}
