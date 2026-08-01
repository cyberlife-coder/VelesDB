//! BDD integration tests for #1737: `recall_where` must return caller
//! memories and nothing else.
//!
//! The service stores five classes of internal scaffolding as ordinary facts
//! in the same collection as caller memories — entity hubs and the four
//! context-compiler artefacts. They are marked with reserved `_veles_*` keys
//! that a caller can neither set nor filter on, which was believed to make
//! them invisible to every caller-facing recall path.
//!
//! It does not, for a `!=` predicate. [`velesdb_core::Condition::Neq`] is
//! `is_none_or`: a fact that has no such field at all *matches*. Scaffolding
//! carries only reserved keys, so it has none of the caller's columns, so
//! `status != "archived"` sweeps in every artefact the service ever wrote.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::{meta, service, SharedTopicExtractor, DIM};
use serde_json::json;
use std::collections::BTreeSet;
use tempfile::TempDir;
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextCompiler, ContextFragment, WorkingContext,
};
use velesdb_memory::{ColumnFilter, ColumnOp, HashEmbedder, MemoryService};

/// One lexical theme for every seeded fact, caller and scaffolding alike, so
/// the vector leg cannot be what separates them — only the fix can.
const THEME: &str = "rebalance the primary node during a deploy";

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

fn request(query: &str, fragments: Vec<ContextFragment>) -> CompileRequest {
    CompileRequest {
        query: query.to_owned(),
        fragments,
        project: Some("veles".to_owned()),
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    }
}

/// What a caller is entitled to see, and the handle/id of the scaffolding the
/// guard tests re-read afterwards.
struct Seeded {
    _dir: TempDir,
    svc: MemoryService<HashEmbedder>,
    /// Every id a caller legitimately owns. Anything else coming back from
    /// `recall_where` is scaffolding — no hard-coded count, no name list.
    caller_ids: BTreeSet<u64>,
    /// A caller fact carrying `status: "active"`, i.e. one the predicate
    /// matches on its own terms.
    with_status: u64,
    /// A caller fact carrying NO `status` column at all. `!=` must keep it
    /// (that is what `is_none_or` means for a caller field) — proving the fix
    /// excludes scaffolding without also blacking out honest facts.
    without_status: u64,
    /// `ctx://source/<hash>` handle of a stored compilation source.
    source_handle: String,
    /// Id returned by `save_working_context`.
    working_id: u64,
}

/// Seed all five classes of scaffolding plus the caller facts, through the
/// public API only — no direct `store_fact`, so the test breaks if any of
/// these paths stops writing its marker.
fn seeded() -> Seeded {
    let (dir, svc) = service();

    // Caller facts: one with the filtered column, one without it.
    let with_status = svc
        .remember(
            &format!("{THEME}: the runbook is current"),
            &[],
            Some(&meta(&[("status", json!("active"))])),
        )
        .expect("remember with status");
    let without_status = svc
        .remember(&format!("{THEME}: no status column on this one"), &[], None)
        .expect("remember without status");

    // Class 1 — `_veles_hub`: entity hubs minted by extraction.
    let extracted = svc
        .remember_extracted(THEME, &SharedTopicExtractor, None)
        .expect("remember_extracted");

    // Classes 2 and 3 — `_veles_ctx_source` and `_veles_ctx_event`: stored
    // originals and the compilation event recorded for `context_savings`.
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let out = svc
        .compile_context(&compiler, &request(THEME, vec![fragment(THEME)]))
        .expect("compile_context");
    let source_handle = out.sources[0].handle.clone();

    // Classes 4 and 5 — `_veles_ctx_working` and `_veles_ctx_working_index`.
    let working = WorkingContext {
        goal: Some(THEME.to_owned()),
        ..WorkingContext::default()
    };
    let working_id = svc
        .save_working_context("veles", "session-1", &working)
        .expect("save_working_context");

    let mut caller_ids: BTreeSet<u64> = [with_status, without_status].into_iter().collect();
    caller_ids.extend(extracted.ids.iter().copied());

    Seeded {
        _dir: dir,
        svc,
        caller_ids,
        with_status,
        without_status,
        source_handle,
        working_id,
    }
}

/// The scaffolding ids present in `hits` — the leak, named by what it is
/// rather than counted.
fn leaked(hits: &[velesdb_memory::Recollection], caller_ids: &BTreeSet<u64>) -> Vec<(u64, String)> {
    hits.iter()
        .filter(|hit| !caller_ids.contains(&hit.id))
        .map(|hit| (hit.id, hit.content.clone()))
        .collect()
}

