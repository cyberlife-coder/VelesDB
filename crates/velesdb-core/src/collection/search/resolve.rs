//! Shared result resolution helpers for search methods.
//!
//! Eliminates duplicated point hydration logic (fetching vector + payload
//! from storage and building `SearchResult`) across vector, text, batch,
//! and sparse search modules.
//!
//! These helpers are ready for adoption by search submodules.
//! Currently tested directly; callers will migrate in a follow-up.

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::point::{Point, SearchResult};
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};

/// Hydrates, in `pairs` order, the first `limit` points that are rows of
/// `universe`, are not TTL-expired at `now_secs` and that `accept` takes.
/// `accept` sees the point's id and its payload (`None` when the point has
/// none), so a filter built from `VelesQL` can read its `id` column.
///
/// A vector ranking passes the vectors' universe (`RowScope::Embedded`): a
/// candidate without a vector is no row of it. A text search passes the
/// collection's rows (`RowScope::All`), so a metadata row or a graph node
/// without an embedding is hydrated with an empty `vector`, as SELECT reads
/// it.
///
/// A point's payload is read and judged before its vector is copied, so a
/// filtered search copies the vectors of the `limit` points it returns, not
/// of every candidate it rejects.
pub(crate) fn hydrate_accepted(
    pairs: impl IntoIterator<Item = (u64, f32)>,
    limit: usize,
    now_secs: u64,
    universe: RowUniverse,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
    accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    for (id, score) in pairs {
        if results.len() >= limit {
            break;
        }
        let payload = payload_storage.retrieve(id).ok().flatten();
        if is_payload_expired(payload.as_ref(), now_secs) || !accept(id, payload.as_ref()) {
            continue;
        }
        let vector = vector_storage.retrieve(id).ok().flatten();
        if !universe.admits(vector.is_some(), payload.is_some()) {
            continue;
        }
        let point = Point {
            id,
            vector: vector.unwrap_or_default(),
            payload,
            sparse_vectors: None,
        };
        results.push(SearchResult::new(point, score));
    }
    results
}

/// Hydrates `rows`, in order, as SELECT reads them: rows the caller admitted
/// under the storage guards it passes, so none is judged again — a row
/// without a vector (a metadata row, a graph node without an embedding)
/// gets an empty one. What a ranked walk hands back
/// (`Collection::ranked_hits`).
pub(crate) fn hydrate_rows(
    rows: Vec<(u64, f32)>,
    vector_storage: &dyn VectorStorage,
    payload_storage: &dyn PayloadStorage,
) -> Vec<SearchResult> {
    rows.into_iter()
        .map(|(id, score)| {
            let point = Point {
                id,
                vector: vector_storage
                    .retrieve(id)
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                payload: payload_storage.retrieve(id).ok().flatten(),
                sparse_vectors: None,
            };
            SearchResult::new(point, score)
        })
        .collect()
}

impl Collection {
    /// Hydrates, in `pairs` order, the first `limit` that are rows of
    /// `universe` and not TTL-expired, as SELECT reads them: one deleted or
    /// expired since it was ranked is skipped before the `limit` is counted,
    /// so the next one takes its place, never returned empty. What every
    /// fused ranking resolves by — a multi-query search's, a dense+sparse
    /// hybrid's.
    ///
    /// LOCK ORDER: vector_storage(2) → payload_storage(3), both read.
    pub(crate) fn first_rows(
        &self,
        pairs: impl IntoIterator<Item = (u64, f32)>,
        limit: usize,
        universe: RowUniverse,
    ) -> Vec<SearchResult> {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        hydrate_accepted(
            pairs,
            limit,
            now_unix_secs(),
            universe,
            &*vector_storage,
            &*payload_storage,
            |_, _| true,
        )
    }
}

/// Sorts `ScoredResult` values by score according to metric direction.
///
/// Uses unstable sort: equal-score tie-breaking order is irrelevant for ranking.
pub(crate) fn sort_scored_by_metric(results: &mut [ScoredResult], higher_is_better: bool) {
    results
        .sort_unstable_by(|a, b| crate::distance::rank_scores(a.score, b.score, higher_is_better));
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
