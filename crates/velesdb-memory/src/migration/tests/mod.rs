//! The feasibility gate for #1762 (PR A).
//!
//! Whether a rebuild is even possible turns on one question the architecture
//! does not answer on paper: can every fact be read back out with its `u64` id,
//! its content, its ordinary metadata, its RESERVED metadata and its absolute
//! expiry — completely, once each, across page boundaries?
//!
//! `MemoryStore` cannot: every read is by id or a top-`k` vector search. Two
//! paths below it can — a `VelesQL` scan walked by `LIMIT`/`OFFSET`, and the
//! collection's own `scroll_batch` cursor — and this file exists because
//! neither is trustworthy on the strength of compiling. Each is run against a
//! seeded store, compared field by field, and compared against the other.
//!
//! The measurements are the point, and they did not agree with the plan: the
//! `OFFSET` walk is not merely quadratic but BOUNDED, going silently empty past
//! offset `100_000`, while the cursor — which an earlier reading of this same
//! architecture concluded did not exist — reads a 100 001-fact store whole in
//! about a second. See `past_the_ceiling_the_offset_walk_truncates_and_the_cursor_does_not`.
//!
//! A `Missing` verdict here stops the whole project before PR B rather than
//! producing an identifier mapping nobody asked for.

use super::*;
use crate::storage::NativeStore;
use crate::{MemoryStore, Metadata};
use serde_json::Value;
use std::collections::BTreeSet;

mod diagnostic_copy;
mod execute;
mod rebuild;
mod rebuild_state;
mod state_persistence;

const DIM: usize = 4;
const EMBEDDING: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

/// Run the public diagnosis with an isolated staging parent.
fn diagnose(
    source: &std::path::Path,
    target_model: &str,
    target_dimension: usize,
    destination: Option<&std::path::Path>,
) -> Result<DiagnosisReport, crate::MemoryError> {
    let staging = tempfile::tempdir().expect("diagnostic staging");
    super::diagnose(
        source,
        staging.path(),
        &TargetContract::automatic(target_model, target_dimension),
        destination,
    )
}

/// Facts enough to cross a page boundary that is not a divisor of the count,
/// so an off-by-one in the walk shows up as a gap or a repeat.
const SEEDED: u64 = 7;
const PAGE: usize = 3;

/// Ids that are NOT contiguous and are NOT inserted in ascending order.
///
/// A fixture of `1..=n` written in order proves far less than it appears to:
/// insertion order, id order and physical order all coincide, so a walk that
/// paged by position would look correct. These do not coincide — `2000` is
/// written first and `7` last — which is what makes a gap or a repeat visible.
const SCRAMBLED: &[u64] = &[2000, 41, 999, 3, 17, 1_000_000, 58, 7];

fn meta(pairs: &[(&str, Value)]) -> Metadata {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

/// A store holding facts that carry every shape the rebuild must preserve:
/// an ordinary field, a RESERVED field, and one fact under a TTL.
///
/// The store handle is DROPPED before returning, and that is not tidiness: the
/// engine holds an exclusive `velesdb.lock` on the directory, so a second
/// handle cannot open it while the first lives. A diagnosis therefore only ever
/// runs against a store nothing else has open — which is the offline protocol
/// this migration is built on, observed here rather than assumed.
fn seeded() -> (tempfile::TempDir, Metadata) {
    let dir = tempfile::tempdir().expect("tempdir");
    let ttl_metadata;
    {
        let store = NativeStore::open(dir.path(), DIM).expect("open store");
        for id in 1..=SEEDED {
            store
                .store_with_metadata(
                    id,
                    &format!("fact number {id}"),
                    &EMBEDDING,
                    &meta(&[
                        ("project", Value::from("veles")),
                        ("_veles_hub", Value::Bool(id == 1)),
                    ]),
                )
                .expect("seed fact");
        }
        store
            .store_with_metadata_and_ttl(
                100,
                "a fact under a ttl",
                &EMBEDDING,
                &meta(&[("project", Value::from("veles"))]),
                3600,
            )
            .expect("seed ttl fact");
        ttl_metadata = store
            .get_metadata(100)
            .expect("read back")
            .expect("the ttl fact exists");
    }
    (dir, ttl_metadata)
}

/// Reopen the seeded directory as a bare `Database` — the read path a
/// diagnosis uses, which takes no dimension and so does not refuse.
fn database(dir: &tempfile::TempDir) -> velesdb_core::Database {
    velesdb_core::Database::open(dir.path()).expect("open database")
}

mod cli;
mod diagnosis;
mod edges;
mod enumeration;
mod performance;
mod preservation;
mod state;
mod strategy;
