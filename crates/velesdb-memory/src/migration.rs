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

mod diagnosis;
mod diagnostic_copy;
mod enumeration;
mod filesystem;
mod state;
mod strategy;

pub use diagnosis::{
    diagnose, same_filesystem, Capability, CollectionInventory, DiagnosisReport, SourceProvenance,
    TargetContract, TtlSummary, DIAGNOSIS_FORMAT_VERSION,
};
pub use enumeration::{
    enumerate_by_cursor, enumerate_collection, enumerate_page, reinsert, reinsert_batch,
    scroll_page, BatchReinsertion, RawFact, Reinsertion, AGENT_COLLECTIONS,
};
pub use filesystem::{bytes_on_disk, fingerprint};
pub use state::{
    MigrationLock, MigrationState, Phase, Recovery, SwitchState, LOCK_FILE, PHASES, STATE_FILE,
    STATE_FORMAT_VERSION, STATE_TEMP_FILE,
};
pub use strategy::{assess, resolve, Compatibility, Resolution, Strategy};

#[cfg(test)]
mod tests;
