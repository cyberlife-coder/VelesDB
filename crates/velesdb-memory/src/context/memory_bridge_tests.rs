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

use super::compile::source_id;
use super::*;
use crate::context::model::CompilePolicy;
use crate::context::{fragment_id, ContextAction};
use crate::embedder::HashEmbedder;

use crate::storage::NativeStore;

const DIM: usize = 384;

fn open_service() -> (tempfile::TempDir, MemoryService<HashEmbedder>) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open memory store");
    (dir, svc)
}

/// The smallest working context a save accepts. An entirely empty one is
/// refused (it would wipe whatever the same project+session already holds),
/// and these tests are about the index and the slot, not the payload — so
/// they carry the least content that gets past that guard.
fn minimal_working() -> WorkingContext {
    WorkingContext {
        goal: Some("resume this session".to_owned()),
        ..WorkingContext::default()
    }
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

fn explain_request(
    fragments: Vec<ContextFragment>,
    policy: Option<CompilePolicy>,
) -> CompileRequest {
    CompileRequest {
        query: "deploy".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy,
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
    svc.save_working_context("veles", "session-a", &minimal_working())
        .expect("save session-a");
    svc.save_working_context("veles", "session-b", &minimal_working())
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
fn test_bound_sessions_keeps_cap_entries_newest_first_and_pins_the_one_just_saved() {
    use crate::limits::MAX_WORKING_SESSIONS_PER_PROJECT as CAP;

    // Given CAP + 1 entries that all share one saved_at — the same-second
    // flood where ordering cannot lean on the timestamp — and the session
    // just saved sorting LAST by name, so an unpinned truncate would drop it
    let mut sessions: Vec<WorkingContextSession> = (0..=CAP)
        .map(|i| WorkingContextSession {
            session: format!("session-{i:05}"),
            saved_at: 1_700_000_000,
        })
        .collect();
    let just_saved = format!("session-{CAP:05}");

    // When bounding
    bound_sessions(&mut sessions, &just_saved);

    // Then exactly CAP survive, the just-saved one among them, and the one
    // that left is the last by name among the unpinned tie.
    assert_eq!(sessions.len(), CAP);
    assert!(
        sessions.iter().any(|s| s.session == just_saved),
        "just-saved evicted"
    );
    let evicted = format!("session-{:05}", CAP - 1);
    assert!(
        sessions.iter().all(|s| s.session != evicted),
        "the last unpinned tie-loser must be the one evicted"
    );
}

#[test]
fn test_bound_sessions_evicts_the_oldest_saved_first_and_is_a_no_op_under_the_cap() {
    use crate::limits::MAX_WORKING_SESSIONS_PER_PROJECT as CAP;

    // Under the cap nothing moves — not even the order.
    let mut few: Vec<WorkingContextSession> = (0..3)
        .map(|i| WorkingContextSession {
            session: format!("s{i}"),
            saved_at: 10 - i,
        })
        .collect();
    let before: Vec<(String, u64)> = few
        .iter()
        .map(|s| (s.session.clone(), s.saved_at))
        .collect();
    bound_sessions(&mut few, "s0");
    let after: Vec<(String, u64)> = few
        .iter()
        .map(|s| (s.session.clone(), s.saved_at))
        .collect();
    assert_eq!(after, before);

    // Over it, the oldest saved_at leaves regardless of name.
    let mut many: Vec<WorkingContextSession> = (0..=CAP)
        .map(|i| WorkingContextSession {
            session: format!("s{i}"),
            saved_at: 1_000 + i as u64,
        })
        .collect();
    bound_sessions(&mut many, "s5");
    assert_eq!(many.len(), CAP);
    assert!(
        many.iter().all(|s| s.session != "s0"),
        "oldest (s0) must be evicted"
    );
    assert!(many.iter().any(|s| s.session == "s5"));
}

#[test]
fn test_save_working_context_refuses_an_empty_project_or_session_key() {
    // Given a store; an empty or whitespace-only key would hash, encode and
    // list just fine — as "" — and be unrecoverable by anyone who does not
    // think to ask for the empty string.
    let (_dir, svc) = open_service();
    for (project, session, key) in [
        ("", "session-a", "project"),
        ("   ", "session-a", "project"),
        ("veles", "", "session"),
        ("veles", "\t\n", "session"),
    ] {
        // When saving under it
        let err = svc
            .save_working_context(project, session, &minimal_working())
            .expect_err("an empty key must be refused");

        // Then it is the key that is named, and nothing was written.
        assert!(
            matches!(err, MemoryError::EmptyWorkingContextKey { key: k } if k == key),
            "{project:?}/{session:?}: {err:?}"
        );
    }
    assert!(svc.list_working_contexts("veles").expect("list").is_empty());
    assert!(svc.list_working_contexts("").expect("list").is_empty());
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
    svc.save_working_context("veles", "session-a", &minimal_working())
        .expect("save first");
    let first_at = svc
        .list_working_contexts("veles")
        .expect("list")
        .into_iter()
        .find(|s| s.session == "session-a")
        .expect("session-a present")
        .saved_at;

    std::thread::sleep(std::time::Duration::from_millis(1100));
    svc.save_working_context("veles", "session-a", &minimal_working())
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
    let req = explain_request(vec![fragment("a fact"), fragment("other")], None);

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
    let req = explain_request(vec![fragment("a fact")], None);

    let err = svc
        .explain_compilation(&req, 424_242, None)
        .expect_err("no such fragment in the request — must fail");

    assert!(matches!(err, MemoryError::FragmentNotFound(424_242)));
}

#[test]
fn test_explain_compilation_fragment_index_out_of_bounds_is_rejected() {
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let req = explain_request(vec![fragment("a fact")], None);

    let err = svc
        .explain_compilation(&req, wanted, Some(5))
        .expect_err("fragment_index 5 has no fragment — must fail");

    assert!(matches!(
        err,
        MemoryError::FragmentIndexOutOfBounds { index: 5, len: 1 }
    ));
}

/// #1745. `slim_response` empties `sections` and `decisions` from a
/// `compile_context` response — a PRESENTATION option, for a caller who wants
/// the compiled content without its audit trail.
///
/// `explain_compilation` asks for exactly ONE decision. Letting that option
/// through does not trim its answer, it deletes it: `apply_slim` clears
/// `decisions`, the lookup then finds nothing, and the caller gets
/// `FragmentNotFound` for a fragment that was compiled perfectly well.
///
/// The message is what makes this worse than a limitation. A caller told
/// "fragment not found" reasonably concludes they passed the wrong id and
/// goes looking in the wrong place.
///
/// And the option exists to SAVE TOKENS, so a caller under a tight budget
/// turns it on by default — losing the audit tool exactly when they most need
/// it.
#[test]
fn test_explain_compilation_ignores_slim_response() {
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let slim = CompilePolicy {
        slim_response: true,
        ..CompilePolicy::default()
    };
    let req = explain_request(vec![fragment("a fact"), fragment("other")], Some(slim));

    let decision = svc
        .explain_compilation(&req, wanted, None)
        .expect("slim_response is a presentation option; it must not hide the decision");

    // Not merely "no error": the decision must be the one that was ASKED for,
    // and it must carry the explanation. An empty reason or a neighbour's
    // decision would satisfy `is_ok()` while telling the caller nothing.
    assert_eq!(
        decision.fragment_id, wanted,
        "the decision must belong to the fragment that was asked about"
    );
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert!(
        !decision.reason.is_empty(),
        "an explanation with no reason explains nothing"
    );
    assert!(
        !decision.rule_id.is_empty(),
        "the rule that produced the decision is half the explanation"
    );
}

/// The other half of the same guard. The fix works by FORCING a policy field,
/// so this pins that the nominal path — the one every caller already uses —
/// is untouched: same decision, same action, same rule, whether the option is
/// absent or explicitly off.
#[test]
fn test_explain_compilation_without_slim_response_is_unchanged() {
    let (_dir, svc) = open_service();
    let wanted = fragment_id("a fact");
    let explicit_off = CompilePolicy {
        slim_response: false,
        ..CompilePolicy::default()
    };

    let baseline = svc
        .explain_compilation(
            &explain_request(vec![fragment("a fact"), fragment("other")], None),
            wanted,
            None,
        )
        .expect("explain_compilation with no policy at all");
    let with_flag_off = svc
        .explain_compilation(
            &explain_request(
                vec![fragment("a fact"), fragment("other")],
                Some(explicit_off),
            ),
            wanted,
            None,
        )
        .expect("explain_compilation with slim_response explicitly false");

    assert_eq!(with_flag_off.fragment_id, baseline.fragment_id);
    assert_eq!(with_flag_off.action, baseline.action);
    assert_eq!(with_flag_off.rule_id, baseline.rule_id);
    assert_eq!(with_flag_off.reason, baseline.reason);
}

#[test]
fn test_explain_compilation_fragment_index_disambiguates_byte_identical_twins() {
    // Two byte-identical fragments share the same content-addressed
    // fragment_id — the id-only lookup always resolves to the
    // deduplication survivor (kept, Preserve), never a dropped twin's own
    // decision. `fragment_index` picks the SECOND fragment's decision.
    let (_dir, svc) = open_service();
    let shared_id = fragment_id("duplicate payload");
    let req = explain_request(
        vec![fragment("duplicate payload"), fragment("duplicate payload")],
        None,
    );

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

// ---------------------------------------------------------------------------
// Working-context index: concurrency (B3)
// ---------------------------------------------------------------------------

/// A [`NativeStore`] decorator that forces the exact interleaving the shared
/// working-context index is vulnerable to, instead of hoping a thread race
/// reproduces it: the FIRST two readers of `project`'s index slot are held at
/// a rendezvous until both have read, so both observe the same pre-state and
/// the second writer overwrites the first.
///
/// Deliberately a bounded `Condvar::wait_timeout` and not a `Barrier`: once
/// the read-modify-write is serialized the second thread never reaches the
/// rendezvous at all (it waits on the write lock instead), and a `Barrier`
/// would deadlock the green run forever. The timeout makes the red instant
/// and the green bounded (~0.3 s).
struct IndexRaceStore {
    inner: NativeStore,
    index_slot: u64,
    arrived: std::sync::Mutex<usize>,
    gate: std::sync::Condvar,
}

impl IndexRaceStore {
    fn new(inner: NativeStore, index_slot: u64) -> Self {
        Self {
            inner,
            index_slot,
            arrived: std::sync::Mutex::new(0),
            gate: std::sync::Condvar::new(),
        }
    }

    /// Hold the caller until a second reader of the index slot shows up, or
    /// 300 ms elapse — whichever comes first.
    fn rendezvous(&self) {
        let mut arrived = self
            .arrived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *arrived >= 2 {
            return;
        }
        *arrived += 1;
        self.gate.notify_all();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        while *arrived < 2 {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (guard, outcome) = self
                .gate
                .wait_timeout(arrived, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            arrived = guard;
            if outcome.timed_out() {
                break;
            }
        }
    }
}

/// The `MemoryStore` surface the fault decorators below never intercept.
/// Emitted once: two decorators copy-pasting ~90 lines of pure delegation
/// trips the repo's duplication gate, which is blocking on this repo.
macro_rules! delegate_untouched_store_methods {
    () => {
        fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError> {
            self.inner.store(id, content, embedding)
        }

        fn store_with_metadata(
            &self,
            id: u64,
            content: &str,
            embedding: &[f32],
            metadata: &Metadata,
        ) -> Result<(), MemoryError> {
            self.inner
                .store_with_metadata(id, content, embedding, metadata)
        }

        fn store_with_ttl(
            &self,
            id: u64,
            content: &str,
            embedding: &[f32],
            ttl_seconds: u64,
        ) -> Result<(), MemoryError> {
            self.inner
                .store_with_ttl(id, content, embedding, ttl_seconds)
        }

        fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
            self.inner.update_metadata(id, metadata)
        }

        fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
            self.inner.get_metadata(id)
        }

        fn delete(&self, id: u64) -> Result<(), MemoryError> {
            self.inner.delete(id)
        }

        fn count(&self) -> usize {
            self.inner.count()
        }
    };
}

impl FactStore for IndexRaceStore {
    delegate_untouched_store_methods!();

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        self.inner.get(id)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        if ids == [self.index_slot] {
            self.rendezvous();
        }
        self.inner.get_metadata_batch(ids)
    }
}

#[test]
fn test_concurrent_saves_on_one_project_keep_both_sessions_in_the_index() {
    // Given one service over a store that pins the two index readers together
    let dir = tempfile::TempDir::new().expect("tempdir");
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    let svc = MemoryService::with_store(
        IndexRaceStore::new(native, working_index_id("veles")),
        HashEmbedder::new(DIM),
    );

    // When two sessions of the SAME project are saved concurrently
    std::thread::scope(|scope| {
        for session in ["alpha", "beta"] {
            let svc = &svc;
            scope.spawn(move || {
                svc.save_working_context("veles", session, &minimal_working())
                    .expect("save_working_context");
            });
        }
    });

    // Then the index still knows about BOTH.
    let names: Vec<String> = svc
        .list_working_contexts("veles")
        .expect("list_working_contexts")
        .into_iter()
        .map(|entry| entry.session)
        .collect();
    assert!(
        names.contains(&"alpha".to_owned()) && names.contains(&"beta".to_owned()),
        "the per-project index is a single fact updated read-modify-write: two \
         concurrent saves both read the pre-state and the second write erases \
         the first session's entry; got {names:?}"
    );

    // And the loss is silent, which is what makes it dangerous: BOTH working
    // contexts are still on disk and loadable by exact id — only the index
    // that lets an agent discover them lost an entry, with no error anywhere.
    for session in ["alpha", "beta"] {
        assert!(
            svc.load_working_context("veles", session)
                .expect("load_working_context")
                .is_some(),
            "{session}'s working context fact itself must still exist"
        );
    }
}

// ---------------------------------------------------------------------------
// Working-context index: silent degradations (B4)
// ---------------------------------------------------------------------------

/// A [`NativeStore`] decorator that reports one slot's metadata normally but
/// serves no body for it — the "torn fact" state: the index marker says an
/// index is there, its content is gone.
///
/// Reached through the decorator rather than forged on `NativeStore`, because
/// on `NativeStore` it is *unforgeable*: `delete` removes metadata and body
/// together (verified), and no store primitive drops a body alone. The state
/// is nonetheless part of the [`MemoryStore`] CONTRACT — any backend whose
/// metadata and content live in separate maps, or any write torn by a crash,
/// can produce it — and the contract under test is what the bridge must do
/// when it sees it: report corruption, not emptiness.
struct TornBodyStore {
    inner: NativeStore,
    torn: u64,
}

impl FactStore for TornBodyStore {
    delegate_untouched_store_methods!();

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        if id == self.torn {
            return Ok(None);
        }
        self.inner.get(id)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.inner.get_metadata_batch(ids)
    }
}

#[test]
fn test_working_index_with_marker_but_no_body_is_an_error_not_an_empty_list() {
    // Given a project whose index fact is marked but whose body is gone
    let dir = tempfile::TempDir::new().expect("tempdir");
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    let svc = MemoryService::with_store(
        TornBodyStore {
            inner: native,
            torn: working_index_id("veles"),
        },
        HashEmbedder::new(DIM),
    );
    svc.save_working_context("veles", "alpha", &minimal_working())
        .expect("save_working_context");

    // When listing the project's sessions
    let listed = svc.list_working_contexts("veles");

    // Then it is an ERROR, not an empty list. Reporting `Ok([])` here tells
    // the caller "this project never saved anything" — the one answer that is
    // certainly false, since the marker proves an index was written. An agent
    // told that starts over from scratch and silently abandons every saved
    // session instead of surfacing a store problem a human could fix.
    assert!(
        listed.is_err(),
        "a marked-but-bodyless index is corruption and must surface as an \
         error; got {listed:?}"
    );
}

/// A service whose project index is corrupt but whose per-session facts are
/// intact — the state both tests below start from.
fn service_with_torn_index(
    dir: &tempfile::TempDir,
    project: &str,
) -> MemoryService<HashEmbedder, TornBodyStore> {
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    MemoryService::with_store(
        TornBodyStore {
            inner: native,
            torn: working_index_id(project),
        },
        HashEmbedder::new(DIM),
    )
}

#[test]
fn test_resume_returns_an_intact_context_even_when_the_project_index_is_corrupt() {
    // Given a project whose index body is gone but whose saved session is fine
    let dir = tempfile::TempDir::new().expect("tempdir");
    let svc = service_with_torn_index(&dir, "veles");
    svc.save_working_context("veles", "alpha", &minimal_working())
        .expect("save_working_context");

    // When resuming that very session
    let resumed = svc.resume_working_context("veles", "alpha");

    // Then the caller gets what it asked for. `other_sessions` is a HINT for
    // spotting a typo, not the answer: letting its index deny an intact
    // payload turns a fault in an auxiliary fact into a total loss of
    // resumption for EVERY session of the project, including the ones that
    // read back perfectly. The corruption stays loudly reachable through
    // `list_working_contexts`, which every surface publishes.
    let resumed = resumed.expect("a corrupt index must not deny an intact working context");
    assert!(resumed.found, "the session's own fact is intact");
    assert!(resumed.working.is_some(), "and must be handed back");
    assert!(
        resumed.other_sessions.is_empty(),
        "the hint is unavailable, so it is empty — not fabricated"
    );
}

#[test]
fn test_resume_still_fails_on_a_miss_when_the_project_index_is_corrupt() {
    // Given the same corrupt index, and a session id that was never saved
    let dir = tempfile::TempDir::new().expect("tempdir");
    let svc = service_with_torn_index(&dir, "veles");
    svc.save_working_context("veles", "alpha", &minimal_working())
        .expect("save_working_context");

    // When resuming a session that does not exist
    let resumed = svc.resume_working_context("veles", "typo");

    // Then it is an ERROR, and must stay one. On a miss `other_sessions` is
    // the ONLY signal the caller has: an empty list is the positive assertion
    // "nothing else was ever saved here", and an agent told that starts over
    // on top of `alpha`. Degrading a miss the way a hit degrades would
    // manufacture exactly the silent restart this envelope exists to prevent.
    assert!(
        resumed.is_err(),
        "a miss must never assert 'no other session' from an index it could \
         not read; got {resumed:?}"
    );
}

#[test]
fn test_load_working_context_does_not_mutate_the_index() {
    // Given a project with two saved sessions, one of whose facts is gone
    let (_dir, svc) = open_service();
    let alpha = svc
        .save_working_context("veles", "alpha", &minimal_working())
        .expect("save alpha");
    svc.save_working_context("veles", "beta", &minimal_working())
        .expect("save beta");
    svc.forget(alpha).expect("forget alpha");

    // When a READ misses on the dead session
    assert!(svc
        .load_working_context("veles", "alpha")
        .expect("load_working_context")
        .is_none());

    // Then the persisted index is untouched: a lookup must not rewrite shared
    // state. Healing from the read path means every transient miss (a store
    // hiccup, a racing writer, a probe for a session that does not exist yet)
    // permanently deletes an index entry for a session that may be perfectly
    // alive — a read that destroys data is a read that cannot be retried.
    let (raw, _) = svc
        .store
        .get(working_index_id("veles"))
        .expect("read index slot")
        .expect("index fact exists");
    let index: WorkingContextIndex = serde_json::from_str(&raw).expect("index parses");
    let persisted: Vec<&str> = index
        .sessions
        .iter()
        .map(|entry| entry.session.as_str())
        .collect();
    // Most-recently-saved first is the index's STORED order (the writer
    // moves each saved entry to the front), so two saves persist as
    // [beta, alpha]. The read must leave exactly that in place.
    assert_eq!(
        persisted,
        vec!["beta", "alpha"],
        "load_working_context must not rewrite the shared index"
    );

    // And the caller still gets the truth: the dead session is filtered out
    // of the listing at read time, without persisting that decision.
    let listed: Vec<String> = svc
        .list_working_contexts("veles")
        .expect("list_working_contexts")
        .into_iter()
        .map(|entry| entry.session)
        .collect();
    assert_eq!(listed, vec!["beta".to_owned()]);
}

#[test]
fn test_load_working_context_with_marker_but_no_body_is_an_error_not_a_fresh_start() {
    // Given a session whose working-context fact is marked but bodyless
    let dir = tempfile::TempDir::new().expect("tempdir");
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    let svc = MemoryService::with_store(
        TornBodyStore {
            inner: native,
            torn: working_id("veles", "alpha"),
        },
        HashEmbedder::new(DIM),
    );
    svc.save_working_context("veles", "alpha", &minimal_working())
        .expect("save_working_context");

    // When resuming it
    let loaded = svc.load_working_context("veles", "alpha");

    // Then it is an ERROR, not `None`. `None` means "fresh start, nothing was
    // ever saved here" and an agent acts on it by discarding the session and
    // redoing the work; the marker proves that is false. Only the UNMARKED
    // slot stays silently `None` — that one is the deliberate squatter guard,
    // and it is also where an ordinary `forget` lands.
    assert!(
        loaded.is_err(),
        "a marked-but-bodyless working context is corruption, not a fresh \
         start; got {loaded:?}"
    );
}

/// Records the byte length of every text it is asked to embed, so a test can
/// pin what actually reaches the embedding backend — the store keeps the
/// whole content either way, so only this spy can tell the difference.
struct LengthSpyEmbedder {
    inner: HashEmbedder,
    seen: std::sync::Mutex<Vec<usize>>,
}

impl LengthSpyEmbedder {
    fn new() -> Self {
        Self {
            inner: HashEmbedder::new(DIM),
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl crate::embedder::Embedder for &LengthSpyEmbedder {
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>, crate::embedder::EmbedError> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(text.len());
        self.inner.embed(text)
    }
}

/// Issue #1654, the residue: `compile_context` stores every non-duplicate
/// fragment as a source (`store_sources` defaults to true) and embedded the
/// FULL content, bypassing the `MAX_EMBEDDABLE_TEXT_BYTES` gate `remember`
/// gets — an 8 KiB fragment reached the real backend whole and came back as
/// a raw `ollama embeddings call failed`, naming neither size nor cap. The
/// compile itself was fine; the write-back killed it.
///
/// Contract pinned here: the compile SUCCEEDS, the source text sent to the
/// embedder is capped, and the STORED content stays whole — retrieval is
/// hash-addressed, so truncating the embedded text must not cost a byte of
/// the retrievable source.
#[test]
fn oversized_fragment_compiles_and_embeds_a_capped_text() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let spy = LengthSpyEmbedder::new();
    let svc = MemoryService::open(dir.path(), &spy).expect("open memory store");

    let big = "mot ".repeat(2048); // 8192 bytes, 4x the embeddable cap
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request(&big, CompilePolicy::default()))
        .expect("an oversized fragment must compile; the cap applies to the embedded text");

    let worst = spy
        .seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .copied()
        .max()
        .expect("the compile embedded at least the source");
    assert!(
        worst <= crate::limits::MAX_EMBEDDABLE_TEXT_BYTES,
        "the embedder must never see more than the embeddable cap \
         ({} bytes), got {worst}",
        crate::limits::MAX_EMBEDDABLE_TEXT_BYTES,
    );

    let handle = out
        .sources
        .first()
        .expect("the fragment was stored as a source")
        .handle
        .clone();
    let source = svc
        .retrieve_context_source(&handle)
        .expect("the stored source resolves");
    assert_eq!(
        source.content, big,
        "the STORED content must stay whole — only the embedded text is capped"
    );
}

/// Returns an unparseable index body only while `armed` — a transient read
/// failure (a torn page, a partial write later repaired), the case that
/// separates "the listing is gone because the data is gone" from "the
/// listing is gone because the write path threw it away".
struct TransientlyCorruptStore {
    inner: NativeStore,
    torn: u64,
    armed: std::sync::atomic::AtomicBool,
}

impl FactStore for TransientlyCorruptStore {
    delegate_untouched_store_methods!();

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        if id == self.torn && self.armed.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Some(("}{ not json".to_owned(), vec![0.0; DIM])));
        }
        self.inner.get(id)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.inner.get_metadata_batch(ids)
    }

    fn list(
        &self,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<crate::storage::RawListedFact>, Option<u64>), MemoryError> {
        self.inner.list(cursor, limit)
    }
}

