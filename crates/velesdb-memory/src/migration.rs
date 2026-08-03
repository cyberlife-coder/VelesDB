//! Read-only diagnosis of a store that a changed embedding model has made
//! unopenable, and the feasibility proof the rebuild depends on (#1762, PR A).
//!
//! # What this module is NOT
//!
//! It does not migrate, does not switch anything over, and never writes to the
//! store it inspects. Producing a [`MigrationState`] is a later step behind an
//! explicit prepare command; a diagnosis yields a [`DiagnosisReport`] and
//! nothing else.
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
//!   ([`enumerate_collection`]) — complete and deterministic, but quadratic,
//!   and BOUNDED: the pipeline clamps `limit + offset` to 100_000 and goes
//!   silently empty past that mark;
//! * the collection's own `scroll_batch` ([`enumerate_by_cursor`]) — a cursor
//!   keyed on the point id, exclusive and ascending, which bypasses the query
//!   pipeline and so carries neither the clamp nor the re-sort.
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

use serde_json::Value;

/// Collections `AgentMemory` opens, all at the same dimension — so any one of
/// them refusing the new dimension makes the whole store unopenable, and an
/// inventory that skipped the empty ones would under-report the work.
pub const AGENT_COLLECTIONS: &[&str] =
    &["_semantic_memory", "_episodic_memory", "_procedural_memory"];

/// One fact as the rebuild will need to re-create it.
///
/// The old vector is deliberately absent: it is recomputed by the new
/// embedder, and carrying it would only invite writing it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFact {
    /// The id the fact must keep. Not a suggestion.
    pub id: u64,
    /// The whole stored payload, reserved keys included — `content` and every
    /// `_veles_*` key travel here verbatim.
    pub payload: String,
}

/// Whether a capability the rebuild depends on is established, or missing.
///
/// `Missing` is a full stop, not a warning: PR B does not start while one is
/// outstanding, and no identifier mapping is invented to work around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Established by running it, with the evidence that established it.
    Proven {
        /// What was run, and what it produced.
        evidence: String,
    },
    /// Not available, with the blocker named.
    Missing {
        /// Why the rebuild cannot rely on this.
        blocker: String,
    },
}

impl Capability {
    /// Whether this capability may be relied on.
    #[must_use]
    pub fn is_proven(&self) -> bool {
        matches!(self, Self::Proven { .. })
    }
}

/// Read every fact of `collection` out of `db`, in pages of `page`.
///
/// `ORDER BY id` is load-bearing, not decoration. Measured on 2026-08-03: the
/// natural scan order is NOT id order — a seeded store answered
/// `[1, 2, 3, 100, 4, 5, 6, 7]` — so an `OFFSET` walk over the unordered scan
/// pages by *position* in a layout the engine is free to rearrange
/// (`reorder_for_locality` does exactly that). Ordering first makes each page
/// boundary a value.
///
/// This is only sound because the walk runs under the exclusive store lock with
/// nothing else writing.
///
/// Prefer [`enumerate_by_cursor`] for a rebuild. This walk is kept as the
/// independent second path the cursor is checked against — two routes through
/// the engine agreeing is evidence, where a cursor compared against a list this
/// module built would only prove self-consistency — and it is bounded at
/// 100_000 facts, so it is not the one to migrate a real store with.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the scan cannot be parsed or executed.
pub fn enumerate_collection(
    db: &velesdb_core::Database,
    collection: &str,
    page: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let mut out: Vec<RawFact> = Vec::new();
    let mut offset = 0usize;
    loop {
        let batch = enumerate_page(db, collection, page, offset)?;
        if batch.is_empty() {
            break;
        }
        let returned = batch.len();
        out.extend(batch);
        if returned < page {
            break;
        }
        offset += page;
    }
    Ok(out)
}

