//! Read-only diagnosis of a store that a changed embedding model has made
//! unopenable, and the feasibility proof the rebuild depends on (#1762, PR A).
//!
//! # What this module is NOT
//!
//! It does not migrate, does not switch anything over, and never writes to the
//! store it inspects. Producing a [`crate::migration::MigrationState`] is a
//! later step behind an explicit prepare command; a diagnosis yields a
//! [`crate::migration::DiagnosisReport`] and nothing else.
//!
//! # Why a feasibility proof comes first
//!
//! A rebuild must re-insert every fact under its ORIGINAL `u64` id: edges are
//! `(id, from, to, relation)` with no vector of their own, entity hubs derive
//! their id from the topic, and the working-context index addresses facts by
//! id. Renumbering would silently sever all three. So before any rebuild code
//! is written, the architecture has to be shown to support reading every fact
//! back out — ids, content, ordinary metadata, RESERVED metadata and the
//! absolute expiry — and putting it back unchanged.
//!
//! `MemoryStore` offers no enumeration at all: every read is by id or a
//! top-`k` vector search, and `count()` counts without listing. Two paths down
//! into the engine do, and they are not equivalent:
//!
//! * a `VelesQL` scan with no vector predicate, walked by `LIMIT`/`OFFSET`
//!   ([`crate::migration::enumerate_collection`]) — complete and
//!   deterministic, but quadratic, and BOUNDED: the pipeline clamps
//!   `limit + offset` to 100_000 and goes silently empty past that mark;
//! * the collection's own `scroll_batch`
//!   ([`crate::migration::enumerate_by_cursor`]) — a cursor keyed on the point
//!   id, exclusive and ascending, which bypasses the query pipeline and so
//!   carries neither the clamp nor the re-sort.
//!
//! The first was written first because `WHERE id > n` genuinely does not work —
//! filters read the payload and the id is not in it. That ruled out expressing
//! a cursor *in `VelesQL`*; it did not rule out the cursor, and treating the
//! query language's limit as the architecture's limit is the error this module
//! now records rather than repeats.
//!
//! That either *parses* is not the proof. Both are measured by running them
//! against a seeded store and comparing what comes back, field by field, and
//! against each other.

// The `persistence` gate lives on the `pub mod migration;` declaration in
// `lib.rs`; repeating it here as an inner attribute is what `clippy::
// duplicated_attributes` fires on.

mod cli;
mod diagnosis;
mod diagnostic_copy;
mod edges;
mod enumeration;
mod execute;
mod filesystem;
#[allow(dead_code)] // Wired by the control-surface slice after this internal seam.
mod live;
mod orchestrate;
mod rebuild;
mod state;
mod strategy;
mod switchover;
mod validate;

pub use cli::{
    default_scratch_parent, dry_run, migration_complete_notice, parse as parse_migrate_args,
    refuses, render, require_destination, MigrateOptions,
};
pub use diagnosis::{
    diagnose, same_filesystem, Capability, CollectionInventory, DiagnosisReport, SourceProvenance,
    TargetContract, TtlSummary, DIAGNOSIS_FORMAT_VERSION,
};
pub use edges::{
    cross_check_edges, export_edges, export_edges_verified, reinsert_edges, EdgeReinsertion,
};
pub use enumeration::{
    enumerate_by_cursor, enumerate_collection, enumerate_page, reinsert, reinsert_batch,
    scroll_page, BatchReinsertion, RawFact, Reinsertion, AGENT_COLLECTIONS,
};
pub(crate) use execute::target_embedder_witness;
pub use execute::{execute, ExecuteOutcome};
pub use filesystem::{bytes_on_disk, fingerprint};
#[allow(unused_imports)] // Wired by the control-surface slice after this internal seam.
pub(crate) use live::prepare_live_switch;
pub use orchestrate::{migrate, MigrateOutcome};
#[cfg(test)]
pub(crate) use rebuild::rebuild_with_stop;
pub use rebuild::{
    rebuild, RebuildDestination, RebuildJournal, RebuildOutcome, RebuildSource, VectorPolicy,
};
pub use state::{
    CollectionProgress, MigrationLock, MigrationState, Phase, Recovery, SwitchState, LOCK_FILE,
    PHASES, STATE_FILE, STATE_FORMAT_VERSION, STATE_TEMP_FILE,
};
pub use strategy::{assess, resolve, Compatibility, Resolution, Strategy};
pub(crate) use switchover::{
    commit_retained_switch, finalize_staged_live_switch, rollback_staged_live_switch,
    stage_live_switch,
};
pub use switchover::{switch_over, SwitchOutcome, ARCHIVE_SUFFIX};
#[cfg(test)]
pub(crate) use validate::divergence_explained_by_expiry;
pub use validate::{validate_destination, ValidationOutcome};

/// The one conversion every migration module needs: a message become the
/// engine's query error, become this crate's. Defined once — six private
/// copies of it had already drifted into two signatures.
pub(crate) fn query_error(message: impl Into<String>) -> crate::MemoryError {
    velesdb_core::Error::Query(message.into()).into()
}

#[cfg(test)]
mod tests;