#[test]
fn test_a_transient_index_corruption_does_not_erase_the_listing() {
    // Given three saved sessions and a healthy listing
    let dir = tempfile::TempDir::new().expect("tempdir");
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    let svc = MemoryService::with_store(
        TransientlyCorruptStore {
            inner: native,
            torn: working_index_id("veles"),
            armed: std::sync::atomic::AtomicBool::new(false),
        },
        HashEmbedder::new(DIM),
    );
    for session in ["alpha", "beta", "gamma"] {
        svc.save_working_context("veles", session, &minimal_working())
            .expect("save");
    }
    assert_eq!(svc.list_working_contexts("veles").expect("list").len(), 3);

    // When one save coincides with an unreadable index
    svc.store
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    svc.save_working_context("veles", "delta", &minimal_working())
        .expect("a corrupt index must never brick saving");
    svc.store
        .armed
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Then every session is still listed. Before the rebuild the write path
    // started from an EMPTY index and overwrote the corrupt one with a
    // single entry: alpha, beta and gamma left the listing forever while
    // their facts sat intact on disk, and the read-path error a human was
    // supposed to act on was destroyed by that same write.
    let mut listed: Vec<String> = svc
        .list_working_contexts("veles")
        .expect("list after")
        .into_iter()
        .map(|s| s.session)
        .collect();
    listed.sort();
    assert_eq!(listed, ["alpha", "beta", "delta", "gamma"]);

    // And the facts themselves were never in doubt.
    for session in ["alpha", "beta", "gamma", "delta"] {
        assert!(svc
            .load_working_context("veles", session)
            .expect("load")
            .is_some());
    }
}

