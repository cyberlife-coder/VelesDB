//! The switch (#1762, PR C3): put the validated destination where the source
//! was, and free the archive.
//!
//! Two renames, each journalled AFTER it happens — a journal that ran ahead of
//! the disk would let a resume skip a rename that never happened. Every crash
//! window therefore leaves the disk one step ahead of the journal at most, and
//! the re-run's job is to recognise that step and journal it late, never to
//! undo it: going backwards would discard work the disk already holds, and
//! [`Phase::may_follow`] refuses journal regressions anyway.
//!
//! # The one ambiguous landing spot, and its discriminator
//!
//! After the second rename and before its journal entry, the disk says: a
//! store at the source's name, an archive beside it, no destination — the
//! shape [`SwitchState`] calls two authorities and refuses, because from the
//! filesystem alone nothing distinguishes "the destination was activated" from
//! "something else sat down at the source's name". This module has one more
//! fact available: validation stamped the destination with the TARGET's
//! provenance, and the old source does not carry that stamp. An occupant WITH
//! the stamp is the activated destination (continue); an occupant without it
//! is an impostor (refuse, both stores intact).
//!
//! # What commit means
//!
//! [`Phase::recovery`] has said since C1 that advancing from
//! `DestinationActivated` "frees the archive". Commit is that advance: verify
//! the activated store opens and carries the stamp, delete the archive, then
//! journal `Committed`. Deletion before journalling, so a crash between the
//! two re-runs an idempotent commit instead of leaving a freed archive that
//! the journal still believes in.

use super::query_error;
use std::path::{Path, PathBuf};

use super::execute::journal_workspace;
use super::state::{MigrationLock, MigrationState, Phase, SwitchState};

#[path = "switchover/live.rs"]
mod live;
pub(crate) use live::{
    finalize_staged_live_switch, rollback_staged_live_switch, stage_live_switch,
};

/// The archive slot: a sibling of the source, named after it.
pub const ARCHIVE_SUFFIX: &str = ".archive";

/// What a completed switch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOutcome {
    /// Where the rebuilt store now lives — the source's own path.
    pub activated: PathBuf,
    /// The archive slot that held the old source until commit freed it.
    pub archive: PathBuf,
}

/// Drive the switch from wherever the journal stands to `Committed`.
///
/// Requires a validated destination ([`Phase::DestinationValidated`] or
/// later). Each step is act-then-journal; a re-run after a crash recognises
/// completed-but-unjournalled steps and journals them late.
///
/// # Errors
/// Returns [`crate::MemoryError`] when the journal is missing or earlier than
/// validation, the archive slot is occupied, the disk is in a shape no step of
/// this migration produces, or a rename, deletion, or journal write fails.
pub fn switch_over(store: &Path, destination: &Path) -> Result<SwitchOutcome, crate::MemoryError> {
    run_switch(store, destination, true, false)
}

pub(crate) fn commit_retained_switch(
    store: &Path,
    destination: &Path,
) -> Result<SwitchOutcome, crate::MemoryError> {
    run_switch(store, destination, false, true)
}

fn run_switch(
    store: &Path,
    destination: &Path,
    verify_open: bool,
    allow_completed: bool,
) -> Result<SwitchOutcome, crate::MemoryError> {
    let workspace = journal_workspace(destination)?;
    let lock = MigrationLock::acquire(&workspace, "migrate-switch").map_err(query_error)?;
    let result = switch_locked(
        store,
        destination,
        &workspace,
        &lock,
        verify_open,
        allow_completed,
    );
    super::execute::reconcile(result, lock.release())
}

