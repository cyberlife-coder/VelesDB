//! The one expiry rule of a vector read: the rows it ranks are live ones.
//!
//! A deleted row leaves the index lazily and a TTL-expired one stays in
//! every store until a sweep removes it, so the index's nearest candidates
//! can be rows no read returns. Every vector read surface — the `search`
//! family, `search_ids`, the filtered search (and through it `NEAR … AND`
//! a filter, a filtered hybrid's vector branch, a multi-query's filtered
//! branches), the batch searches, a multi-query's branches, an anchored
//! search — ranks through [`Collection::nearest_live_rows`], so it answers
//! as the collection would once every expired row had been swept: a dead
//! candidate (no vector, or an expired payload) takes no place among the
//! `k` results, nor among the `budget` live candidates a filtered search
//! asks the index for.
//!
//! The index is asked for `budget` candidates (`k` unfiltered, the
//! filter's oversampling filtered) and judged under the storage read guards.
//! While dead candidates leave fewer than `k` accepted and fewer than
//! `budget` live, and the answer did not hold every stored vector, it is
//! asked again for twice as many — however short the answer was, when a
//! dead candidate is in it: an HNSW walk returns fewer candidates than asked
//! past the tombstones overwrites leave in its graph, which is no sign the
//! index ran out. A short answer without a dead candidate is the index's
//! own at its quality, which no expired row shrank, and settles the read.
//! That doubling is bounded
//! ([`Nearest::next_ask`]): once it would pass [`MAX_ASK`] or reach the
//! number of stored vectors — where an index walk visits most of the graph
//! anyway — the read scores every stored vector exactly instead
//! ([`Collection::score_current`], one chunk per hold of the vector guard)
//! and walks them best first with the ranked admission
//! (`collection/search/ranked.rs`): the exact `k` nearest live rows the
//! read accepts, at `O(n)` memory for the scores, never for the vectors.

use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::search::ranked::{Admission, Ranked};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::error::Result;
use crate::point::SearchResult;
use crate::scored_result::ScoredResult;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};

/// The most candidates a vector read asks the index for when it asks again
/// past dead ones; beyond it the read scores every stored vector exactly.
/// The largest `LIMIT` a query takes: past it an index walk is no cheaper
/// than the scan.
pub(super) const MAX_ASK: usize = crate::collection::search::query::MAX_LIMIT;

/// What a vector read asks for: the `k` nearest live rows it accepts among
/// the `budget` nearest live rows (`k` when it accepts every live row).
#[derive(Debug, Clone, Copy)]
pub(super) struct Nearest<'q> {
    pub(super) query: &'q [f32],
    pub(super) metric: DistanceMetric,
    pub(super) k: usize,
    pub(super) budget: usize,
}

impl<'q> Nearest<'q> {
    /// The `k` nearest live rows of `query`, every live row accepted.
    pub(super) fn rows(query: &'q [f32], metric: DistanceMetric, k: usize) -> Self {
        Self::judging(query, metric, k, k)
    }

    /// The `k` nearest live rows of `query` a filter accepts, among the
    /// `budget` nearest live rows (never fewer than `k`).
    pub(super) fn judging(
        query: &'q [f32],
        metric: DistanceMetric,
        k: usize,
        budget: usize,
    ) -> Self {
        Self {
            query,
            metric,
            k,
            budget: budget.max(k),
        }
    }

    /// How many candidates to ask the index for after `asked` left too few:
    /// twice as many, unless that passes [`MAX_ASK`] or reaches the
    /// `stored` vectors — `None`, the exact scan.
    pub(super) fn next_ask(asked: usize, stored: usize) -> Option<usize> {
        let next = asked.saturating_mul(2);
        (next <= MAX_ASK && next < stored).then_some(next)
    }
}

/// What judging one answer of the index found, under one hold of the
/// storage read guards: the candidates accepted, how many were live, how
/// many were dead, and how many store a vector, live or expired.
struct Judged {
    accepted: Vec<(u64, f32)>,
    live: usize,
    dead: usize,
    stored: usize,
}

