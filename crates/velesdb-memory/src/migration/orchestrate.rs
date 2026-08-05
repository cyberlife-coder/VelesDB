//! The operator's one command, end to end (#1762, PR C3).
//!
//! `execute`, `validate_destination` and `switch_over` each acquire the lock,
//! do one phase's work, and release — independently provable, independently
//! refusable. What they cannot do alone is RESUME as a chain: after the
//! switch's first rename the source path is vacant, and a re-run that blindly
//! started from the diagnosis would fail on the absent directory before
//! reaching the one function that knows how to continue. This module reads
//! the journal FIRST and enters the chain where the journal says — which is
//! what makes "re-run the same command" a true recovery instruction.

use std::path::Path;

use super::diagnosis::TargetContract;
use super::execute::{execute, journal_workspace, ExecuteOutcome};
use super::query_error;
use super::state::{MigrationState, Phase};
use super::switchover::{switch_over, SwitchOutcome};
use super::validate::{validate_destination, ValidationOutcome};
use crate::embedder::Embedder;

/// What one `migrate` run did. The early stages are `None` exactly when the
/// journal routed past them — reporting a rebuild that did not run would be
/// misreporting, and with the source already archived it could not have run.
#[derive(Debug)]
pub struct MigrateOutcome {
    /// The rebuild, when this run performed it.
    pub executed: Option<ExecuteOutcome>,
    /// The validation, when this run performed it.
    pub validated: Option<ValidationOutcome>,
    /// The switch — every completed run ends here.
    pub switched: SwitchOutcome,
}

/// Rebuild, validate and switch — entering wherever the journal stands.
///
/// A journal already at [`Phase::DestinationValidated`] or beyond routes
/// straight to the switch: the earlier stages are journalled as done, and
/// after the first rename the source path no longer exists for them to run
/// against.
///
/// # Errors
/// Returns [`crate::MemoryError`] from whichever stage refuses; each stage
/// names itself, and re-running the same command resumes from the journal.
pub fn migrate(
    store: &Path,
    scratch_parent: &Path,
    target: &TargetContract,
    destination: &Path,
    embedder: &dyn Embedder,
    batch: usize,
) -> Result<MigrateOutcome, crate::MemoryError> {
    let (executed, validated) = if past_validation(destination)? {
        (None, None)
    } else {
        let executed = execute(store, scratch_parent, target, destination, embedder, batch)?;
        let validated = validate_destination(store, destination, target, batch)?;
        (Some(executed), Some(validated))
    };
    let switched = switch_over(store, destination)?;
    Ok(MigrateOutcome {
        executed,
        validated,
        switched,
    })
}

/// Whether the journal says validation already happened.
fn past_validation(destination: &Path) -> Result<bool, crate::MemoryError> {
    let workspace = journal_workspace(destination)?;
    let state = MigrationState::read(&workspace).map_err(query_error)?;
    Ok(state.is_some_and(|state| state.phase >= Phase::DestinationValidated))
}