/// A store that can hold facts but cannot walk them — `FactStore::list` is
/// defaulted to `Unsupported` precisely so an out-of-crate backend keeps
/// compiling, and the recovery must degrade rather than fail.
struct UnwalkableStore {
    inner: NativeStore,
    torn: u64,
}

impl FactStore for UnwalkableStore {
    delegate_untouched_store_methods!();

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        if id == self.torn {
            return Ok(Some(("}{ not json".to_owned(), vec![0.0; DIM])));
        }
        self.inner.get(id)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.inner.get_metadata_batch(ids)
    }
}

#[test]
fn test_a_backend_that_cannot_enumerate_still_saves_through_a_corrupt_index() {
    // Given a backend whose `list` is the defaulted refusal, and a corrupt index
    let dir = tempfile::TempDir::new().expect("tempdir");
    let native = NativeStore::open(dir.path(), DIM).expect("open native store");
    let svc = MemoryService::with_store(
        UnwalkableStore {
            inner: native,
            torn: working_index_id("veles"),
        },
        HashEmbedder::new(DIM),
    );

    // The FIRST save cannot reach the corruption path: no index fact exists
    // yet, so `working_index` answers `Ok(None)` on the absent marker and
    // never reads a body. It is what PUTS the marker there.
    svc.save_working_context("veles", "alpha", &minimal_working())
        .expect("first save creates the index");

    // When a second save reads that now-marked index and finds it unreadable
    let saved = svc.save_working_context("veles", "beta", &minimal_working());

    // Then the save still succeeds. The listing cannot be recovered here —
    // nothing can walk the facts — but refusing the save would brick the
    // project forever, which is the outcome the rebuild exists to avoid, not
    // to introduce.
    assert!(
        saved.is_ok(),
        "a backend that cannot enumerate must still save: {saved:?}"
    );
}

