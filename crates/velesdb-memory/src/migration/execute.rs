//! The operator's path into the rebuild (#1762, PR C2b).
//!
//! [`super::rebuild`] takes staged handles and a journal and drives the pass;
//! this module is everything an operator's `migrate-embeddings` invocation
//! needs BEFORE that call can be made honestly: the diagnosis, the regime
//! resolution, a destination that provably is not somebody else's data, a
//! journal workspace outside both stores, and the lock. Each refusal here is
//! distinct on purpose — an operator has to know whether they were refused by
//! the regime, by the destination, or by another migration's lock, because the
//! recovery for each is different.
//!
//! # Where the pieces live
//!
//! The journal workspace is a SIBLING of the destination, named after it
//! (`<destination>.migration-journal`). Inside the source it would violate the
//! read-only contract; inside the destination it would be swept along by the
//! eventual switch rename (C3), which must move the rebuilt store and nothing
//! else. A sibling survives the switch, which matters because the phases
//! after the switch are journalled too.
//!
//! # What execute stops short of
//!
//! The pass ends with the journal at [`Phase::Prepared`] and every collection
//! `Complete`. Validation of the destination and the switch itself are the
//! next PR's work; see [`NOT_YET_SWITCHABLE`](super::not_yet_switchable).

use std::path::{Path, PathBuf};

use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

use super::diagnosis::{diagnose, DiagnosisReport, TargetContract};
use super::rebuild::{
    rebuild, RebuildDestination, RebuildJournal, RebuildOutcome, RebuildSource, VectorPolicy,
};
use super::state::{CollectionProgress, MigrationLock, MigrationState, Phase};
use super::strategy::Resolution;
use crate::embedder::Embedder;

/// What one `execute` run did, and where its artefacts live.
#[derive(Debug)]
pub struct ExecuteOutcome {
    /// The diagnosis that gated the run.
    pub report: DiagnosisReport,
    /// What the pass wrote.
    pub rebuild: RebuildOutcome,
    /// The rebuilt store, still unswitched.
    pub destination: PathBuf,
    /// Where the journal (and the lock evidence) lives.
    pub workspace: PathBuf,
}

/// Diagnose, stage, lock, rebuild, release — the whole non-dry-run path.
///
/// A pre-existing journal at the derived workspace is resumed, provided it
/// describes this exact source, fingerprint and target; anything else about it
/// is a refusal, never an overwrite.
///
/// # Errors
/// Returns [`crate::MemoryError`] when the regime resolution refuses, the
/// destination holds data no journal accounts for, the journal describes a
/// different migration, the lock is held or left from a crash, or the pass
/// itself fails.
pub fn execute(
    store: &Path,
    scratch_parent: &Path,
    target: &TargetContract,
    destination: &Path,
    embedder: &dyn Embedder,
    batch: usize,
) -> Result<ExecuteOutcome, crate::MemoryError> {
    let report = diagnose(store, scratch_parent, target, Some(destination))?;
    let staging = stage(&report, destination)?;

    let lock =
        MigrationLock::acquire(&staging.workspace, "migrate-embeddings").map_err(query_error)?;
    let result = execute_locked(
        &report,
        target,
        destination,
        &staging.workspace,
        &lock,
        &ExecutePass {
            embedder,
            batch,
            resuming: staging.resuming,
            settled_fingerprint: &staging.settled_fingerprint,
        },
    );
    // Release on BOTH paths. The fail-closed evidence a dropped lock leaves is
    // for crashes — a run that reached a clean `Err` has nothing for the
    // operator to acknowledge, and making them `rm` a lock file after every
    // refusal would train them to do it after real crashes too.
    let released = lock.release().map_err(query_error);
    let rebuild = result?;
    released?;
    Ok(ExecuteOutcome {
        report,
        rebuild,
        destination: destination.to_path_buf(),
        workspace: staging.workspace,
    })
}

/// What the pre-lock staging established.
struct Staging {
    workspace: PathBuf,
    resuming: bool,
    settled_fingerprint: String,
}

/// Everything between the diagnosis and the lock: the regime gate, the settle,
/// the journal workspace and the destination checks.
fn stage(report: &DiagnosisReport, destination: &Path) -> Result<Staging, crate::MemoryError> {
    if let Resolution::Refuse { because, requested } = &report.resolution {
        return Err(query_error(format!(
            "the requested regime '{}' cannot run: {because:?}. Nothing was \
             created; re-run --dry-run for the full report",
            regime_word(*requested),
        )));
    }
    // Settle the source BEFORE fingerprinting it: the first open of a store
    // compacts its WAL into materialised index files, so the tree after an
    // open is not the tree before it — and the rebuild below opens it. A
    // fingerprint taken pre-settle would therefore never match on resume,
    // refusing every legitimately interrupted migration as "source changed".
    // A second open is proven to change nothing (see
    // `settling_a_store_is_idempotent_which_the_resume_fingerprint_rests_on`),
    // which is what makes the settled fingerprint stable. The settle itself is
    // what any daemon start performs; it runs only after the regime gate, so a
    // refusal leaves the source byte-identical.
    {
        let _settle = Database::open(&report.source_path)?;
    }
    let settled_fingerprint = super::filesystem::fingerprint(&report.source_path)?;
    let workspace = journal_workspace(destination)?;
    let resuming = workspace.join(super::state::STATE_FILE).exists();
    ensure_destination(destination, resuming)?;
    Ok(Staging {
        workspace,
        resuming,
        settled_fingerprint,
    })
}

