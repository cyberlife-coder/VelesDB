//! BDD integration tests for #1761: plain `recall` must return caller
//! memories and nothing else.
//!
//! Sibling of `recall_where_scaffolding_bdd.rs`, which closed the same hole on
//! the `recall_where` leg (#1737). This one covers the OTHER leg: an unfiltered
//! `recall` goes through `query_excluding(embedding, k, &hub_exclude_filter())`,
//! and that filter names exactly one of the five reserved markers —
//! `_veles_hub`. The context compiler's four artefact classes are not excluded
//! by it at all.
//!
//! Widening that one filter is NOT the fix, and this file exists partly to pin
//! why: `payload_matches` keeps a point only when it carries EVERY key of the
//! exclude map, so a map holding all five markers would exclude a point that
//! is simultaneously a hub AND a source AND an event AND a working context AND
//! a working index — that is, nothing at all.
//!
//! Reproducing the leak needs the right query, which is why it would go
//! unnoticed: these artefacts are not embedded from the text a caller would
//! think of. An event embeds a CONSTANT anchor, so every event in the store
//! shares one vector and that anchor retrieves it exactly. A working context
//! embeds `working context {project} {session}`, not its JSON body.
//!
//! # What was measured, and why the leak does not reproduce
//!
//! Driven with those exact anchors, against a store where all five classes are
//! proven present, **the leak does not reproduce** — and the reason is not the
//! one the filter's shape suggests.
//!
//! `system_meta` (`context::memory_bridge`) stamps `_veles_hub: true` on EVERY
//! internal artefact, and only then adds the class-specific marker on top. The
//! four `_veles_ctx_*` fields are therefore discriminators, not the exclusion
//! key: `_veles_hub` is the one marker they all share, so the single-key
//! `hub_exclude_filter` already covers all five classes.
//!
//! That the coverage is real, and not an artefact of ranking, was checked by
//! neutering `hub_exclude_filter` and re-running this file: all five classes
//! then leak at once — working context, working index, compilation event,
//! stored source, and entity hubs. So these tests can fail, and they fail for
//! the right reason.
//!
//! This describes the engine as it stands, **not a guarantee about it
//! forever**, and nothing here should be read as one. The coverage rests on an
//! invariant nothing else enforces: that every internal write goes through
//! `system_meta`. Should a future artefact skip it, or the shared marker be
//! split per class, the exclusion silently stops covering that class — and
//! this file goes red on the spot instead of the leak reaching callers.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

#![cfg(all(feature = "context", feature = "persistence"))]

mod common;

use common::{meta, service, SharedTopicExtractor};
use serde_json::json;
use std::collections::BTreeSet;
use tempfile::TempDir;
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextCompiler, ContextFragment, WorkingContext,
};
use velesdb_memory::{HashEmbedder, MemoryService, Recollection};

/// The lexical anchor every compilation event's content starts with, and — the
/// part that matters here — the exact string its embedding is computed from
/// (`memory_bridge::EVENT_ANCHOR`). Kept as a literal because it is a wire
/// fact this test is pinning, not an implementation detail to import.
const EVENT_ANCHOR: &str = "veles context compilation event";

const PROJECT: &str = "veles";
const SESSION: &str = "session-1";
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
        project: Some(PROJECT.to_owned()),
        target_model: None,
        token_budget: 10_000,
        memory_scope: None,
        policy: None,
    }
}

struct Seeded {
    _dir: TempDir,
    svc: MemoryService<HashEmbedder>,
    /// Every id a caller legitimately owns. Anything else coming back from
    /// `recall` is scaffolding — named by what it is, never counted.
    caller_ids: BTreeSet<u64>,
    /// A caller fact deliberately phrased ON the event anchor, so the fix
    /// cannot pass by blacklisting that text: this one MUST keep coming back.
    caller_on_anchor: u64,
    /// An ordinary caller memory, carrying nothing special at all — the
    /// baseline that must stay visible whatever the scaffolding rules do.
    plain: u64,
    source_handle: String,
    working_id: u64,
}