// ---------------------------------------------------------------------------
// Working-context key cap (MAX_WORKING_CONTEXT_KEY_BYTES).
//
// `project` and `session` are ids, and they were the one caller string that
// reached storage past every other ceiling: into the fact's metadata (past
// MAX_METADATA_BYTES, which only `remember`'s metadata path checks), into the
// embedder (past MAX_EMBEDDABLE_TEXT_BYTES, likewise), and ×1000 into the
// project index rewritten whole on every save. Measured before the cap: a
// 600 KiB session id was accepted, stored 614 677 bytes of metadata, and the
// SECOND such save was refused by the backend's 1 MiB payload cap AFTER its
// fact was stored — durable, unlisted, and reported to the caller as failed.
// ---------------------------------------------------------------------------

#[test]
fn test_a_session_id_over_the_cap_is_refused_before_anything_is_written() {
    let (_dir, svc) = open_service();
    let over = "s".repeat(crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES + 1);

    let refused = svc.save_working_context("proj", &over, &minimal_working());

    assert!(
        matches!(
            refused,
            Err(MemoryError::WorkingContextKeyTooLong { key: "session", len, max })
                if len == crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES + 1
                    && max == crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES
        ),
        "an over-cap session id must be refused as WorkingContextKeyTooLong: {refused:?}"
    );
    // Nothing was written: no fact under that id, no index for the project.
    assert!(svc
        .store
        .get_metadata(working_id("proj", &over))
        .expect("read")
        .is_none());
    assert!(svc.list_working_contexts("proj").expect("list").is_empty());
}

