//! The rebuild pass (#1762, PR C2b): drive the proven parts over a real store.
//!
//! Everything this module calls was proven separately — facts round-trip
//! ([`super::enumeration`]), edges round-trip ([`super::edges`]), the journal
//! can only advance ([`super::state`]). What it adds is the loop, and the loop
//! is where a migration actually dies: between a batch landing and its
//! checkpoint, between the last fact and the first edge, between one
//! collection and the next. Every one of those gaps is covered by the journal
//! contract — a crash replays at most one batch, and the replay is tolerated
//! by construction because [`super::reinsert_batch`] refuses to overwrite and
//! reports collisions instead.
//!
//! # Checkpoint order: destination first, journal second
//!
//! Every batch is written to the destination BEFORE its cursor reaches the
//! journal. The other order would be quietly catastrophic: a journal that runs
//! ahead of the destination makes a resume SKIP facts that never landed, and
//! nothing downstream can tell a skipped fact from a fact the source never
//! had. With this order the crash window replays work instead of losing it,
//! and replays are visible (counted as collisions) rather than silent.
//!
//! # What this module deliberately does not do
//!
//! It does not choose the vector policy — [`super::resolve`] does, one place,
//! tested as one rule. It does not create the destination, acquire the lock,
//! or write the first journal entry — the caller stages those, because each is
//! refused differently and a monolithic "prepare everything" would blur whose
//! refusal the operator is reading. And it does not touch the phase: the pass
//! runs strictly inside [`Phase::Prepared`], and leaving that phase is the
//! validation-and-switch work of a later PR.

use std::path::Path;

use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

use super::edges::{export_edges_verified, reinsert_edges, same_edge_tuples};
use super::enumeration::{reinsert_batch, scroll_page, RawFact, AGENT_COLLECTIONS};
use super::state::{CollectionProgress, MigrationLock, MigrationState, Phase};
use crate::embedder::Embedder;

/// The source store, opened read-only in spirit: nothing here writes to it.
pub struct RebuildSource<'a> {
    /// The database the fact walk scrolls.
    pub db: &'a Database,
    /// The agent view the edge export reads through.
    pub memory: &'a AgentMemory,
}

/// The destination store, created and sized by the caller.
pub struct RebuildDestination<'a> {
    /// The database facts are reinserted into.
    pub db: &'a Database,
    /// The agent view edges are reinserted through.
    pub memory: &'a AgentMemory,
}

/// Where the journal lives and the proof we may write it.
pub struct RebuildJournal<'a> {
    /// The workspace holding `migration-state.json` — never the source store.
    pub workspace: &'a Path,
    /// The exclusive lock the caller acquired on that workspace.
    pub lock: &'a MigrationLock,
}

/// Which vector each reinserted fact carries.
///
/// `Reuse` copies the source vector verbatim and never reads the fact's text;
/// `Reembed` reads the fact's `content` and asks the target embedder. This is
/// an enum rather than a closure so the pass cannot be handed a policy the
/// regime resolution did not produce.
pub enum VectorPolicy<'a> {
    /// The compatibility-proven regime: the source vectors ARE the target's.
    Reuse,
    /// Every other regime: the target embedder produces every vector.
    Reembed(&'a dyn Embedder),
}

/// What a completed pass did — counts, not verdicts. The verdict is the
/// destination re-reads performed along the way.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RebuildOutcome {
    /// Facts written to the destination by this run.
    pub facts: u64,
    /// Ids that were already occupied — nonzero exactly when this run replayed
    /// a batch an interrupted predecessor had landed but not journalled.
    pub collisions: u64,
    /// Edges reinserted (idempotent replays included).
    pub edges: u64,
}

/// Run the rebuild to completion, resuming from whatever `state` records.
///
/// Collections already `Complete` are skipped; a collection at `Edges` re-runs
/// its (idempotent) edge pass; a collection at `Facts` resumes strictly after
/// its journalled cursor.
///
/// # Errors
/// Returns [`crate::MemoryError`] if `state` is not in [`Phase::Prepared`], if
/// any read, embed, write, or journal step fails, or if a destination re-read
/// disagrees with the source export.
pub fn rebuild(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    policy: &VectorPolicy<'_>,
    batch: usize,
) -> Result<RebuildOutcome, crate::MemoryError> {
    rebuild_inner(source, destination, state, journal, policy, batch, None)
}

/// [`rebuild`], with an injected stop after `stop_after_batches` batches.
///
/// The stop fires after a batch is reinserted and BEFORE its checkpoint is
/// journalled — the widest window a real crash can hit. This is the seam the
/// interruption tests drive; production callers go through [`rebuild`], which
/// never stops early — hence the `cfg(test)`: no production build carries it.
#[cfg(test)]
pub(crate) fn rebuild_with_stop(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    policy: &VectorPolicy<'_>,
    batch: usize,
    stop_after_batches: Option<u64>,
) -> Result<RebuildOutcome, crate::MemoryError> {
    rebuild_inner(
        source,
        destination,
        state,
        journal,
        policy,
        batch,
        stop_after_batches,
    )
}

/// Counters shared across collections, plus the run-wide stop seam.
#[derive(Default)]
struct Run {
    facts: u64,
    collisions: u64,
    edges: u64,
    batches: u64,
    stop_after_batches: Option<u64>,
}