/// One page of `collection`, starting at `offset` — the unit a checkpoint
/// resumes from, and what makes the walk above interruptible rather than
/// all-or-nothing.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the scan cannot be parsed or executed.
pub fn enumerate_page(
    db: &velesdb_core::Database,
    collection: &str,
    page: usize,
    offset: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let sql = format!("SELECT * FROM {collection} ORDER BY id LIMIT {page} OFFSET {offset}");
    let query = velesdb_core::velesql::Parser::parse(&sql)
        .map_err(|e| velesdb_core::Error::Query(e.to_string()))?;
    let hits = db.execute_query(&query, &std::collections::HashMap::new())?;
    Ok(hits
        .into_iter()
        .map(|hit| RawFact {
            id: hit.point.id,
            payload: hit
                .point
                .payload
                .as_ref()
                .map_or_else(|| Value::Null.to_string(), std::string::ToString::to_string),
        })
        .collect())
}

/// One batch of `collection`, starting strictly after `cursor`, with the cursor
/// to resume from.
///
/// This is the enumeration the rebuild should use, and it is NOT the `VelesQL`
/// walk above. The engine already carries a cursor primitive — `scroll_batch`
/// on the collection itself, keyed on the point id, exclusive, ascending — and
/// it bypasses the query pipeline entirely. That matters twice over:
///
/// * The query pipeline asks the collection for `limit + offset` rows and
///   clamps the total to `MAX_LIMIT` (100_000), so an `OFFSET` walk goes blind
///   past that mark. A cursor never accumulates an offset, so it has no such
///   bound.
/// * `WHERE id > n` really is unavailable — filters read the payload and the id
///   is not in it — but that only ever ruled out expressing the cursor *in
///   `VelesQL`*. It never ruled out the cursor.
///
/// Returns the batch and the cursor to pass next; `None` means the collection
/// is exhausted.
///
/// # Errors
/// Returns [`crate::MemoryError`] if the collection is absent, is of a kind
/// that does not scroll, or if the scroll itself fails.
pub fn scroll_page(
    db: &velesdb_core::Database,
    collection: &str,
    cursor: Option<u64>,
    batch: usize,
) -> Result<(Vec<RawFact>, Option<u64>), crate::MemoryError> {
    let any = db.get_any_collection(collection).ok_or_else(|| {
        velesdb_core::Error::Query(format!("collection `{collection}` not found"))
    })?;
    let scrolled = match &any {
        velesdb_core::AnyCollection::Vector(c) => c.scroll_batch(cursor, batch, None),
        velesdb_core::AnyCollection::Graph(c) => c.scroll_batch(cursor, batch, None),
        velesdb_core::AnyCollection::Metadata(c) => c.scroll_batch(cursor, batch, None),
        _ => {
            return Err(velesdb_core::Error::Query(format!(
                "collection `{collection}` is of a kind that does not scroll"
            ))
            .into())
        }
    }?;
    let facts = scrolled
        .points
        .into_iter()
        .map(|point| RawFact {
            id: point.id,
            payload: point
                .payload
                .as_ref()
                .map_or_else(|| Value::Null.to_string(), std::string::ToString::to_string),
        })
        .collect();
    Ok((facts, scrolled.next_cursor))
}

/// Read every fact of `collection` out of `db` by cursor, in batches of `batch`.
///
/// Termination is on an exhausted cursor or an empty batch — never on a SHORT
/// one. `scroll_batch` keeps scanning candidate ids until it has `batch_size`
/// live points or the ids run out, and it skips TTL-expired points on the way,
/// so a short batch does mean the end; but the two stop conditions are cheap and
/// the walk should not depend on that internal detail holding.
///
/// Skipping the expired is the behaviour this rebuild wants, not a quirk to work
/// around: the contract says an export must exclude already-expired facts so the
/// rebuild cannot resurrect them.
///
/// # Errors
/// Returns [`crate::MemoryError`] if any batch fails.
pub fn enumerate_by_cursor(
    db: &velesdb_core::Database,
    collection: &str,
    batch: usize,
) -> Result<Vec<RawFact>, crate::MemoryError> {
    let mut out: Vec<RawFact> = Vec::new();
    let mut cursor: Option<u64> = None;
    loop {
        let (facts, next) = scroll_page(db, collection, cursor, batch)?;
        if facts.is_empty() {
            break;
        }
        out.extend(facts);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
