//! Unit tests for the source writer's never-downgrade TTL upgrade rule and
//! its squatter guard.
//!
//! These live here (rather than in the `tests/` integration suite) so the
//! never-downgrade assertions can read the reserved `_veles_expires_at`
//! metadata directly instead of sleeping past a real TTL and retrying
//! retrieval: a sleep-based version of these tests was flaky under the full
//! suite's parallel test load (a compile occasionally landed close enough to
//! a 1-second TTL boundary that a 1.5s sleep wasn't reliably past it), and
//! reading the metadata is both deterministic and faster. The squatter guard
//! is a unit test for a second, independent reason: forging an unmarked
//! occupied slot needs `self.store` (a private field) to write directly at
//! the exact salted `source_id` the bridge would use — unreachable from an
//! integration test, and unreachable through the public API too (a fact's id
//! is `stable_id(fact)`, not caller-chosen, so colliding it with a specific
//! `source_id` is an infeasible preimage search, not a realistic fixture).

use super::*;
use crate::context::model::CompilePolicy;
use crate::context::{fragment_id, ContextAction};
use crate::embedder::HashEmbedder;

const DIM: usize = 384;

fn open_service() -> (tempfile::TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open memory store");
    (dir, svc)
}

fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        path: None,
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

fn request(content: &str, policy: CompilePolicy) -> CompileRequest {
    CompileRequest {
        query: "q".to_owned(),
        fragments: vec![fragment(content)],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: Some(policy),
    }
}

/// The slot a compiled source's handle resolves to.
fn slot_of(handle: &str) -> u64 {
    let hash = provenance::parse_handle(handle).expect("well-formed ctx://source handle");
    source_id(hash)
}

#[test]
fn test_permanent_compile_upgrades_ttl_slot_to_permanent() {
    // Given a compile that stores the source under a short-lived TTL
    let (_dir, svc) = open_service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let content = "must be upgraded to permanent, not left to expire";

    let ttl_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(60),
            ..CompilePolicy::default()
        },
    );
    let out = svc
        .compile_context(&compiler, &ttl_req)
        .expect("compile ttl");
    let slot = slot_of(&out.sources[0].handle);
    let meta = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("marked as a stored source");
    assert!(
        meta.contains_key(EXPIRES_AT_FIELD),
        "sanity: the first compile must carry a TTL"
    );

    // When a later compile of the SAME content asks for permanent storage
    let permanent_req = request(content, CompilePolicy::default());
    svc.compile_context(&compiler, &permanent_req)
        .expect("compile permanent");

    // Then the slot's durable expiry must be gone — upgraded to permanent.
    let meta_after = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("still marked as a stored source");
    assert!(
        !meta_after.contains_key(EXPIRES_AT_FIELD),
        "a later permanent compile must upgrade an existing TTL slot to \
         permanent, not leave it to expire silently: {meta_after:?}"
    );
}

#[test]
fn test_ttl_compile_never_downgrades_permanent_slot() {
    // Given a compile that stores the source permanently
    let (_dir, svc) = open_service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let content = "must stay permanent even after a later short-TTL compile";

    let permanent_req = request(content, CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &permanent_req)
        .expect("compile permanent");
    let slot = slot_of(&out.sources[0].handle);
    let meta = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("marked as a stored source");
    assert!(
        !meta.contains_key(EXPIRES_AT_FIELD),
        "sanity: the first compile must be permanent"
    );

    // When a later compile of the SAME content asks for a short TTL
    let ttl_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(60),
            ..CompilePolicy::default()
        },
    );
    svc.compile_context(&compiler, &ttl_req)
        .expect("compile ttl");

    // Then the slot must still be permanent — never downgraded.
    let meta_after = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("still marked as a stored source");
    assert!(
        !meta_after.contains_key(EXPIRES_AT_FIELD),
        "a later TTL compile must never downgrade an existing permanent slot: {meta_after:?}"
    );
}

#[test]
fn test_ttl_extension_only_never_shrinks_a_longer_ttl() {
    // Given a compile with a long TTL, then a later one with a shorter TTL
    let (_dir, svc) = open_service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let content = "extension-only never shrinks below the longer TTL";

    let long_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(3600),
            ..CompilePolicy::default()
        },
    );
    let out = svc
        .compile_context(&compiler, &long_req)
        .expect("compile long ttl");
    let slot = slot_of(&out.sources[0].handle);
    let long_expiry = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("marked")
        .get(EXPIRES_AT_FIELD)
        .and_then(Value::as_u64)
        .expect("the long-TTL compile must set an expiry");

    // When a later compile of the SAME content requests a much shorter TTL
    let short_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(60),
            ..CompilePolicy::default()
        },
    );
    svc.compile_context(&compiler, &short_req)
        .expect("compile shorter ttl");

    // Then the expiry must be unchanged — never shrunk.
    let expiry_after = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("still marked")
        .get(EXPIRES_AT_FIELD)
        .and_then(Value::as_u64)
        .expect("still carries an expiry");
    assert_eq!(
        expiry_after, long_expiry,
        "a later shorter-TTL compile must never shrink an existing longer TTL"
    );
}