/// Seed all five scaffolding classes plus caller facts, through the public API
/// only — no direct `store_fact`, so this breaks if any path stops writing its
/// marker.
fn seeded() -> Seeded {
    let (dir, svc) = service();

    let plain = svc
        .remember(&format!("{THEME}: the runbook is current"), &[], None)
        .expect("remember plain");
    // A caller fact whose own words are the anchor. Proves the fix excludes by
    // MARKER, not by matching text an artefact happens to use.
    let caller_on_anchor = svc
        .remember(
            &format!("{EVENT_ANCHOR} is what our compiler logs look like"),
            &[],
            Some(&meta(&[("status", json!("active"))])),
        )
        .expect("remember caller fact on the anchor");

    // Class 1 — `_veles_hub`.
    let extracted = svc
        .remember_extracted(THEME, &SharedTopicExtractor, None)
        .expect("remember_extracted");

    // Classes 2 and 3 — `_veles_ctx_source` and `_veles_ctx_event`.
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
        .save_working_context(PROJECT, SESSION, &working)
        .expect("save_working_context");

    let mut caller_ids: BTreeSet<u64> = [plain, caller_on_anchor].into_iter().collect();
    caller_ids.extend(extracted.ids.iter().copied());

    Seeded {
        _dir: dir,
        svc,
        caller_ids,
        caller_on_anchor,
        plain,
        source_handle,
        working_id,
    }
}

/// The scaffolding present in `hits`, named rather than counted.
fn leaked(hits: &[Recollection], caller_ids: &BTreeSet<u64>) -> Vec<(u64, String)> {
    hits.iter()
        .filter(|hit| !caller_ids.contains(&hit.id))
        .map(|hit| (hit.id, hit.content.chars().take(80).collect()))
        .collect()
}

// ===========================================================================
// Nominal — the leak, on each query that actually retrieves an artefact
// ===========================================================================

#[test]
fn recall_on_the_event_anchor_returns_no_scaffolding() {
    // The sharpest probe there is: an event's embedding IS the anchor, so this
    // query matches it exactly. Nothing about ranking can save us here.
    let s = seeded();
    let hits = s.svc.recall(EVENT_ANCHOR, 50, None).expect("recall");

    assert!(
        !hits.is_empty(),
        "the query must retrieve something, or the test proves nothing"
    );
    let leak = leaked(&hits, &s.caller_ids);
    assert!(
        leak.is_empty(),
        "unfiltered recall leaked internal scaffolding: {leak:?}"
    );
}

#[test]
fn recall_on_the_working_context_anchor_returns_no_scaffolding() {
    // A working context embeds `working context {project} {session}` — not its
    // JSON body, which is why a query shaped like the body never surfaces it.
    let s = seeded();
    let hits = s
        .svc
        .recall(&format!("working context {PROJECT} {SESSION}"), 50, None)
        .expect("recall");

    assert!(
        !hits.is_empty(),
        "the query must retrieve something, or this proof is vacuously green"
    );
    let leak = leaked(&hits, &s.caller_ids);
    assert!(
        leak.is_empty(),
        "unfiltered recall leaked internal scaffolding: {leak:?}"
    );
}

#[test]
fn recall_on_the_theme_returns_no_scaffolding() {
    // Every seeded artefact shares this theme, so the vector leg cannot be
    // what separates caller facts from scaffolding — only the fix can.
    let s = seeded();
    let hits = s.svc.recall(THEME, 50, None).expect("recall");

    assert!(!hits.is_empty(), "the theme must retrieve caller facts");
    // An ordinary caller memory — nothing special about it — must be among
    // them. Excluding scaffolding is only correct if honest facts survive.
    assert!(
        hits.iter().any(|hit| hit.id == s.plain),
        "an ordinary caller memory must still be returned by generic recall"
    );
    let leak = leaked(&hits, &s.caller_ids);
    assert!(
        leak.is_empty(),
        "unfiltered recall leaked internal scaffolding: {leak:?}"
    );
}

// ===========================================================================
// Negative — the fix must not black out honest results
// ===========================================================================

#[test]
fn a_caller_fact_phrased_on_the_anchor_still_comes_back() {
    // Guards against a fix that filters by TEXT rather than by marker.
    let s = seeded();
    let hits = s.svc.recall(EVENT_ANCHOR, 50, None).expect("recall");

    assert!(
        hits.iter().any(|hit| hit.id == s.caller_on_anchor),
        "a caller fact whose words match the anchor must still be returned"
    );
}