fn switch_locked(
    store: &Path,
    destination: &Path,
    workspace: &Path,
    lock: &MigrationLock,
    verify_open: bool,
    allow_completed: bool,
) -> Result<SwitchOutcome, crate::MemoryError> {
    let mut state = entry_state(workspace, allow_completed)?;
    let slots = Slots::resolve(store, &state, destination)?;
    loop {
        match state.phase {
            Phase::Prepared => {
                return Err(query_error(
                    "the destination has not been validated; validate it first \
                     — the switch moves stores and must not be the step that \
                     discovers a bad rebuild",
                ));
            }
            Phase::DestinationValidated => step_archive(&slots, &mut state, workspace, lock)?,
            Phase::SourceArchived => step_activate(&slots, &mut state, workspace, lock)?,
            Phase::DestinationActivated => {
                step_commit(&slots, &mut state, workspace, lock, verify_open)?;
            }
            Phase::Committed => {
                return Ok(outcome(&slots));
            }
        }
    }
}

fn outcome(slots: &Slots) -> SwitchOutcome {
    SwitchOutcome {
        activated: slots.source.clone(),
        archive: slots.archive.clone(),
    }
}

/// Read the journal, refusing an absent one and a migration already done.
///
/// The Committed check runs only at ENTRY: a run that just advanced to
/// `Committed` reports its outcome, while a fresh invocation on a committed
/// journal has nothing left to do — the recovery table has said since C1 that
/// replaying a step would act on a store that is already the new one.
fn entry_state(
    workspace: &Path,
    allow_completed: bool,
) -> Result<MigrationState, crate::MemoryError> {
    let state = MigrationState::read(workspace)
        .map_err(query_error)?
        .ok_or_else(|| {
            query_error(format!(
                "no migration journal at {}; there is nothing to switch",
                workspace.display()
            ))
        })?;
    if state.phase == Phase::Committed && !allow_completed {
        return Err(query_error(
            "this migration is complete; there is nothing left to switch, and \
             replaying a step would act on a store that is already the new one",
        ));
    }
    Ok(state)
}

/// The three fixed paths of the switch, resolved once and checked against the
/// journal's identity.
struct Slots {
    source: PathBuf,
    archive: PathBuf,
    destination: PathBuf,
}

impl Slots {
    fn resolve(
        store: &Path,
        state: &MigrationState,
        destination: &Path,
    ) -> Result<Self, crate::MemoryError> {
        let source = canonical_slot(store)?;
        if source != state.source_path {
            return Err(query_error(format!(
                "this journal describes a migration of '{}', and the request \
                 names '{}'; a switch cannot be transferred between stores",
                state.source_path.display(),
                source.display()
            )));
        }
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                query_error(format!(
                    "the source {} has no usable directory name to derive the \
                     archive slot from",
                    source.display()
                ))
            })?;
        Ok(Self {
            archive: source.with_file_name(format!("{name}{ARCHIVE_SUFFIX}")),
            destination: canonical_slot(destination)?,
            source,
        })
    }

    fn on_disk(&self) -> SwitchState {
        SwitchState {
            source: self.source.exists(),
            archive: self.archive.exists(),
            destination: self.destination.exists(),
        }
    }
}

/// First rename: the source steps aside into the archive slot.
fn step_archive(
    slots: &Slots,
    state: &mut MigrationState,
    workspace: &Path,
    lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    archive_source(slots, state)?;
    advance(state, Phase::SourceArchived, workspace, lock)
}

fn archive_source(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    match slots.on_disk() {
        // The step is pending: the slot must be free, or renaming would eat
        // whatever sits there — and the source must still BE the store the
        // journal describes. A stale journal (a crashed migration, overtaken
        // by later writes or by a whole second migration) would otherwise
        // archive the LIVE store here and destroy it at commit.
        SwitchState {
            source: true,
            archive: false,
            destination: true,
        } => {
            require_journalled_fingerprint(&slots.source, state, "source")?;
            rename_durably(&slots.source, &slots.archive)?;
        }
        // The step already happened and its journal entry did not — the crash
        // window. Journal it late; undoing a rename the disk holds would
        // discard the step. The archive must fingerprint as the journalled
        // source, or what was archived is not what this journal describes.
        SwitchState {
            source: false,
            archive: true,
            destination: true,
        } => {
            require_journalled_fingerprint(&slots.archive, state, "archive")?;
        }
        SwitchState {
            source: true,
            archive: true,
            ..
        } => {
            return Err(query_error(format!(
                "the archive slot {} is already occupied; renaming the source \
                 over it would destroy whatever it holds — move it aside \
                 deliberately, or remove it if it is yours to remove",
                slots.archive.display()
            )));
        }
        other => return Err(unrecognised_disk(other, Phase::DestinationValidated)),
    }
    Ok(())
}

