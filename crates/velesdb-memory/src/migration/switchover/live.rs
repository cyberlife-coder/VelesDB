use std::path::Path;

use super::{
    activate_destination, advance, archive_source, entry_state, journal_workspace, late_activation,
    outcome, query_error, rename_durably, require_journalled_fingerprint, unrecognised_disk,
    MigrationLock, MigrationState, Phase, Slots, SwitchOutcome, SwitchState,
};

pub(crate) fn stage_live_switch(
    store: &Path,
    destination: &Path,
) -> Result<SwitchOutcome, crate::MemoryError> {
    run_live_action(store, destination, stage_locked, false)
}

pub(crate) fn rollback_staged_live_switch(
    store: &Path,
    destination: &Path,
) -> Result<SwitchOutcome, crate::MemoryError> {
    run_live_action(store, destination, rollback_locked, false)
}

pub(crate) fn finalize_staged_live_switch(
    store: &Path,
    destination: &Path,
) -> Result<SwitchOutcome, crate::MemoryError> {
    run_live_action(store, destination, finalize_locked, true)
}

type LiveAction =
    fn(&Slots, &mut MigrationState, &Path, &MigrationLock) -> Result<(), crate::MemoryError>;

fn run_live_action(
    store: &Path,
    destination: &Path,
    action: LiveAction,
    allow_completed: bool,
) -> Result<SwitchOutcome, crate::MemoryError> {
    let workspace = journal_workspace(destination)?;
    let lock = MigrationLock::acquire(&workspace, "migrate-live-switch").map_err(query_error)?;
    let result = live_action_locked(
        store,
        destination,
        &workspace,
        &lock,
        action,
        allow_completed,
    );
    crate::migration::execute::reconcile(result, lock.release())
}

fn live_action_locked(
    store: &Path,
    destination: &Path,
    workspace: &Path,
    lock: &MigrationLock,
    action: LiveAction,
    allow_completed: bool,
) -> Result<SwitchOutcome, crate::MemoryError> {
    let mut state = entry_state(workspace, allow_completed)?;
    let slots = Slots::resolve(store, &state, destination)?;
    action(&slots, &mut state, workspace, lock)?;
    Ok(outcome(&slots))
}

fn stage_locked(
    slots: &Slots,
    state: &mut MigrationState,
    _workspace: &Path,
    _lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    require_phase(state.phase, Phase::DestinationValidated, "stage")?;
    archive_source(slots, state)?;
    activate_destination(slots, state)
}

fn rollback_locked(
    slots: &Slots,
    state: &mut MigrationState,
    _workspace: &Path,
    _lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    require_phase(state.phase, Phase::DestinationValidated, "roll back")?;
    match slots.on_disk() {
        SwitchState {
            source: true,
            archive: false,
            destination: true,
        } => Ok(()),
        SwitchState {
            source: false,
            archive: true,
            destination: true,
        } => restore_first_rename(slots, state),
        SwitchState {
            source: true,
            archive: true,
            destination: false,
        } => restore_both_renames(slots, state),
        other => Err(unrecognised_disk(other, Phase::DestinationValidated)),
    }
}

fn restore_first_rename(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    require_journalled_fingerprint(&slots.archive, state, "archive")?;
    rename_durably(&slots.archive, &slots.source)
}

fn restore_both_renames(slots: &Slots, state: &MigrationState) -> Result<(), crate::MemoryError> {
    late_activation(slots, state)?;
    rename_durably(&slots.source, &slots.destination)?;
    rename_durably(&slots.archive, &slots.source)
}

fn finalize_locked(
    slots: &Slots,
    state: &mut MigrationState,
    workspace: &Path,
    lock: &MigrationLock,
) -> Result<(), crate::MemoryError> {
    if state.phase == Phase::Committed {
        return Ok(());
    }
    late_activation(slots, state)?;
    if state.phase == Phase::DestinationValidated {
        advance(state, Phase::SourceArchived, workspace, lock)?;
    }
    require_phase(state.phase, Phase::SourceArchived, "finalize")?;
    advance(state, Phase::DestinationActivated, workspace, lock)
}

fn require_phase(
    actual: Phase,
    expected: Phase,
    operation: &str,
) -> Result<(), crate::MemoryError> {
    if actual == expected {
        return Ok(());
    }
    Err(query_error(format!(
        "cannot {operation} live switch from journal phase {actual:?}; expected {expected:?}"
    )))
}