#[test]
fn an_explicit_caller_filter_still_selects_its_facts() {
    // The filtered leg takes a different branch (`query_filtered`); it must
    // keep working, and keep combining the caller's own predicate.
    let s = seeded();
    let hits = s
        .svc
        .recall(THEME, 50, Some(&meta(&[("status", json!("active"))])))
        .expect("filtered recall");

    assert!(
        hits.iter().any(|hit| hit.id == s.caller_on_anchor),
        "the caller's own filter must still select its fact"
    );
    let leak = leaked(&hits, &s.caller_ids);
    assert!(
        leak.is_empty(),
        "filtered recall leaked scaffolding: {leak:?}"
    );
}

#[test]
fn the_specialised_apis_still_reach_their_own_artefacts() {
    // Excluding scaffolding from RECALL must not make it unreachable to the
    // APIs that own it — the artefacts are hidden, not deleted.
    let s = seeded();

    let sessions = s.svc.list_working_contexts(PROJECT).expect("list");
    assert!(
        sessions.iter().any(|entry| entry.session == SESSION),
        "list_working_contexts must still see the working context"
    );

    let loaded = s
        .svc
        .load_working_context(PROJECT, SESSION)
        .expect("load")
        .expect("the working context must still load");
    assert_eq!(loaded.goal.as_deref(), Some(THEME));

    let source = s
        .svc
        .retrieve_context_source(&s.source_handle)
        .expect("retrieve_context_source");
    assert!(
        source.content.contains(THEME),
        "a stored source must still be retrievable by its handle"
    );

    assert!(s.working_id > 0, "the working context really was written");
}

#[test]
fn every_seeded_artefact_class_is_really_present() {
    // Without this, "recall leaked nothing" could just mean "nothing was
    // there to leak". Each class is proven present through the API that owns
    // it, BEFORE any claim about recall is worth making.
    let s = seeded();

    // `_veles_ctx_event`: counted by the aggregation that reads events, which
    // filters on the reserved event marker at the storage layer.
    let savings = s
        .svc
        .context_savings(Some(PROJECT))
        .expect("context_savings");
    assert!(
        savings.events > 0,
        "at least one compilation event must exist, otherwise the anchor probe \
         is vacuous — got {} events",
        savings.events
    );

    // `_veles_ctx_source`: retrievable by its handle.
    assert!(s
        .svc
        .retrieve_context_source(&s.source_handle)
        .expect("retrieve_context_source")
        .content
        .contains(THEME));

    // `_veles_ctx_working` and `_veles_ctx_working_index`: the index is what
    // makes the session listable at all, so listing it proves both.
    let sessions = s.svc.list_working_contexts(PROJECT).expect("list");
    assert!(sessions.iter().any(|entry| entry.session == SESSION));

    // `_veles_hub`: extraction minted at least one caller-visible id beyond
    // the two plain remembers.
    assert!(
        s.caller_ids.len() > 2,
        "remember_extracted must have minted ids, so a hub exists"
    );
}

// ===========================================================================
// Edge — ordering, scores and the limit of legitimate results are untouched
// ===========================================================================

#[test]
fn legitimate_results_keep_their_order_scores_and_limit() {
    let s = seeded();
    let hits = s.svc.recall(THEME, 50, None).expect("recall");

    let scores: Vec<f32> = hits.iter().map(|hit| hit.score).collect();
    assert!(
        scores.windows(2).all(|w| w[0] >= w[1]),
        "results must stay in descending score order, got {scores:?}"
    );

    let limited = s.svc.recall(THEME, 1, None).expect("recall k=1");
    assert!(limited.len() <= 1, "k must still bound the result count");
    if let (Some(first_of_all), Some(only)) = (hits.first(), limited.first()) {
        assert_eq!(
            first_of_all.id, only.id,
            "narrowing k must not change which result ranks first"
        );
        assert!(
            (first_of_all.score - only.score).abs() < f32::EPSILON,
            "a result's score must not depend on k"
        );
    }
}