#[test]
fn test_a_project_id_over_the_cap_is_refused_and_names_the_key() {
    let (_dir, svc) = open_service();
    let over = "p".repeat(crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES + 1);
    let refused = svc.save_working_context(&over, "sess", &minimal_working());
    assert!(
        matches!(
            refused,
            Err(MemoryError::WorkingContextKeyTooLong { key: "project", .. })
        ),
        "{refused:?}"
    );
}

#[test]
fn test_an_id_exactly_at_the_cap_is_accepted() {
    // The cap is inclusive: 256 bytes is an id, 257 is a payload.
    let (_dir, svc) = open_service();
    let at_cap = "s".repeat(crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES);
    svc.save_working_context("proj", &at_cap, &minimal_working())
        .expect("an id exactly at the cap is accepted");
    let listed = svc.list_working_contexts("proj").expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session, at_cap);
}

#[test]
fn test_the_empty_check_still_wins_over_the_length_check() {
    // Whitespace-only and short: the EMPTY refusal is the right one. A
    // whitespace-only id over the cap is also empty first — the emptiness is
    // the more fundamental defect, and the error a caller can act on.
    let (_dir, svc) = open_service();
    let blank_and_long = " ".repeat(crate::limits::MAX_WORKING_CONTEXT_KEY_BYTES + 1);
    let refused = svc.save_working_context("proj", &blank_and_long, &minimal_working());
    assert!(
        matches!(
            refused,
            Err(MemoryError::EmptyWorkingContextKey { key: "session" })
        ),
        "{refused:?}"
    );
}