#[test]
fn test_ttl_extension_only_extends_a_shorter_ttl() {
    // Given a compile with a short TTL, then a later one with a longer TTL
    let (_dir, svc) = open_service();
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let content = "extension-only extends past a shorter original TTL";

    let short_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(60),
            ..CompilePolicy::default()
        },
    );
    let out = svc
        .compile_context(&compiler, &short_req)
        .expect("compile shorter ttl");
    let slot = slot_of(&out.sources[0].handle);
    let short_expiry = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("marked")
        .get(EXPIRES_AT_FIELD)
        .and_then(Value::as_u64)
        .expect("the short-TTL compile must set an expiry");

    // When a later compile of the SAME content requests a much longer TTL
    let long_req = request(
        content,
        CompilePolicy {
            source_ttl_seconds: Some(3600),
            ..CompilePolicy::default()
        },
    );
    svc.compile_context(&compiler, &long_req)
        .expect("compile longer ttl");

    // Then the expiry must have moved further out — extended, not left as-is.
    let expiry_after = svc
        .context_source_metadata(slot)
        .expect("meta lookup")
        .expect("still marked")
        .get(EXPIRES_AT_FIELD)
        .and_then(Value::as_u64)
        .expect("still carries an expiry");
    assert!(
        expiry_after > short_expiry,
        "a later longer-TTL compile must extend an existing shorter TTL \
         (before={short_expiry}, after={expiry_after})"
    );
}

#[test]
fn test_load_working_context_never_serves_an_unmarked_squatter() {
    // Given a slot occupied by a caller fact that carries none of the
    // bridge's `_veles_ctx_working` marker (forged directly via the store —
    // the working-context slot is salted/deterministic, so this can't be
    // reached through the public API: a real fact's id is `stable_id(fact)`,
    // not caller-chosen)
    let (_dir, svc) = open_service();
    let project = "veles";
    let session = "forged-session";
    let slot = working_id(project, session);
    let forged_content = "{\"goal\":\"forged working state\"}";
    let embedding = svc.embedder.embed(forged_content).expect("embed");
    svc.store
        .store(slot, forged_content, &embedding)
        .expect("forge an unmarked squatter at the exact working-context slot");

    // When a later session tries to load the working context
    let loaded = svc
        .load_working_context(project, session)
        .expect("load must not error on a squatted slot");

    // Then it must never see the forged content — indistinguishable from no
    // working context ever having been saved.
    assert!(
        loaded.is_none(),
        "an unmarked occupied working-context slot must never be served back: {loaded:?}"
    );
}

#[test]
fn test_list_working_contexts_returns_sessions_saved_under_a_project() {
    // Given two sessions saved under the same project
    let (_dir, svc) = open_service();
    svc.save_working_context("veles", "session-a", &WorkingContext::default())
        .expect("save session-a");
    svc.save_working_context("veles", "session-b", &WorkingContext::default())
        .expect("save session-b");

    // When listing the project's working contexts
    let sessions = svc
        .list_working_contexts("veles")
        .expect("list_working_contexts");

    // Then both sessions are reported, each with a saved_at.
    let names: Vec<&str> = sessions.iter().map(|s| s.session.as_str()).collect();
    assert!(names.contains(&"session-a"), "{names:?}");
    assert!(names.contains(&"session-b"), "{names:?}");
}

#[test]
fn test_list_working_contexts_empty_for_unknown_project() {
    // Given a store with nothing saved
    let (_dir, svc) = open_service();

    // When listing a project that never saved anything
    let sessions = svc
        .list_working_contexts("never-used-project")
        .expect("list_working_contexts must not error on an empty index");

    // Then the list is empty, not an error.
    assert!(sessions.is_empty());
}

#[test]
fn test_list_working_contexts_resaving_same_session_updates_saved_at_not_duplicates() {
    // Given a session saved twice under the same project+session
    let (_dir, svc) = open_service();
    svc.save_working_context("veles", "session-a", &WorkingContext::default())
        .expect("save first");
    let first_at = svc
        .list_working_contexts("veles")
        .expect("list")
        .into_iter()
        .find(|s| s.session == "session-a")
        .expect("session-a present")
        .saved_at;

    std::thread::sleep(std::time::Duration::from_millis(1100));
    svc.save_working_context("veles", "session-a", &WorkingContext::default())
        .expect("save again");

    // When listing again
    let sessions = svc
        .list_working_contexts("veles")
        .expect("list_working_contexts");

    // Then there is still exactly one entry for that session, with an
    // updated saved_at.
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| s.session == "session-a")
        .collect();
    assert_eq!(matches.len(), 1, "must not duplicate: {sessions:?}");
    assert!(
        matches[0].saved_at >= first_at,
        "saved_at must advance on resave"
    );
}