impl Judged {
    /// Whether the answer settles the read: `k` accepted, `budget` live rows
    /// judged, an answer holding every one of the `stored` vectors — the
    /// index has no more to give — or an answer `short` of what was asked
    /// with no dead candidate in it: the index's own answer at its quality,
    /// which no dead row took a place in, as it would answer once every
    /// expired row is swept.
    ///
    /// A short answer holding a dead candidate settles nothing: an HNSW walk
    /// returns fewer candidates than asked past the tombstones overwrites
    /// leave in its graph, and the dead rows it found instead may be all it
    /// found — it was taken for every stored vector once, and the read
    /// returned nothing while live rows matched.
    fn settles(&self, want: &Nearest<'_>, short: bool, stored: usize) -> bool {
        self.accepted.len() >= want.k
            || self.live >= want.budget
            || self.stored >= stored
            || (short && self.dead == 0)
    }
}

/// Judges `candidates`, best first, under the caller's storage guards: a
/// candidate without a vector or with an expired payload is dead, counted
/// apart and skipped; a live one counts, and is kept when `accept` takes it — until
/// `k` are kept. Every candidate the index returned is judged, however many
/// live ones that makes: a search merging pending points unfiltered returns
/// more than it asked for, and the filter may refuse those nearest.
fn judge(
    candidates: &[ScoredResult],
    want: &Nearest<'_>,
    accept: &impl Fn(u64, Option<&serde_json::Value>) -> bool,
    vector_storage: &MmapStorage,
    payload_storage: &LogPayloadStorage,
    now_secs: u64,
) -> Judged {
    let mut judged = Judged {
        accepted: Vec::new(),
        live: 0,
        dead: 0,
        stored: 0,
    };
    for candidate in candidates {
        if judged.accepted.len() >= want.k {
            break;
        }
        if !vector_storage.contains(candidate.id) {
            judged.dead += 1;
            continue;
        }
        judged.stored += 1;
        let payload = payload_storage.retrieve(candidate.id).ok().flatten();
        if crate::collection::expiry::is_payload_expired(payload.as_ref(), now_secs) {
            judged.dead += 1;
            continue;
        }
        judged.live += 1;
        if accept(candidate.id, payload.as_ref()) {
            judged.accepted.push((candidate.id, candidate.score));
        }
    }
    judged
}