/// `recall_where` under the predicate that opened #1737: `status` is a
/// CALLER column, and no piece of scaffolding carries it.
fn recall_status_ne_archived(
    svc: &MemoryService<HashEmbedder>,
) -> Vec<velesdb_memory::Recollection> {
    svc.recall_where(
        THEME,
        50,
        &[ColumnFilter {
            field: "status".to_owned(),
            op: ColumnOp::Ne,
            value: json!("archived"),
        }],
    )
    .expect("recall_where")
}

// --- Nominal: the `!=` predicate that opened the leak ------------------------

#[test]
fn recall_where_ne_predicate_returns_no_internal_scaffolding() {
    let seed = seeded();

    let hits = recall_status_ne_archived(&seed.svc);

    let leak = leaked(&hits, &seed.caller_ids);
    assert!(
        leak.is_empty(),
        "`status != \"archived\"` returned internal scaffolding, which carries \
         no `status` column and so matches every `!=` predicate:\n{leak:#?}"
    );
}

#[test]
fn recall_where_ne_predicate_still_returns_caller_facts() {
    let seed = seeded();

    let hits = recall_status_ne_archived(&seed.svc);
    let ids: BTreeSet<u64> = hits.iter().map(|hit| hit.id).collect();

    assert!(
        ids.contains(&seed.with_status),
        "a caller fact whose `status` is not \"archived\" must be returned"
    );
    assert!(
        ids.contains(&seed.without_status),
        "a caller fact with NO `status` column must still be returned: `!=` \
         keeps a missing caller field, and excluding scaffolding must not \
         quietly turn that into a blackout"
    );
}

// --- Edge: the same contract on the no-predicate path ------------------------

#[test]
fn recall_where_without_predicates_returns_no_internal_scaffolding() {
    let seed = seeded();

    let hits = seed.svc.recall_where(THEME, 50, &[]).expect("recall_where");

    // Paired with the exclusion assertion on purpose: "no scaffolding came
    // back" passes on its own the moment nothing comes back at all.
    let ids: BTreeSet<u64> = hits.iter().map(|hit| hit.id).collect();
    assert!(
        ids.contains(&seed.with_status) && ids.contains(&seed.without_status),
        "the caller facts must come back, or the exclusion below proves nothing; \
         got {} hit(s)",
        hits.len()
    );

    let leak = leaked(&hits, &seed.caller_ids);
    assert!(
        leak.is_empty(),
        "an empty filter set routes through `recall`, which excludes hubs but \
         not the four context-compiler classes:\n{leak:#?}"
    );
}

// --- Negative: the fix must not blind the internal readers -------------------

#[test]
fn internal_readers_still_see_their_own_artefacts() {
    let seed = seeded();

    // `retrieve_context_source` — reads a source by salted id, marker-checked.
    let source = seed
        .svc
        .retrieve_context_source(&seed.source_handle)
        .expect("retrieve_context_source must still resolve its handle");
    assert_eq!(source.content, THEME, "the exact original must round-trip");

    // `load_working_context` — reads the working fact by salted id.
    let loaded = seed
        .svc
        .load_working_context("veles", "session-1")
        .expect("load_working_context must still find the working fact")
        .expect("the saved working context must be found");
    assert_eq!(
        loaded.goal.as_deref(),
        Some(THEME),
        "the saved working context must come back intact"
    );
    assert!(seed.working_id != 0, "save_working_context returned an id");

    // `list_working_contexts` — reads the index fact.
    let sessions = seed
        .svc
        .list_working_contexts("veles")
        .expect("list_working_contexts must still find the index fact");
    assert!(
        sessions.iter().any(|s| s.session == "session-1"),
        "the working-context index must still list the saved session, got {sessions:?}"
    );

    // `context_savings` — aggregates events at the storage layer.
    let savings = seed
        .svc
        .context_savings(Some("veles"))
        .expect("context_savings must still aggregate its events");
    assert!(
        savings.events > 0,
        "the compilation event must still be counted, got {savings:?}"
    );

    // `entity_profile` — reads a hub by salted id.
    let profile = seed
        .svc
        .entity_profile("rust")
        .expect("entity_profile must still resolve")
        .expect("the `rust` hub must exist");
    assert_eq!(profile.name, "rust", "the hub must still be reachable");
}

#[test]
fn scaffolding_stays_excluded_across_a_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let working = WorkingContext {
        goal: Some(THEME.to_owned()),
        ..WorkingContext::default()
    };
    let caller_id = {
        let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("open");
        let id = svc.remember(THEME, &[], None).expect("remember");
        svc.save_working_context("veles", "session-1", &working)
            .expect("save_working_context");
        id
    };

    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM)).expect("reopen");
    let hits = recall_status_ne_archived(&svc);

    let caller_ids: BTreeSet<u64> = [caller_id].into_iter().collect();
    let leak = leaked(&hits, &caller_ids);
    assert!(
        leak.is_empty(),
        "the exclusion is a property of the stored marker, not of this \
         process's memory of what it wrote:\n{leak:#?}"
    );
}