/// Second rename: the destination takes the source's name.
fn step_activate(
    slots: &Slots,
    state: &mut MigrationState,
    workspace: &Path,
    lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    activate_destination(slots, state)?;
    advance(state, Phase::DestinationActivated, workspace, lock)
}

fn activate_destination(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    match slots.on_disk() {
        SwitchState {
            source: false,
            archive: true,
            destination: true,
        } => {
            require_journalled_fingerprint(&slots.archive, state, "archive")?;
            rename_durably(&slots.destination, &slots.source)?;
        }
        // Source-name occupied, archive present, destination gone: either the
        // rename happened and its journal entry did not, or something else sat
        // down at the source's name. Two proofs discriminate: the occupant
        // carries the TARGET's provenance stamp (validation wrote it on the
        // destination and nothing else has it), and the archive fingerprints
        // as the journalled source (so this archive belongs to THIS journal,
        // not to a later migration toward the same target).
        SwitchState {
            source: true,
            archive: true,
            destination: false,
        } => late_activation(slots, state)?,
        // The recovery table's manual advice for this phase is "move the
        // archive back to the source's name". An operator who followed it and
        // re-ran presents the journal AHEAD of the disk: source restored,
        // archive slot empty, destination intact. Redo the first rename —
        // fingerprint-checked like any other — and continue, rather than
        // stranding the migration in a shape everything refuses.
        SwitchState {
            source: true,
            archive: false,
            destination: true,
        } => redo_after_manual_restore(slots, state)?,
        other => return Err(unrecognised_disk(other, Phase::SourceArchived)),
    }
    Ok(())
}

/// The second rename already happened and only the journal is behind. Two
/// proofs before the late journal entry: the occupant carries the TARGET's
/// stamp, and the archive fingerprints as THIS journal's source.
fn late_activation(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    require_target_stamp(slots, state)?;
    require_journalled_fingerprint(&slots.archive, state, "archive")
}

/// The operator followed the recovery table's manual advice and moved the
/// archive back; the journal is AHEAD of the disk. Redo both renames —
/// fingerprint-checked — instead of stranding the migration in a shape
/// everything refuses.
fn redo_after_manual_restore(
    slots: &Slots,
    state: &MigrationState,
) -> Result<(), crate::MemoryError> {
    require_journalled_fingerprint(&slots.source, state, "restored source")?;
    rename_durably(&slots.source, &slots.archive)?;
    rename_durably(&slots.destination, &slots.source)
}

/// Commit: verify the activated store, free the archive, journal the end.
fn step_commit(
    slots: &Slots,
    state: &mut MigrationState,
    workspace: &Path,
    lock: &MigrationLock,
    verify_open: bool,
) -> Result<(), crate::MemoryError> {
    require_target_stamp(slots, state)?;
    if verify_open {
        let _opens = velesdb_core::Database::open(&slots.source)?;
    }
    if slots.archive.exists() {
        // The archive is the ONLY copy of the old data, and no flock stops a
        // rename or a recursive delete (measured in review: a daemon that
        // opened the source in a lock-free window keeps writing into the
        // archive after the first rename, and remove_dir_all succeeds under
        // it, unlinking its inodes silently). Before destruction, the archive
        // must still fingerprint as the settled source the journal knows —
        // anything else means writes landed here that exist nowhere else.
        require_journalled_fingerprint(&slots.archive, state, "archive")?;
        std::fs::remove_dir_all(&slots.archive).map_err(|err| {
            query_error(format!(
                "the activated store is verified but the archive {} could not \
                 be freed: {err}; nothing is lost — re-run the switch",
                slots.archive.display()
            ))
        })?;
    }
    advance(state, Phase::Committed, workspace, lock)
}

