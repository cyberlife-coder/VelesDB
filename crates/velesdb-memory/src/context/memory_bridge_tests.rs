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
    assert_eq!(
        persisted,
        vec!["alpha", "beta"],
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
