//! GATE 11 — the operator's one command, end to end (#1762, PR C3).
//!
//! `execute`, `validate_destination` and `switch_over` are each proven; what
//! an operator actually runs is one command that chains them — and the chain
//! has a failure mode none of the parts has: after the first rename, the
//! SOURCE PATH IS VACANT, so a re-run that blindly started from the diagnosis
//! would fail on an absent directory and never reach the switch that knows
//! how to continue. `migrate` reads the journal first and enters the chain
//! where the journal says, which is what makes "re-run the same command" a
//! true recovery instruction instead of a wall.

use super::execute::{root_with_source, NEW_DIM, SEEDED};
use super::switchover::journal_phase;
use super::*;
use crate::embedder::HashEmbedder;

fn migrate(root: &std::path::Path) -> Result<super::super::MigrateOutcome, crate::MemoryError> {
    let embedder = HashEmbedder::new(NEW_DIM);
    super::super::migrate(
        &root.join("store"),
        root,
        &TargetContract::automatic("hash", NEW_DIM),
        &root.join("rebuilt"),
        &embedder,
        1024,
    )
}

#[test]
fn a_fresh_migrate_runs_the_whole_chain_to_committed() {
    let root = root_with_source();
    let outcome = migrate(root.path()).expect("migrate");

    let executed = outcome.executed.expect("a fresh run rebuilds, and says so");
    assert_eq!(executed.rebuild.facts, SEEDED);
    let validated = outcome
        .validated
        .expect("a fresh run validates, and says so");
    assert_eq!(validated.facts, SEEDED);

    let store = root.path().join("store");
    assert_eq!(
        outcome.switched.activated,
        store.canonicalize().expect("exists"),
        "the chain must end with the rebuilt store live at the source's path"
    );
    let journalled = MigrationState::read(&executed.workspace)
        .expect("read journal")
        .expect("journal exists");
    assert_eq!(journalled.phase, Phase::Committed);
}

#[test]
fn migrate_resumes_past_a_crash_at_source_archived() {
    // The crash the chain must survive: the switch archived the source and
    // died. The source path is now VACANT — a re-run that started from the
    // diagnosis would fail on the absent directory before ever reaching the
    // code that knows how to continue. The journal is what routes around it.
    let root = root_with_source();
    let executed = super::execute::run(root.path()).expect("execute");
    super::super::validate_destination(
        &root.path().join("store"),
        &executed.destination,
        &TargetContract::automatic("hash", NEW_DIM),
        1024,
    )
    .expect("validate");
    let store = root.path().join("store");
    std::fs::rename(&store, root.path().join("store.archive")).expect("crashed first rename");
    journal_phase(&executed.workspace, Phase::SourceArchived);

    let outcome = migrate(root.path()).expect("the re-run must resume at the switch");
    assert!(
        outcome.executed.is_none() && outcome.validated.is_none(),
        "a resume past validation must not re-run the earlier stages — with \
         the source vacant it could not, and pretending it did would misreport"
    );
    assert_eq!(
        outcome.switched.activated,
        store.canonicalize().expect("exists"),
        "the switch must have completed from where the journal stood"
    );
    assert_eq!(
        MigrationState::read(&executed.workspace)
            .expect("read journal")
            .expect("journal exists")
            .phase,
        Phase::Committed
    );
}

/// An embedder that proves it was never consulted: any call is a test failure.
struct PanickingEmbedder(usize);

impl crate::Embedder for PanickingEmbedder {
    fn dimension(&self) -> usize {
        self.0
    }

    fn embed(&self, _text: &str) -> Result<Vec<f32>, crate::EmbedError> {
        panic!("reuse must never call the embedder — this call IS the regression");
    }
}

#[test]
fn reuse_migrates_end_to_end_without_ever_calling_the_embedder() {
    // #1815's arbitration in one sentence: reuse is NOT an embedding
    // migration. The doc says it, the strategy module enforces when it is
    // allowed — and this pins that the whole chain (rebuild, witness,
    // validation, switch) structurally cannot embed under reuse, by running
    // it with an embedder whose every call panics.
    let root = root_with_source();
    let store = root.path().join("store");
    crate::embedding_provenance::write(
        &store,
        &crate::embedding_provenance::EmbeddingProvenance::new("hash", DIM),
    )
    .expect("record provenance matching the target");

    let embedder = PanickingEmbedder(DIM);
    let outcome = super::super::migrate(
        &store,
        root.path(),
        &TargetContract::automatic("hash", DIM),
        &root.path().join("rebuilt"),
        &embedder,
        1024,
    )
    .expect("a provenance-matched store migrates under reuse");

    let executed = outcome.executed.expect("a fresh run rebuilds");
    assert!(
        matches!(executed.report.resolution, Resolution::Reuse),
        "positive control: the regime must actually have been reuse, or the \
         panicking embedder proved nothing — got {:?}",
        executed.report.resolution
    );
    let db = velesdb_core::Database::open(&store).expect("activated store opens");
    let facts = super::super::enumerate_by_cursor(&db, "_semantic_memory", 1024).expect("walk");
    assert_eq!(
        facts.len() as u64,
        SEEDED,
        "every fact crossed, no embedder involved"
    );
}