// ---------------------------------------------------------------------------
// "Most-recently-saved first" must hold where `saved_at` cannot decide.
//
// `saved_at` is whole seconds, so two saves in one second tie; on wasm32 it
// is always 0, so EVERY entry ties. The old tiebreak was `session`
// ascending — alphabetical — so on wasm the whole listing was alphabetical
// while five surfaces promised recency. Now the index's vector ORDER is
// recency (the writer moves the saved entry to the front), the read path
// and `bound_sessions` sort stably by `saved_at` alone, and ties keep it.
// ---------------------------------------------------------------------------

fn tied(sessions: &[&str]) -> Vec<WorkingContextSession> {
    sessions
        .iter()
        .map(|s| WorkingContextSession {
            session: (*s).to_owned(),
            saved_at: 0,
        })
        .collect()
}

#[test]
fn test_bound_sessions_evicts_by_recency_not_by_name_when_timestamps_tie() {
    use crate::limits::MAX_WORKING_SESSIONS_PER_PROJECT as CAP;
    // CAP + 1 entries, ALL at saved_at 0 (the wasm case), in recency order:
    // index 0 is the most recent. Names are chosen so that alphabetical
    // order is the REVERSE of recency: the most recent is named "1000", the
    // oldest "0000". The pinned (just-saved) entry is in the middle so the
    // pin cannot mask the tiebreak.
    let names: Vec<String> = (0..=CAP).map(|i| format!("{:04}", CAP - i)).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut sessions = tied(&refs);
    let pinned = names[CAP / 2].clone();

    bound_sessions(&mut sessions, &pinned);

    let kept: Vec<&str> = sessions.iter().map(|s| s.session.as_str()).collect();
    assert_eq!(kept.len(), CAP);
    assert!(
        kept.contains(&"1000"),
        "the most recent entry must survive eviction on a timestamp tie"
    );
    assert!(
        !kept.contains(&"0000"),
        "the OLDEST entry is the one to evict on a timestamp tie — the old \
         alphabetical tiebreak evicted the most recent instead"
    );
    assert!(
        kept.contains(&pinned.as_str()),
        "the pinned entry always survives"
    );
}