/// The run's inputs beyond the diagnosis: how to embed, how much per batch,
/// whether a journal already existed, and the post-settle fingerprint the
/// journal carries (the diagnosis's own fingerprint predates the settle and
/// would never match on resume).
struct ExecutePass<'a> {
    embedder: &'a dyn Embedder,
    batch: usize,
    resuming: bool,
    settled_fingerprint: &'a str,
}

fn execute_locked(
    report: &DiagnosisReport,
    target: &TargetContract,
    destination: &Path,
    workspace: &Path,
    lock: &MigrationLock,
    pass: &ExecutePass<'_>,
) -> Result<RebuildOutcome, crate::MemoryError> {
    let mut state = journal_entry(
        report,
        target,
        workspace,
        lock,
        pass.resuming,
        pass.settled_fingerprint,
    )?;
    let policy = match report.resolution {
        Resolution::Reuse => VectorPolicy::Reuse,
        Resolution::Reembed { .. } => VectorPolicy::Reembed(pass.embedder),
        Resolution::Refuse { .. } => {
            unreachable!("execute gated Refuse before the lock was taken")
        }
    };
    let Some(source_dimension) = report.source_dimension else {
        return Err(query_error(
            "the source collections do not establish one shared dimension, so \
             no AgentMemory view can open them; the diagnosis carries the \
             details",
        ));
    };

    let source_db = std::sync::Arc::new(Database::open(&report.source_path)?);
    let source_memory =
        AgentMemory::with_dimension(std::sync::Arc::clone(&source_db), source_dimension)?;
    let destination_db = std::sync::Arc::new(Database::open(destination)?);
    let destination_memory =
        AgentMemory::with_dimension(std::sync::Arc::clone(&destination_db), target.dimension)?;

    rebuild(
        &RebuildSource {
            db: &source_db,
            memory: &source_memory,
        },
        &RebuildDestination {
            db: &destination_db,
            memory: &destination_memory,
        },
        &mut state,
        &RebuildJournal { workspace, lock },
        &policy,
        pass.batch,
    )
}

/// Read-and-verify the existing journal, or write the first entry.
///
/// Both directions use the SETTLED fingerprint, never the diagnosis's: the
/// diagnosis fingerprinted the tree before the settle compacted it.
fn journal_entry(
    report: &DiagnosisReport,
    target: &TargetContract,
    workspace: &Path,
    lock: &MigrationLock,
    resuming: bool,
    settled_fingerprint: &str,
) -> Result<MigrationState, crate::MemoryError> {
    if resuming {
        let state = MigrationState::read(workspace)
            .map_err(query_error)?
            .ok_or_else(|| {
                query_error(format!(
                    "the journal at {} disappeared between inspection and locking",
                    workspace.display()
                ))
            })?;
        state
            .may_resume(
                &report.source_path,
                settled_fingerprint,
                &target.model,
                target.dimension,
            )
            .map_err(query_error)?;
        return Ok(state);
    }
    let state = MigrationState {
        format_version: super::state::STATE_FORMAT_VERSION,
        phase: Phase::Prepared,
        source_path: report.source_path.clone(),
        source_fingerprint: settled_fingerprint.to_owned(),
        target_model: target.model.clone(),
        target_dimension: target.dimension,
        progress: super::enumeration::AGENT_COLLECTIONS
            .iter()
            .map(|name| {
                (
                    (*name).to_owned(),
                    CollectionProgress::Facts { cursor: None },
                )
            })
            .collect(),
    };
    state.write(workspace, lock).map_err(query_error)?;
    Ok(state)
}

/// The operator's word for a regime, as they typed it on the CLI.
fn regime_word(strategy: super::strategy::Strategy) -> &'static str {
    match strategy {
        super::strategy::Strategy::Auto => "auto",
        super::strategy::Strategy::Reuse => "reuse",
        super::strategy::Strategy::Reembed => "reembed",
    }
}

/// The journal's home: a sibling of the destination, named after it.
fn journal_workspace(destination: &Path) -> Result<PathBuf, crate::MemoryError> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            query_error(format!(
                "the destination {} has no usable directory name to derive the \
                 journal workspace from",
                destination.display()
            ))
        })?;
    let workspace = destination.with_file_name(format!("{name}.migration-journal"));
    std::fs::create_dir_all(&workspace).map_err(|err| {
        query_error(format!(
            "cannot create the journal workspace {}: {err}",
            workspace.display()
        ))
    })?;
    Ok(workspace)
}

/// Create the destination, or verify that what is there is ours to continue.
fn ensure_destination(destination: &Path, resuming: bool) -> Result<(), crate::MemoryError> {
    if !destination.exists() {
        std::fs::create_dir_all(destination).map_err(|err| {
            query_error(format!(
                "cannot create the destination {}: {err}",
                destination.display()
            ))
        })?;
        return Ok(());
    }
    if resuming {
        // The journal accounts for whatever the interrupted run left here, and
        // `reinsert_batch`'s collision refusal is what protects each id.
        return Ok(());
    }
    let mut entries = std::fs::read_dir(destination).map_err(|err| {
        query_error(format!(
            "cannot inspect the destination {}: {err}",
            destination.display()
        ))
    })?;
    if entries.next().is_some() {
        return Err(query_error(format!(
            "the destination {} already holds data and no migration journal \
             accounts for it; rebuilding into it could mix two stores, so \
             choose an empty destination or remove it deliberately",
            destination.display()
        )));
    }
    Ok(())
}

fn query_error(message: impl Into<String>) -> crate::MemoryError {
    velesdb_core::Error::Query(message.into()).into()
}
