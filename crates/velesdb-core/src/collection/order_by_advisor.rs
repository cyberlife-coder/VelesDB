//! Recommendation-only advisor for scalar `ORDER BY <field>` queries
//! (EPIC-081 phase 3a).
//!
//! Records every query whose shape is eligible for the index-backed
//! `ORDER BY <field> LIMIT k` fast path (EPIC-081 phase 2) but which fell back
//! to the exhaustive sort because the sort field has no *fully covering*
//! secondary index. An operator reads [`order_by_index_advice`] to learn which
//! fields would benefit from `CREATE INDEX`.
//!
//! This advisor is **observation-only**: it never creates, drops, or mutates an
//! index, and never alters a query result. Auto-creation is intentionally out
//! of scope — `create_index` re-backfills the index `O(n)`, which on the query
//! thread would stall the very query that triggered it.
//!
//! [`order_by_index_advice`]: crate::VectorCollection::order_by_index_advice

use std::collections::HashMap;

/// Upper bound on the number of distinct fields tracked. `ORDER BY` field names
/// come from the raw query AST and are not validated to exist, so an
/// adversarial stream of `ORDER BY <fresh_name> LIMIT k` queries could otherwise
/// grow the map without bound for the collection's lifetime. This cap is far
/// above any real schema's count of distinct `ORDER BY` fields; once reached,
/// further *unseen* fields are dropped (already-tracked fields keep counting).
const MAX_TRACKED_FIELDS: usize = 1024;

/// Why an eligible `ORDER BY <field>` query could not use the ordered-index
/// fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderByIndexState {
    /// No secondary index exists on the field. `CREATE INDEX (<field>)` would
    /// enable the `O(log n + k)` ordered-index fast path.
    Missing,
    /// A secondary index exists but does not fully cover the collection (some
    /// rows lack the field, or hold a non-primitive value), so the fast path
    /// still declines. The gap is the data, not a missing index — creating
    /// another index would not help. Surfaced as a distinct state so a
    /// never-firing index is not invisible.
    BuiltButUncovered,
}

/// A single index recommendation for a scalar `ORDER BY` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderByIndexSuggestion {
    /// The payload field used in `ORDER BY`.
    pub field: String,
    /// How many eligible `ORDER BY <field>` queries fell back to the exhaustive
    /// path since the collection was opened.
    pub observed_count: u64,
    /// Whether an index is missing or present-but-not-covering.
    pub state: OrderByIndexState,
}

/// Tracks fall-back observations per field. Cheap: one `HashMap<String, u64>`
/// guarded by the collection's advisor `RwLock` (lock order position 7).
#[derive(Debug, Default)]
pub struct OrderByIndexAdvisor {
    /// Field name -> count of eligible `ORDER BY` queries that fell back.
    observations: HashMap<String, u64>,
}

impl OrderByIndexAdvisor {
    /// Records one eligible `ORDER BY <field>` query that fell back to the
    /// exhaustive path. Only ever called from the fast-path decline branch, so
    /// it never observes a query the route already served. Bounded by
    /// [`MAX_TRACKED_FIELDS`]: an unseen field past the cap is dropped, but
    /// already-tracked fields keep counting (`saturating_add` guards the
    /// physically-unreachable `u64` overflow).
    pub(crate) fn observe(&mut self, field: &str) {
        if let Some(count) = self.observations.get_mut(field) {
            *count = count.saturating_add(1);
        } else if self.observations.len() < MAX_TRACKED_FIELDS {
            self.observations.insert(field.to_owned(), 1);
        }
    }

    /// Fields observed at least `min_observations` times, with their counts,
    /// sorted by descending count then field name for deterministic output.
    pub(crate) fn observed(&self, min_observations: u64) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = self
            .observations
            .iter()
            .filter(|&(_, &count)| count >= min_observations)
            .map(|(field, &count)| (field.clone(), count))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }
}