#[test]
fn test_bound_sessions_keeps_the_just_saved_entry_at_the_front() {
    use crate::limits::MAX_WORKING_SESSIONS_PER_PROJECT as CAP;
    // In the real flow the writer has just put the saved entry at index 0.
    // Past the cap it is removed to be pinned, and it must come BACK to
    // index 0 — not to the end — because on wasm32 (every saved_at 0) the
    // stored order is the only order there is.
    let names: Vec<String> = (0..=CAP).map(|i| format!("s{i:04}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut sessions = tied(&refs);
    let just_saved = names[0].clone();

    bound_sessions(&mut sessions, &just_saved);

    assert_eq!(sessions.len(), CAP);
    assert_eq!(
        sessions[0].session, just_saved,
        "the just-saved session must stay first after eviction at the cap"
    );
    assert_eq!(
        sessions[CAP - 1].session,
        names[CAP - 1],
        "the oldest survivor is last"
    );
    assert!(
        !sessions.iter().any(|s| s.session == names[CAP]),
        "the entry past the cap is the one evicted"
    );
}

#[test]
fn test_listing_keeps_recency_order_when_every_saved_at_ties() {
    // Three real sessions so `live_sessions` keeps them, then the index is
    // overwritten with all three at saved_at 0 in recency order [c, b, a]
    // (c most recent), named so that alphabetical order is the opposite.
    let (_dir, svc) = open_service();
    for session in ["a", "b", "c"] {
        svc.save_working_context("proj", session, &minimal_working())
            .expect("save");
    }
    let index = WorkingContextIndex {
        sessions: tied(&["c", "b", "a"]),
    };
    let content = serde_json::to_string(&index).expect("encode");
    let embedding = svc
        .embedder
        .embed("working context index proj")
        .expect("embed");
    svc.write_working_index("proj", &content, &embedding)
        .expect("write index");

    let listed: Vec<String> = svc
        .list_working_contexts("proj")
        .expect("list")
        .into_iter()
        .map(|s| s.session)
        .collect();

    assert_eq!(
        listed,
        ["c", "b", "a"],
        "with every saved_at tied, the listing must follow the index's recency \
         order — alphabetical would be [a, b, c]"
    );
}

#[test]
fn test_a_resave_moves_the_session_to_the_front_of_the_index() {
    // The writer's half of the contract: saving an EXISTING session moves it
    // to index 0, so the stored order stays recency even when the clock
    // cannot tell (wasm) or does not move (same second).
    let (_dir, svc) = open_service();
    for session in ["a", "b", "c"] {
        svc.save_working_context("proj", session, &minimal_working())
            .expect("save");
    }
    svc.save_working_context("proj", "a", &minimal_working())
        .expect("resave a");
    let index = svc
        .working_index("proj")
        .expect("read index")
        .expect("index present");
    let order: Vec<&str> = index.sessions.iter().map(|s| s.session.as_str()).collect();
    assert_eq!(
        order,
        ["a", "c", "b"],
        "stored order must be recency, most recent first"
    );
}
