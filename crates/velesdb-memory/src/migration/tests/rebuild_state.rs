//! GATE 6 — the journal knows how far the rebuild got (#1762, PR C2b).
//!
//! v2 recorded WHERE a migration stood (its phase) but nothing about how far
//! the work inside `Prepared` had progressed: a resumed rebuild would have had
//! to start every collection from zero and re-answer every collision. v3 adds
//! per-collection progress, and these tests pin the properties that make it a
//! journal rather than a scratchpad: it round-trips exactly, it can only
//! advance ALONG THE TRANSITIONS THE PASS ACTUALLY EMITS (`Facts → Edges →
//! Complete`, never skipping `Edges`), and the phase cannot leave `Prepared`
//! while any collection is unfinished.

use super::state_persistence::VALID_FINGERPRINT;
use super::*;
use std::collections::BTreeMap;

fn lock(workspace: &std::path::Path) -> MigrationLock {
    MigrationLock::acquire(workspace, "rebuild-state-test").expect("lock")
}

/// A v3 state with explicit per-collection progress.
fn state_with_progress(progress: BTreeMap<String, CollectionProgress>) -> MigrationState {
    MigrationState {
        format_version: STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: std::path::PathBuf::from("/store"),
        source_fingerprint: VALID_FINGERPRINT.to_owned(),
        target_model: diagnosis::TARGET_MODEL.to_owned(),
        target_dimension: diagnosis::TARGET_DIM,
        progress,
        embedder_witness: None,
    }
}

fn fresh_progress() -> BTreeMap<String, CollectionProgress> {
    AGENT_COLLECTIONS
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                CollectionProgress::Facts { cursor: None },
            )
        })
        .collect()
}

#[test]
fn mixed_progress_round_trips_byte_exactly_through_write_and_read() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());

    // Every stage is represented at once, because the bug this guards against
    // is a serde attribute that flattens one variant into another. The mixed
    // shape is reached through the pass's own transitions — a journal must
    // begin at the beginning and may not skip Edges.
    let mut step = fresh_progress();
    step.insert(
        "_semantic_memory".to_owned(),
        CollectionProgress::Facts { cursor: Some(42) },
    );
    step.insert("_episodic_memory".to_owned(), CollectionProgress::Edges);
    step.insert("_procedural_memory".to_owned(), CollectionProgress::Edges);
    let mut mixed = step.clone();
    mixed.insert(
        "_procedural_memory".to_owned(),
        CollectionProgress::Complete,
    );
    let written = state_with_progress(mixed);

    state_with_progress(fresh_progress())
        .write(workspace.path(), &lock)
        .expect("initial write");
    state_with_progress(step)
        .write(workspace.path(), &lock)
        .expect("advance through edges");
    written.write(workspace.path(), &lock).expect("advance");

    let read = MigrationState::read(workspace.path())
        .expect("read back")
        .expect("state exists");
    assert_eq!(
        read, written,
        "per-collection progress must survive the write/read round trip \
         exactly; a variant that collapsed into another would resume the wrong \
         amount of work"
    );
}

#[test]
fn a_v2_state_is_refused_for_predating_rebuild_progress() {
    let workspace = tempfile::tempdir().expect("workspace");
    // Written by hand because no current API can produce one: this is the file
    // a real v2 build left behind.
    let v2 = serde_json::json!({
        "format_version": 2,
        "phase": "prepared",
        "source_path": "/store",
        "source_fingerprint": VALID_FINGERPRINT,
        "target_model": diagnosis::TARGET_MODEL,
        "target_dimension": diagnosis::TARGET_DIM,
    });
    std::fs::write(
        workspace.path().join(STATE_FILE),
        serde_json::to_string_pretty(&v2).expect("serialize v2"),
    )
    .expect("write v2 state");

    let refusal = MigrationState::read(workspace.path())
        .expect_err("a v2 state must be refused, not silently upgraded");
    assert!(
        refusal.contains("version 2") && refusal.contains(&STATE_FORMAT_VERSION.to_string()),
        "the refusal must name both versions: {refusal}"
    );
    assert!(
        refusal.contains("fresh diagnosis"),
        "the refusal must tell the operator what to do: {refusal}"
    );
    assert!(
        !refusal.contains("does not parse"),
        "an old version must be refused AS a version, not surface as a parse \
         error about the missing progress field: {refusal}"
    );
}