fn rebuild_inner(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    policy: &VectorPolicy<'_>,
    batch: usize,
    stop_after_batches: Option<u64>,
) -> Result<RebuildOutcome, crate::MemoryError> {
    if state.phase != Phase::Prepared {
        return Err(query_error(format!(
            "the rebuild runs strictly inside {:?}, and this journal stands at \
             {:?}; a pass that ran after validation would invalidate what was \
             validated",
            Phase::Prepared,
            state.phase
        )));
    }
    let mut run = Run {
        stop_after_batches,
        ..Run::default()
    };
    for name in AGENT_COLLECTIONS {
        let step = Step {
            collection: name,
            policy,
            batch,
        };
        rebuild_collection(source, destination, state, journal, &step, &mut run)?;
    }
    Ok(RebuildOutcome {
        facts: run.facts,
        collisions: run.collisions,
        edges: run.edges,
    })
}

/// The immutable context of one collection's pass.
struct Step<'a> {
    collection: &'a str,
    policy: &'a VectorPolicy<'a>,
    batch: usize,
}

fn rebuild_collection(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    step: &Step<'_>,
    run: &mut Run,
) -> Result<(), crate::MemoryError> {
    let current = *state.progress.get(step.collection).ok_or_else(|| {
        query_error(format!(
            "the journal carries no progress entry for '{}'; refusing to \
             invent one mid-pass",
            step.collection
        ))
    })?;
    match current {
        CollectionProgress::Complete => return Ok(()),
        CollectionProgress::Edges => {}
        CollectionProgress::Facts { cursor } => {
            walk_facts(source, destination, state, journal, step, run, cursor)?;
            journal_progress(state, journal, step.collection, CollectionProgress::Edges)?;
        }
    }
    run.edges += edge_pass(source, destination, step)?;
    journal_progress(
        state,
        journal,
        step.collection,
        CollectionProgress::Complete,
    )
}

fn walk_facts(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    step: &Step<'_>,
    run: &mut Run,
    mut cursor: Option<u64>,
) -> Result<(), crate::MemoryError> {
    loop {
        let (facts, next) = scroll_page(source.db, step.collection, cursor, step.batch)?;
        if facts.is_empty() {
            return Ok(());
        }
        let mut pairs: Vec<(RawFact, Vec<f32>)> = Vec::with_capacity(facts.len());
        for fact in facts {
            let vector = vector_for(step.policy, &fact)?;
            pairs.push((fact, vector));
        }
        let outcome = reinsert_batch(destination.db, step.collection, &pairs)?;
        run.facts += outcome.inserted;
        run.collisions += outcome.collisions.len() as u64;
        run.batches += 1;
        if run.stop_after_batches == Some(run.batches) {
            return Err(query_error(format!(
                "rebuild interrupted by the injected stop after {} batches; the \
                 destination holds this batch and the journal does not — the \
                 exact window a crash leaves, and what a resume replays",
                run.batches
            )));
        }
        let Some(next) = next else {
            return Ok(());
        };
        cursor = Some(next);
        journal_progress(
            state,
            journal,
            step.collection,
            CollectionProgress::Facts { cursor: Some(next) },
        )?;
    }
}

/// The vector a fact carries at the destination, per the resolved regime.
fn vector_for(policy: &VectorPolicy<'_>, fact: &RawFact) -> Result<Vec<f32>, crate::MemoryError> {
    match policy {
        VectorPolicy::Reuse => Ok(fact.source_vector.clone()),
        VectorPolicy::Reembed(embedder) => {
            let payload: serde_json::Value =
                serde_json::from_str(&fact.payload).map_err(|err| {
                    query_error(format!(
                        "fact {} carries unreadable payload: {err}",
                        fact.id
                    ))
                })?;
            let Some(content) = payload.get("content").and_then(serde_json::Value::as_str) else {
                return Err(query_error(format!(
                    "fact {} carries no `content` text, so `reembed` cannot \
                     produce its vector; skipping it would silently drop the \
                     fact and re-using its old vector would mix models, so the \
                     pass stops here",
                    fact.id
                )));
            };
            embedder
                .embed(content)
                .map_err(|err| query_error(format!("embedding fact {} failed: {err}", fact.id)))
        }
    }
}

/// Export the source's edges, put them back, and re-read the destination.
fn edge_pass(
    source: &RebuildSource<'_>,
    destination: &RebuildDestination<'_>,
    step: &Step<'_>,
) -> Result<u64, crate::MemoryError> {
    let exported = export_edges_verified(source.memory, source.db, step.collection, step.batch)?;
    let outcome = reinsert_edges(destination.memory, step.collection, &exported)?;
    let back = export_edges_verified(
        destination.memory,
        destination.db,
        step.collection,
        step.batch,
    )?;
    same_edge_tuples(&exported, &back).map_err(|difference| {
        // Honesty about what this mismatch can mean: the export, the
        // reinsertion and the re-read are three separate clock reads, and a
        // fact whose ABSOLUTE expiry falls between them shrinks one side
        // without anything being lost — the C2a lesson (two walks must share
        // one snapshot) cannot apply here because a write sits between the
        // walks. Distinguishing that transient from real loss mechanically is
        // the validation pass's job (C3); until then the pass stops, says
        // both readings, and stays resumable.
        query_error(format!(
            "after reinsertion the destination's edges do not match the export \
             for '{}': {difference}. Either an edge was lost, or an endpoint's \
             absolute expiry passed between the export and the re-read. The \
             pass is resumable: re-run it, and a mismatch that PERSISTS across \
             runs is real loss",
            step.collection
        ))
    })?;
    Ok(outcome.inserted)
}

fn journal_progress(
    state: &mut MigrationState,
    journal: &RebuildJournal<'_>,
    collection: &str,
    progress: CollectionProgress,
) -> Result<(), crate::MemoryError> {
    state.progress.insert(collection.to_owned(), progress);
    state
        .write(journal.workspace, journal.lock)
        .map_err(query_error)
}

fn query_error(message: impl Into<String>) -> crate::MemoryError {
    velesdb_core::Error::Query(message.into()).into()
}