/// The tree at `path` must still fingerprint as the source this journal was
/// written about. This is the switch's identity check — the stamp says WHAT
/// KIND of store an occupant is, the fingerprint says WHICH store a tree is,
/// and only the second can tell this migration's source from a later state of
/// the same path.
fn require_journalled_fingerprint(
    path: &Path,
    state: &MigrationState,
    role: &str,
) -> Result<(), crate::MemoryError> {
    let observed = super::filesystem::fingerprint(path)?;
    if observed == state.source_fingerprint {
        return Ok(());
    }
    Err(query_error(format!(
        "the {role} at {} no longer fingerprints as the store this journal \
         describes — something wrote to it after the journal was written. \
         Nothing was moved or deleted; a store that changed hands must be \
         inspected, not migrated on a stale journal",
        path.display(),
    )))
}

/// The occupant of the source's name must carry the TARGET's provenance stamp
/// — the one validation wrote on the destination and the old source never had.
fn require_target_stamp(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    let stamped = crate::embedding_provenance::read(&slots.source)
        .map_err(query_error)?
        .filter(|stamp| {
            stamp.model == state.target_model && stamp.dimension == state.target_dimension
        });
    if stamped.is_some() {
        return Ok(());
    }
    Err(query_error(format!(
        "what occupies {} does not carry the target's provenance stamp \
         ('{}', {} dimensions), so it cannot be assumed to be the activated \
         destination; the archive and the destination are left untouched — \
         inspect {} by hand",
        slots.source.display(),
        state.target_model,
        state.target_dimension,
        slots.source.display(),
    )))
}

fn advance(
    state: &mut MigrationState,
    phase: Phase,
    workspace: &Path,
    lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    state.phase = phase;
    state.write(workspace, lock).map_err(query_error)
}

fn unrecognised_disk(observed: SwitchState, at: Phase) -> crate::MemoryError {
    let recovery = observed.recovery();
    query_error(format!(
        "the journal stands at {at:?} but the disk does not match any step of \
         this migration (source: {}, archive: {}, destination: {}). The \
         recovery table says: {recovery:?}",
        observed.source, observed.archive, observed.destination,
    ))
}

/// A path's canonical form, computable even while its slot is empty: the
/// parent canonicalises, the final component is carried verbatim.
fn canonical_slot(path: &Path) -> Result<PathBuf, crate::MemoryError> {
    let name = path
        .file_name()
        .ok_or_else(|| query_error(format!("{} has no final path component", path.display())))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let base = match parent {
        Some(parent) => parent
            .canonicalize()
            .map_err(|err| query_error(format!("cannot resolve {}: {err}", parent.display())))?,
        None => std::env::current_dir()
            .map_err(|err| query_error(format!("cannot resolve the working directory: {err}")))?,
    };
    Ok(base.join(name))
}

/// A rename, made durable: the parent directory is synced so the entry's move
/// survives a power cut, not just a process crash.
fn rename_durably(from: &Path, to: &Path) -> Result<(), crate::MemoryError> {
    std::fs::rename(from, to).map_err(|err| {
        query_error(format!(
            "cannot rename {} to {}: {err}",
            from.display(),
            to.display()
        ))
    })?;
    if let Some(parent) = to.parent() {
        let directory = std::fs::File::open(parent).map_err(|err| {
            query_error(format!(
                "cannot open {} to sync it: {err}",
                parent.display()
            ))
        })?;
        directory.sync_all().map_err(|err| {
            query_error(format!(
                "the rename of {} is visible but not yet durable: {err}; do \
                 not power off before re-running",
                to.display()
            ))
        })?;
    }
    Ok(())
}