impl Collection {
    /// The `k` nearest live rows `nearest(n)` finds, hydrated and tagged with
    /// their `vector_score` ([`Self::nearest_live_rows`]).
    ///
    /// # Errors
    ///
    /// What `nearest` returns.
    pub(super) fn nearest_live(
        &self,
        want: &Nearest<'_>,
        nearest: impl Fn(usize) -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<SearchResult>> {
        self.nearest_live_where(want, |_, _| true, nearest)
    }

    /// The `k` nearest live rows `accept` takes among the `budget` nearest
    /// live rows `nearest(n)` finds, hydrated and tagged with their
    /// `vector_score` ([`Self::nearest_live_rows`]).
    ///
    /// # Errors
    ///
    /// What `nearest` returns.
    pub(super) fn nearest_live_where(
        &self,
        want: &Nearest<'_>,
        accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
        nearest: impl Fn(usize) -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<SearchResult>> {
        let mut results =
            self.nearest_live_rows(want, accept, nearest, |kept, vectors, payloads| {
                super::resolve::hydrate_rows(kept, vectors, payloads)
            })?;
        super::tag_vector_component_scores(&mut results);
        Ok(results)
    }

    /// The ids and scores of the `k` nearest live rows `nearest(n)` finds
    /// ([`Self::nearest_live_rows`]): no vector is copied.
    ///
    /// # Errors
    ///
    /// What `nearest` returns.
    pub(super) fn nearest_live_ids(
        &self,
        want: &Nearest<'_>,
        nearest: impl Fn(usize) -> Result<Vec<ScoredResult>>,
    ) -> Result<Vec<ScoredResult>> {
        self.nearest_live_rows(
            want,
            |_, _| true,
            nearest,
            |kept, _, _| {
                kept.into_iter()
                    .map(|(id, score)| ScoredResult::new(id, score))
                    .collect()
            },
        )
    }

    /// The `k` nearest live rows `accept` takes among the `budget` nearest
    /// live rows `nearest(n)` finds, best first, handed to `finish` under the
    /// storage read guards they were judged under (module doc). `nearest(n)`
    /// asks the index for its `n` nearest candidates, the pending points
    /// merged; it is asked again for more while dead candidates crowd the
    /// answer, and past [`Nearest::next_ask`]'s bound the read scans every
    /// stored vector exactly ([`Self::exact_nearest_live`]).
    ///
    /// # Errors
    ///
    /// What `nearest` returns.
    ///
    /// LOCK ORDER: what `nearest` takes, released; then vector_storage(2) →
    /// payload_storage(3), both read, per answer judged.
    pub(super) fn nearest_live_rows<T>(
        &self,
        want: &Nearest<'_>,
        accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
        nearest: impl Fn(usize) -> Result<Vec<ScoredResult>>,
        finish: impl Fn(Vec<(u64, f32)>, &MmapStorage, &LogPayloadStorage) -> T,
    ) -> Result<T> {
        let mut asked = want.budget;
        loop {
            let mut candidates = nearest(asked)?;
            index_answered();
            let short = candidates.len() < asked;
            super::resolve::sort_scored_by_metric(&mut candidates, want.metric.higher_is_better());
            let vector_storage = self.storage.vector_storage.read();
            let payload_storage = self.storage.payload_storage.read();
            let now = crate::collection::expiry::now_unix_secs();
            let judged = judge(
                &candidates,
                want,
                &accept,
                &vector_storage,
                &payload_storage,
                now,
            );
            if judged.settles(want, short, vector_storage.len()) {
                return Ok(finish(judged.accepted, &vector_storage, &payload_storage));
            }
            drop(payload_storage);
            drop(vector_storage);
            match Nearest::next_ask(asked, self.vector_count()) {
                Some(next) => asked = next,
                None => return Ok(self.exact_nearest_live(want, accept, finish)),
            }
        }
    }

    /// The exact `k` nearest live rows `accept` takes: every stored vector
    /// scored on what storage holds for it now, one chunk per hold of the
    /// vector guard ([`Self::score_current`]), then walked best first —
    /// a tie by ascending id — with the ranked admission, a batch of
    /// candidates per hold (`Collection::walk_admitted`). What a vector read
    /// falls back to once dead candidates crowded the index's answers past
    /// [`Nearest::next_ask`]'s bound. It judges every live row it needs, not
    /// only the `budget` nearest: the exact answer of a filtered read.
    ///
    /// LOCK ORDER: vector_storage(2), read, released; then as
    /// `Collection::walk_admitted`.
    fn exact_nearest_live<T>(
        &self,
        want: &Nearest<'_>,
        accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
        finish: impl Fn(Vec<(u64, f32)>, &MmapStorage, &LogPayloadStorage) -> T,
    ) -> T {
        #[cfg(test)]
        EXACT_SCANS.with(|count| count.set(count.get() + 1));
        let ids = self.storage.vector_storage.read().ids();
        let scored = self
            .score_current(want.query, &ids)
            .into_iter()
            .map(|result| (result.id, result.score))
            .collect();
        let ranked = Ranked::by_metric(scored, want.metric.higher_is_better());
        let admission = Admission {
            rows: RowUniverse::Vectors,
            within: |_| true,
            accept,
        };
        self.walk_admitted(ranked, want.k, &admission, |kept, vectors, payloads, _| {
            finish(kept, vectors, payloads)
        })
    }
}

#[cfg(test)]
thread_local! {
    /// Run once, the next time a vector read on this thread has the index's
    /// answer and holds no storage guard ([`on_index_answered`]).
    static ON_ANSWER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `hook` once, the next time a vector read on this thread has the
/// index's answer, before it takes the storage guards to judge it.
#[cfg(test)]
pub(super) fn on_index_answered(hook: impl FnOnce() + 'static) {
    ON_ANSWER.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

/// The index answered and no guard is held: a test's hook runs here.
fn index_answered() {
    #[cfg(test)]
    if let Some(hook) = ON_ANSWER.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
thread_local! {
    /// How many exact scans this thread's vector reads fell back to.
    static EXACT_SCANS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many exact scans this thread's vector reads have fallen back to.
#[cfg(test)]
pub(super) fn exact_scans() -> u32 {
    EXACT_SCANS.with(std::cell::Cell::get)
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