#[test]
fn progress_may_advance_or_repeat_but_never_regress() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());
    let mut current = fresh_progress();
    current.insert(
        "_semantic_memory".to_owned(),
        CollectionProgress::Facts { cursor: Some(10) },
    );
    state_with_progress(fresh_progress())
        .write(workspace.path(), &lock)
        .expect("initial write");
    state_with_progress(current.clone())
        .write(workspace.path(), &lock)
        .expect("advance to cursor 10");

    // Idempotent replay of the same entry is a resume, not a regression.
    state_with_progress(current.clone())
        .write(workspace.path(), &lock)
        .expect("the same progress may be written again");

    let regressions: Vec<(&str, CollectionProgress)> = vec![
        (
            "a smaller cursor",
            CollectionProgress::Facts { cursor: Some(5) },
        ),
        (
            "no cursor at all",
            CollectionProgress::Facts { cursor: None },
        ),
    ];
    for (label, regressed) in regressions {
        let mut candidate = current.clone();
        candidate.insert("_semantic_memory".to_owned(), regressed);
        let refusal = state_with_progress(candidate)
            .write(workspace.path(), &lock)
            .expect_err("a journal that can regress is a scratchpad");
        assert!(
            refusal.contains("_semantic_memory"),
            "refusing {label} must name the collection: {refusal}"
        );
    }

    // Skipping the Edges stage is REFUSED, not tolerated as "forward". The
    // pass journals Edges unconditionally, so no honest writer ever produces
    // Facts→Complete — only a writer that lost its edge pass would, and that
    // is precisely the bug this refusal makes un-journallable.
    let mut skipped = current.clone();
    skipped.insert("_semantic_memory".to_owned(), CollectionProgress::Complete);
    let refusal = state_with_progress(skipped)
        .write(workspace.path(), &lock)
        .expect_err("Complete without passing through Edges means the edge pass never ran");
    assert!(
        refusal.contains("_semantic_memory"),
        "the refusal must name the collection: {refusal}"
    );

    // ...and a finished stage cannot be reopened.
    let mut done = current.clone();
    done.insert("_semantic_memory".to_owned(), CollectionProgress::Edges);
    state_with_progress(done.clone())
        .write(workspace.path(), &lock)
        .expect("facts to edges is the pass's own transition");
    done.insert("_semantic_memory".to_owned(), CollectionProgress::Complete);
    state_with_progress(done.clone())
        .write(workspace.path(), &lock)
        .expect("edges to complete is the pass's own transition");
    let mut reopened = done;
    reopened.insert("_semantic_memory".to_owned(), CollectionProgress::Edges);
    let refusal = state_with_progress(reopened)
        .write(workspace.path(), &lock)
        .expect_err("a completed collection cannot be reopened");
    assert!(
        refusal.contains("_semantic_memory"),
        "the refusal must name the collection: {refusal}"
    );
}

#[test]
fn the_phase_cannot_leave_prepared_while_any_collection_is_unfinished() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());
    // Reached through the pass's own transitions: everything to Edges, then
    // everything but episodic to Complete.
    let mut edging = BTreeMap::new();
    for name in AGENT_COLLECTIONS {
        edging.insert((*name).to_owned(), CollectionProgress::Edges);
    }
    let mut almost = edging.clone();
    for name in AGENT_COLLECTIONS {
        almost.insert((*name).to_owned(), CollectionProgress::Complete);
    }
    almost.insert("_episodic_memory".to_owned(), CollectionProgress::Edges);

    state_with_progress(fresh_progress())
        .write(workspace.path(), &lock)
        .expect("initial write");
    state_with_progress(edging)
        .write(workspace.path(), &lock)
        .expect("advance to edges");
    state_with_progress(almost.clone())
        .write(workspace.path(), &lock)
        .expect("advance");

    let mut premature = state_with_progress(almost);
    premature.phase = Phase::DestinationValidated;
    let refusal = premature
        .write(workspace.path(), &lock)
        .expect_err("validating a destination whose rebuild is unfinished");
    assert!(
        refusal.contains("_episodic_memory"),
        "the refusal must name the unfinished collection: {refusal}"
    );

    // Positive control: with every collection Complete the same transition is
    // accepted, so the refusal above was about the progress and nothing else.
    let mut complete = BTreeMap::new();
    for name in AGENT_COLLECTIONS {
        complete.insert((*name).to_owned(), CollectionProgress::Complete);
    }
    let mut ready = state_with_progress(complete);
    ready.phase = Phase::DestinationValidated;
    ready
        .write(workspace.path(), &lock)
        .expect("a finished rebuild may advance the phase");
}

#[test]
fn progress_keys_must_be_exactly_the_agent_collections() {
    let workspace = tempfile::tempdir().expect("workspace");
    let lock = lock(workspace.path());

    let mut missing = fresh_progress();
    missing.remove("_procedural_memory");
    let refusal = state_with_progress(missing)
        .write(workspace.path(), &lock)
        .expect_err("a journal silently dropping a collection would skip its rebuild");
    assert!(
        refusal.contains("_procedural_memory"),
        "the refusal must name the absent collection: {refusal}"
    );

    let mut extra = fresh_progress();
    extra.insert(
        "_not_a_collection".to_owned(),
        CollectionProgress::Facts { cursor: None },
    );
    let refusal = state_with_progress(extra)
        .write(workspace.path(), &lock)
        .expect_err("a journal tracking an unknown collection describes work nobody will do");
    assert!(
        refusal.contains("_not_a_collection"),
        "the refusal must name the unknown collection: {refusal}"
    );
}