#[test]
fn test_should_store_source_never_rewrites_an_unmarked_occupied_slot() {
    // Given a slot occupied by a caller fact that carries none of the
    // bridge's `_veles_ctx_source` marker (forged directly via the store —
    // see module doc for why this can't be reached through `remember`)
    let (_dir, svc) = open_service();

    let probe = fragment("squatter probe content");
    let slot = source_id(fragment_handle_hash(&probe));
    let embedding = svc
        .embedder
        .embed("an unrelated caller fact")
        .expect("embed");
    svc.store
        .store(slot, "an unrelated caller fact", &embedding)
        .expect("forge an unmarked squatter at the exact slot");

    // When the source writer is asked whether it should (re-)write that slot
    let should_store = svc
        .should_store_source(slot, None)
        .expect("should_store_source must not error on a squatted slot");

    // Then it must refuse — an unmarked occupied slot is a caller fact, and
    // clobbering it would destroy user data.
    assert!(
        !should_store,
        "an unmarked occupied slot must never be (re-)written by the source writer"
    );
}

// --- explain_compilation: extracted selection primitive (V2d-2) -----------
// The MCP `explain_compilation` tool's selection logic (record-off recompile
// + select-by-index/id) lives here now, so every adapter (MCP, Node, Python)
// shares one implementation. These tests pin the primitive's own contract
// directly, independent of the MCP wire layer.

#[test]
fn test_explain_compilation_returns_the_decision_for_a_matching_fragment_id() {
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let req = CompileRequest {
        query: "deploy".to_owned(),
        fragments: vec![fragment("a fact"), fragment("other")],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    };

    let decision = svc
        .explain_compilation(&req, wanted, None)
        .expect("explain_compilation");

    assert_eq!(decision.fragment_id, wanted);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert!(!decision.reason.is_empty());
}

#[test]
fn test_explain_compilation_unknown_fragment_id_is_fragment_not_found() {
    let (_dir, svc) = open_service();
    let req = CompileRequest {
        query: "deploy".to_owned(),
        fragments: vec![fragment("a fact")],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    };

    let err = svc
        .explain_compilation(&req, 424_242, None)
        .expect_err("no such fragment in the request — must fail");

    assert!(matches!(err, MemoryError::FragmentNotFound(424_242)));
}

#[test]
fn test_explain_compilation_fragment_index_out_of_bounds_is_rejected() {
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let req = CompileRequest {
        query: "deploy".to_owned(),
        fragments: vec![fragment("a fact")],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    };

    let err = svc
        .explain_compilation(&req, wanted, Some(5))
        .expect_err("fragment_index 5 has no fragment — must fail");

    assert!(matches!(
        err,
        MemoryError::FragmentIndexOutOfBounds { index: 5, len: 1 }
    ));
}

#[test]
fn test_explain_compilation_fragment_index_disambiguates_byte_identical_twins() {
    // Two byte-identical fragments share the same content-addressed
    // fragment_id — the id-only lookup always resolves to the
    // deduplication survivor (kept, Preserve), never a dropped twin's own
    // decision. `fragment_index` picks the SECOND fragment's decision.
    let (_dir, svc) = open_service();
    let shared_id = fragment_id("duplicate payload");
    let req = CompileRequest {
        query: "deploy".to_owned(),
        fragments: vec![fragment("duplicate payload"), fragment("duplicate payload")],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    };

    let survivor = svc
        .explain_compilation(&req, shared_id, None)
        .expect("explain_compilation (by id)");
    let twin = svc
        .explain_compilation(&req, shared_id, Some(1))
        .expect("explain_compilation (by index)");

    assert!(matches!(survivor.action, ContextAction::Preserve));
    assert!(matches!(twin.action, ContextAction::Drop));
    assert_eq!(twin.rule_id, "drop.duplicate");
    assert_eq!(twin.fragment_id, shared_id);
}

#[test]
fn test_explain_compilation_never_records_an_event_or_stores_a_source() {
    // An explanation is a read-only question about a deterministic
    // function: it must not leave side effects behind, even when the
    // request's own policy asked for them.
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let req = CompileRequest {
        query: "deploy".to_owned(),
        fragments: vec![fragment("a fact")],
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: Some(CompilePolicy {
            record_events: true,
            store_sources: true,
            ..CompilePolicy::default()
        }),
    };

    svc.explain_compilation(&req, wanted, None)
        .expect("explain_compilation");

    let savings = svc.context_savings(None).expect("context_savings");
    assert_eq!(
        savings.events, 0,
        "explain_compilation must not record a compile event"
    );
}
