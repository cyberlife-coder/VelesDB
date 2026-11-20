//! The hits an index ranked, admitted as rows under the storage guards.
//!
//! A text or sparse search scores its candidates in its own index, then
//! keeps the best `k` that pass its admission: they are rows of the
//! collection (`RowUniverse`) — an index may hold a document for an id that
//! is no row (what an earlier version let a vector collection store without
//! a vector), and such a document must not take a row's place among the
//! `k` — they are not TTL-expired, and, when the search is filtered, their
//! payload matches the filter. All three are read under the storage guards;
//! the scoring needs none. One admission serves a search's hits and a hybrid
//! branch's rows, so an expired row takes no place in either.
//!
//! So the steps hold different locks. The index scores every candidate
//! holding only its own locks; a filtered search sets aside the candidates
//! its filter's prefilter bitmap rules out — the secondary indexes' bitmap a
//! filtered vector search restricts the index by, read under their own lock
//! ([`Collection::ranked_within_prefilter`]) — and [`Ranked::new`] arranges
//! the rest into a binary heap in linear time, still without a storage
//! guard. Only then does the walk take the storage read guards, and it walks
//! the candidates best first, a tie by ascending id ([`Ranked`] is that
//! walk, an iterator),
//! admitting each until it has `k` or the candidates run out — the filtered
//! top `k` is exact, however far below the best a matching row ranks.
//!
//! The walk offers at most [`WALK_BATCH`] fresh candidates under one hold of
//! the guards, then releases them and takes them again: a writer waits for
//! one batch, never for the whole walk, however few candidates the admission
//! takes. What it returns is the exact top `k` of the candidates the index
//! scored, admitted by one state of the stores — the one the hold that
//! finishes the walk reads. Every store and delete records the ids it wrote
//! in its store's change log, under the store's write guard
//! (`storage::ChangeLog`), and each hold reads which ids the writes since
//! the last hold changed: a candidate refused for one of them, or set aside
//! by the prefilter bitmap, which described the stores before them, is
//! offered again in rank order — every other refusal still holds — and the
//! candidates an earlier hold kept are judged again under the hold that
//! finishes, so a row a write or time changed since is dropped — and
//! replaced under that same hold. The first hold reads the writes since the
//! revision read before the prefilter bitmap was built, which describes the
//! stores at that revision or later (every write updates the secondary
//! indexes under its payload write guard), so a write landing between the
//! bitmap and the first hold is offered again too. A hold's work is its
//! batch plus the candidates written since the last one and those they
//! displace, whatever the writes. Only when more ids were written between
//! two holds than the logs keep can the walk not tell what changed: it
//! starts over from the best candidate. A restart, and a hold whose finish
//! a drop made fail, set the walk back; after a bounded number of setbacks
//! ([`RESTARTS`]) the next pass offers every candidate it needs under one
//! hold, so neither floods of writes nor a writer that keeps changing the
//! rows the walk kept stop it from finishing — at the price, then, of a
//! writer waiting for that pass. The candidates
//! themselves — the index's scores — are read before the walk, from an
//! index that follows the stores (BM25 is changed once a write's guards are
//! released).
//!
//! Each candidate offered costs one heap pop, `O(log n)`, and one admission:
//! a row-index lookup, then a payload read and the filter for a candidate
//! that is a row; a refused one is kept in a map, so a write can bring it
//! back. Each id written between two holds costs a map lookup, and a heap
//! push when it names a refused or set-aside candidate. Under the hold that
//! finishes, each kept row an earlier hold admitted is judged once more,
//! and each row returned is hydrated — its vector and payload read —
//! without being judged again. So a walk costs `O(k log n)` pops and `k`
//! payload reads when the first `k` candidates are admitted, and
//! `O(n log n)` pops and `n` payload reads at worst, when admission refuses
//! almost all of them, plus the writes it sees; a restart puts the
//! candidates back in linear time and multiplies that by at most
//! [`RESTARTS`] + 1.
//!
//! BM25 and sparse search share the walk and its admission:
//! [`Collection::ranked_hits`] hydrates the admitted rows (`text_search`,
//! `text_search_with_filter`, a sparse-only search), and
//! [`Collection::ranked_rows`] hands back their ids and scores (a hybrid
//! search's BM25 or sparse branch, which fusion ranks again).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use rustc_hash::FxHashMap;

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::search::resolve;
use crate::collection::types::Collection;
use crate::distance::hit_rank_key as rank_key;
use crate::point::SearchResult;
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage};

/// One scored candidate, ordered by its rank key — the crate's one hit
/// order ([`crate::distance::hit_rank_key`]), so the heap compares two
/// candidates in one comparison: the best is the greatest.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    rank: u128,
    id: u64,
    score: f32,
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Candidate {}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        #[cfg(test)]
        COMPARISONS.with(|count| count.set(count.get() + 1));
        self.rank.cmp(&other.rank)
    }
}

#[cfg(test)]
thread_local! {
    /// How many candidate comparisons this thread made: what bounds the
    /// cost of a walk ([`comparisons`]).
    static COMPARISONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many candidate comparisons this thread has made so far.
#[cfg(test)]
fn comparisons() -> u64 {
    COMPARISONS.with(std::cell::Cell::get)
}

/// Every candidate an index scored, handed out best first as `(id, score)`
/// (module doc). A walk hands back the candidates it refused
/// ([`Ranked::refuse`]), and those a prefilter set aside stay here, so a
/// write that changes one of them puts it back in the walk
/// ([`Ranked::reconsider`]) — a write since the revision of the stores the
/// prefilter described (`judged_at`), the walk's first hold included.
#[derive(Debug, Default)]
pub(crate) struct Ranked {
    heap: BinaryHeap<Candidate>,
    refused: FxHashMap<u64, Candidate>,
    set_aside: FxHashMap<u64, Candidate>,
    judged_at: Option<Revision>,
}

impl Ranked {
    /// The candidates `scored` as `(id, score)`, in any order, arranged in
    /// linear time — the caller does this before it takes a lock, so its
    /// walk under the storage guards pays only the pops.
    pub(crate) fn new(scored: Vec<(u64, f32)>) -> Self {
        Self::keeping(scored, |_| true)
    }

    /// [`Self::new`] for metric scores: best first by the metric's
    /// direction — the highest similarity, or the lowest distance when
    /// `higher_is_better` is false — a NaN last and a tie by ascending id
    /// either way.
    pub(crate) fn by_metric(scored: Vec<(u64, f32)>, higher_is_better: bool) -> Self {
        let rank = |score: f32| if higher_is_better { score } else { -score };
        Self::arranged(scored, rank, |_| true)
    }

    /// [`Self::new`] over the candidates `walks` takes; the others are set
    /// aside, handed out only once a write changes them or the walk starts
    /// over.
    fn keeping(scored: Vec<(u64, f32)>, walks: impl Fn(u64) -> bool) -> Self {
        Self::arranged(scored, |score| score, walks)
    }

    /// The candidates `scored`, ranked by `rank` of their score, those
    /// `walks` refuses set aside.
    fn arranged(
        scored: Vec<(u64, f32)>,
        rank: impl Fn(f32) -> f32,
        walks: impl Fn(u64) -> bool,
    ) -> Self {
        let mut walked: Vec<Candidate> = scored
            .into_iter()
            .map(|(id, score)| Candidate {
                rank: rank_key(rank(score), id),
                id,
                score,
            })
            .collect();
        let mut set_aside = FxHashMap::default();
        walked.retain(|candidate| {
            let walk = walks(candidate.id);
            if !walk {
                set_aside.insert(candidate.id, *candidate);
            }
            walk
        });
        Self {
            heap: BinaryHeap::from(walked),
            refused: FxHashMap::default(),
            set_aside,
            judged_at: None,
        }
    }

    /// These candidates, those set aside judged by a prefilter that
    /// described the stores at `revision` or later.
    fn judged_at(mut self, revision: Revision) -> Self {
        self.judged_at = Some(revision);
        self
    }

    /// The best candidate left, without handing it out.
    fn peek(&self) -> Option<&Candidate> {
        self.heap.peek()
    }

    /// Hands out the best candidate left.
    fn pop(&mut self) -> Option<Candidate> {
        self.heap.pop()
    }

    /// Takes back a candidate the walk handed out and no longer holds, to
    /// hand it out again in its place.
    fn put_back(&mut self, candidate: Candidate) {
        self.heap.push(candidate);
    }

    /// Keeps a candidate the walk refused, so a write that changes it can
    /// put it back ([`Self::reconsider`]).
    fn refuse(&mut self, candidate: Candidate) {
        self.refused.insert(candidate.id, candidate);
    }

    /// A write changed `id`: a candidate refused or set aside for it is
    /// handed out again, in rank order; whether one was.
    fn reconsider(&mut self, id: u64) -> bool {
        let back = self
            .refused
            .remove(&id)
            .or_else(|| self.set_aside.remove(&id));
        back.inspect(|&candidate| self.heap.push(candidate))
            .is_some()
    }

    /// Puts back every candidate refused and every one set aside, so the
    /// walk starts over from the best of them all; in linear time.
    fn restart(&mut self) {
        let back = self.refused.drain().chain(self.set_aside.drain());
        let mut back: BinaryHeap<Candidate> = back.map(|(_, candidate)| candidate).collect();
        self.heap.append(&mut back);
    }
}

impl ExactSizeIterator for Ranked {}

impl Iterator for Ranked {
    type Item = (u64, f32);

    fn next(&mut self) -> Option<Self::Item> {
        self.pop().map(|candidate| (candidate.id, candidate.score))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.heap.len(), Some(self.heap.len()))
    }
}

/// How many candidates a walk offers under one hold of the storage guards:
/// a writer waits for one batch, never for the whole walk (module doc).
pub(crate) const WALK_BATCH: usize = 1024;

/// How many times a walk is set back — it starts over, more writes having
/// landed between two of its holds than the stores' change logs keep, or a
/// hold that would have finished it did not, a write since having changed a
/// row it kept — before it offers every candidate it needs under one hold
/// (module doc).
pub(crate) const RESTARTS: u32 = 3;

/// What the stores held under one hold of their read guards: two holds that
/// read the same revisions read the same vectors and payloads
/// ([`crate::storage::ChangeLog`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Revision {
    vectors: u64,
    payloads: u64,
}

impl Revision {
    fn of(vector_storage: &MmapStorage, payload_storage: &LogPayloadStorage) -> Self {
        Self {
            vectors: vector_storage.changes().revision(),
            payloads: payload_storage.changes().revision(),
        }
    }
}

/// What a hold learns of the writes since the walk's last hold.
enum Since {
    /// The ids they changed (none when nothing was written).
    Changed(Vec<u64>),
    /// More than the change logs keep: the walk must start over.
    Lost,
}

/// Where a walk stands: the revision its last hold read — before the first
/// hold, the one its prefilter described, if any — and how many times it
/// was set back: it started over, or a hold that would have finished it did
/// not ([`Step::SetBack`]).
#[derive(Debug, Default)]
struct Pass {
    read: Option<Revision>,
    setbacks: u32,
}

impl Pass {
    /// A walk whose candidates were judged at `revision`, if any: its first
    /// hold reads what changed since.
    fn since(revision: Option<Revision>) -> Self {
        Self {
            read: revision,
            setbacks: 0,
        }
    }

    /// What changed in the stores since the last hold, which this hold, its
    /// guards held, now reads.
    fn observe(
        &mut self,
        vector_storage: &MmapStorage,
        payload_storage: &LogPayloadStorage,
    ) -> Since {
        let now = Revision::of(vector_storage, payload_storage);
        let Some(read) = self.read.replace(now) else {
            return Since::Changed(Vec::new());
        };
        let vectors = vector_storage.changes().changed_since(read.vectors);
        let payloads = payload_storage.changes().changed_since(read.payloads);
        match (vectors, payloads) {
            (Some(mut changed), Some(payloads)) => {
                changed.extend(payloads);
                Since::Changed(changed)
            }
            _ => Since::Lost,
        }
    }

    /// The walk starts over: the next hold is its first again.
    fn restart(&mut self) {
        self.read = None;
        self.setbacks += 1;
    }

    /// A hold that would have finished the walk did not ([`Step::SetBack`]).
    fn set_back(&mut self) {
        self.setbacks += 1;
    }

    /// How many fresh candidates a hold offers at most: one [`WALK_BATCH`],
    /// or every candidate it needs once the walk was set back [`RESTARTS`]
    /// times.
    fn batch(&self) -> usize {
        if self.setbacks >= RESTARTS {
            usize::MAX
        } else {
            WALK_BATCH
        }
    }
}

/// What a ranked candidate must pass to be kept: be a row of `rows`, be
/// taken by `within`, not be TTL-expired, and have its payload taken by
/// `accept` (a point without one is judged with a `null` payload, as on
/// every path). One rule for a search's hits and for a hybrid branch's
/// rows.
pub(super) struct Admission<W, A> {
    pub(super) rows: RowUniverse,
    pub(super) within: W,
    pub(super) accept: A,
}

impl<W, A> Admission<W, A>
where
    W: Fn(u64) -> bool,
    A: Fn(u64, Option<&serde_json::Value>) -> bool,
{
    /// Whether `id` passes, read under the caller's storage guards. Whether
    /// it is a row is an index lookup, made before its payload is read.
    fn admits(
        &self,
        id: u64,
        vector_storage: &MmapStorage,
        payload_storage: &LogPayloadStorage,
        now_secs: u64,
    ) -> bool {
        if !(self.within)(id) || !self.rows.contains(vector_storage, payload_storage, id) {
            return false;
        }
        let payload = payload_storage.retrieve(id).ok().flatten();
        !is_payload_expired(payload.as_ref(), now_secs) && (self.accept)(id, payload.as_ref())
    }
}

#[cfg(test)]
pub(crate) use super::ranked_hooks::on_guard_release;

/// The walk released the storage guards: a test's hook runs here.
fn guards_released() {
    #[cfg(test)]
    super::ranked_hooks::guards_released();
}

/// What one hold did for a walk ([`Walk::advance`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The walk holds its `k`, every one judged under this hold: it ends.
    Finished,
    /// The hold offered its budget before the walk settled.
    Unsettled,
    /// The walk settled, a row an earlier hold kept was dropped, and what
    /// was left of the budget did not settle it again: a finish that failed,
    /// counted toward [`RESTARTS`].
    SetBack,
}

/// A candidate a walk keeps, and the hold that admitted it.
#[derive(Debug, Clone, Copy)]
struct Kept {
    candidate: Candidate,
    hold: u32,
}

/// A walk in progress: the candidates left ([`Ranked`]), the best `k` kept
/// so far, best first, and the number of the current hold.
struct Walk {
    ranked: Ranked,
    kept: Vec<Kept>,
    k: usize,
    hold: u32,
}

impl Walk {
    fn new(ranked: Ranked, k: usize) -> Self {
        Self {
            ranked,
            kept: Vec::new(),
            k,
            hold: 0,
        }
    }

    /// Whether `candidate` could take a place among the `k`: fewer are kept,
    /// or it ranks above the worst kept — a candidate a write put back can.
    fn wants(&self, candidate: &Candidate) -> bool {
        self.kept.len() < self.k
            || self
                .kept
                .last()
                .is_some_and(|worst| candidate.rank > worst.candidate.rank)
    }

    /// Whether no candidate left could change the `k` kept.
    fn settled(&self) -> bool {
        !self.ranked.peek().is_some_and(|best| self.wants(best))
    }

    /// Offers the next `batch` candidates at most, best first, while they
    /// could take a place, keeping those `admits` takes and handing back the
    /// rest; how many it offered.
    fn offer(&mut self, batch: usize, admits: impl Fn(u64) -> bool) -> usize {
        for offered in 0..batch {
            if self.settled() {
                return offered;
            }
            let Some(candidate) = self.ranked.pop() else {
                return offered;
            };
            if admits(candidate.id) {
                self.keep(candidate);
            } else {
                self.ranked.refuse(candidate);
            }
        }
        batch
    }

    /// One hold's work: offers `budget` candidates at most until the walk is
    /// settled, then judges again what earlier holds kept
    /// ([`Self::keeps_earlier`]). A row dropped there frees its place under
    /// this same hold, which goes on offering what is left of its budget: a
    /// drop never by itself costs the walk a release of the guards.
    fn advance(&mut self, budget: usize, admits: impl Fn(u64) -> bool) -> Step {
        let mut left = budget;
        let mut dropped = false;
        loop {
            left -= self.offer(left, &admits);
            if !self.settled() {
                return if dropped {
                    Step::SetBack
                } else {
                    Step::Unsettled
                };
            }
            // A second pass finds every row kept judged under this hold.
            if self.keeps_earlier(&admits) {
                return Step::Finished;
            }
            dropped = true;
        }
    }

    /// Keeps `candidate` in rank order; the worst kept beyond `k` goes back
    /// among the candidates left, for a later drop to bring it back.
    fn keep(&mut self, candidate: Candidate) {
        let at = self
            .kept
            .partition_point(|kept| kept.candidate.rank > candidate.rank);
        let hold = self.hold;
        self.kept.insert(at, Kept { candidate, hold });
        if self.kept.len() > self.k {
            if let Some(displaced) = self.kept.pop() {
                self.ranked.put_back(displaced.candidate);
            }
        }
    }

    /// Drops the kept candidates `drops` takes, handing them back as
    /// refused; whether none was dropped.
    fn drop_kept(&mut self, drops: impl Fn(&Kept) -> bool) -> bool {
        let before = self.kept.len();
        let (dropped, kept): (Vec<Kept>, Vec<Kept>) = self.kept.drain(..).partition(drops);
        self.kept = kept;
        for kept in dropped {
            self.ranked.refuse(kept.candidate);
        }
        self.kept.len() == before
    }

    /// The writes since the last hold changed `ids`: a candidate refused or
    /// set aside for one goes back among those left, to be judged again when
    /// it is offered; how many went back. A kept one is judged again with
    /// every row an earlier hold kept, under the hold that finishes
    /// ([`Self::keeps_earlier`]).
    fn reconsider(&mut self, ids: &[u64]) -> usize {
        ids.iter().filter(|&&id| self.ranked.reconsider(id)).count()
    }

    /// Judges again the candidates an earlier hold kept, under the hold that
    /// finishes: a write since, or time — a row that expired — may have
    /// changed them. Whether every one stays; those that do are judged under
    /// this hold from now on.
    fn keeps_earlier(&mut self, admits: impl Fn(u64) -> bool) -> bool {
        let hold = self.hold;
        let stays = self.drop_kept(|kept| kept.hold < hold && !admits(kept.candidate.id));
        for kept in &mut self.kept {
            kept.hold = hold;
        }
        stays
    }

    /// Starts over: every candidate kept, refused or set aside back among
    /// those left.
    fn restart(&mut self) {
        for kept in self.kept.drain(..) {
            self.ranked.put_back(kept.candidate);
        }
        self.ranked.restart();
    }

    /// The `k` kept, best first, as `(id, score)`.
    fn into_rows(self) -> Vec<(u64, f32)> {
        self.kept
            .into_iter()
            .map(|kept| (kept.candidate.id, kept.candidate.score))
            .collect()
    }
}

impl Collection {
    /// Walks `ranked` best first, keeping the first `k` candidates
    /// `admission` takes, then hands them to `finish` under the same guards:
    /// the exact top `k` of the candidates `ranked` holds, admitted by the
    /// state of the stores the hold that finishes reads.
    ///
    /// The walk offers at most [`WALK_BATCH`] fresh candidates under one
    /// hold of the storage read guards, then releases and takes them again,
    /// so a writer never waits for more than one batch. Each hold first
    /// reads which ids the writes since the last one changed (the stores'
    /// change logs; the first hold, since the revision a prefilter
    /// described): a candidate refused or set aside for one is offered again
    /// in rank order, on top of the batch — every other refusal still holds.
    /// Once the walk has `k` and no candidate left could take a place, the
    /// candidates an earlier hold kept are judged again; a row a write or
    /// time changed since is dropped, and the same hold goes on offering
    /// ([`Walk::advance`]). When more writes landed between two holds than
    /// the logs keep, the walk cannot tell what changed: it releases the
    /// guards and starts over ([`Walk::restart`]). A restart, and a hold
    /// whose finish a drop made fail ([`Step::SetBack`]), set the walk back;
    /// after [`RESTARTS`] setbacks it offers every candidate it needs under
    /// one hold, which a writer then waits for. So the walk always ends: past
    /// the candidates writes put back and the rows they displace, which a
    /// hold's budget pays for on top of its batch, every hold that does not
    /// end it offers candidates no hold offered before — finitely many — or
    /// sets it back, a bounded number of times.
    ///
    /// LOCK ORDER: vector_storage(2) → payload_storage(3), both read, per
    /// batch.
    pub(super) fn walk_admitted<T>(
        &self,
        ranked: Ranked,
        k: usize,
        admission: &Admission<
            impl Fn(u64) -> bool,
            impl Fn(u64, Option<&serde_json::Value>) -> bool,
        >,
        finish: impl FnOnce(Vec<(u64, f32)>, &MmapStorage, &LogPayloadStorage, u64) -> T,
    ) -> T {
        let mut pass = Pass::since(ranked.judged_at);
        let mut walk = Walk::new(ranked, k);
        loop {
            let vector_storage = self.storage.vector_storage.read();
            let payload_storage = self.storage.payload_storage.read();
            let Since::Changed(changed) = pass.observe(&vector_storage, &payload_storage) else {
                drop(payload_storage);
                drop(vector_storage);
                walk.restart();
                pass.restart();
                guards_released();
                continue;
            };
            let now = now_unix_secs();
            let admits = |id| admission.admits(id, &vector_storage, &payload_storage, now);
            walk.hold += 1;
            // A candidate a write put back, and the kept one it may
            // displace, cost the hold no fresh candidate of its batch.
            let back = walk.reconsider(&changed);
            let budget = pass.batch().saturating_add(back.saturating_mul(2));
            // Judged again even when the candidates ran out: what `finish`
            // gets passed under the guards it holds.
            match walk.advance(budget, admits) {
                Step::Finished => {
                    return finish(walk.into_rows(), &vector_storage, &payload_storage, now);
                }
                Step::SetBack => pass.set_back(),
                Step::Unsettled => {}
            }
            drop(payload_storage);
            drop(vector_storage);
            guards_released();
        }
    }

    /// The first `k` candidates of `ranked` that are rows of `rows`, that
    /// `within` takes and whose payload `accept` takes — a filtered search
    /// passes its filter as `accept` — hydrated in rank order as SELECT
    /// reads them ([`resolve::hydrate_rows`], which judges none of them
    /// again): the exact top `k` of the rows passing all three, a
    /// TTL-expired row skipped ([`Self::walk_admitted`]).
    pub(crate) fn ranked_hits(
        &self,
        ranked: Ranked,
        k: usize,
        rows: RowUniverse,
        within: impl Fn(u64) -> bool,
        accept: impl Fn(u64, Option<&serde_json::Value>) -> bool,
    ) -> Vec<SearchResult> {
        let admission = Admission {
            rows,
            within,
            accept,
        };
        self.walk_admitted(ranked, k, &admission, |kept, vectors, payloads, _| {
            resolve::hydrate_rows(kept, vectors, payloads)
        })
    }

    /// The first `k` candidates of `ranked` that are rows of `rows`, that
    /// `within` takes, that are not TTL-expired and — given a `filter` —
    /// whose payload it matches (a point without one is judged with a `null`
    /// payload, as on every path), as `(id, score)` in rank order: the exact
    /// top `k` of the rows passing all of them ([`Self::walk_admitted`]).
    pub(crate) fn ranked_rows(
        &self,
        ranked: Ranked,
        k: usize,
        rows: RowUniverse,
        within: impl Fn(u64) -> bool,
        filter: Option<&crate::filter::Filter>,
    ) -> Vec<(u64, f32)> {
        let admission = Admission {
            rows,
            within,
            accept: |id, payload: Option<&serde_json::Value>| {
                filter.is_none_or(|filter| filter.matches_point(id, payload))
            },
        };
        self.walk_admitted(ranked, k, &admission, |kept, _, _, _| kept)
    }

    /// The candidates `scored` an index handed over, arranged to walk
    /// ([`Ranked::new`]), those `filter`'s prefilter bitmap rules out set
    /// aside — the bitmap the secondary indexes build for it, as a filtered
    /// vector search restricts the index by it
    /// ([`Collection::build_prefilter_bitmap`]): a superset of the rows the
    /// filter matches when it is built, among the ids a bitmap can hold (an
    /// id past `u32::MAX` stays a candidate). The filter itself still judges
    /// every candidate the walk offers.
    ///
    /// The bitmap describes the stores at the revision read just before it
    /// is built, or later: every write updates the secondary indexes under
    /// the payload write guard (`Collection::index_payload_writes`), so a
    /// write the revision counts is in them. The walk's first hold offers
    /// again every set-aside candidate a write since that revision changed
    /// ([`Ranked::judged_at`]) — a write landing between the bitmap and the
    /// first hold included — and starts over when it cannot tell.
    ///
    /// LOCK ORDER: vector_storage(2) → payload_storage(3), both read, for
    /// the revision, released; then secondary_indexes(6), alone, released
    /// before the walk takes any storage guard.
    pub(crate) fn ranked_within_prefilter(
        &self,
        scored: Vec<(u64, f32)>,
        filter: Option<&crate::filter::Filter>,
    ) -> Ranked {
        let Some(filter) = filter else {
            return Ranked::new(scored);
        };
        let revision = self.storage_revision();
        let bitmap = self.build_prefilter_bitmap(filter);
        #[cfg(test)]
        super::ranked_hooks::prefilter_built();
        match bitmap {
            Some(bitmap) => Ranked::keeping(scored, |id| {
                u32::try_from(id).map_or(true, |id| bitmap.contains(id))
            })
            .judged_at(revision),
            None => Ranked::new(scored),
        }
    }

    /// The revision of the stores now.
    ///
    /// LOCK ORDER: vector_storage(2) → payload_storage(3), both read.
    fn storage_revision(&self) -> Revision {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        Revision::of(&vector_storage, &payload_storage)
    }
}

#[cfg(test)]
#[path = "ranked_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ranked_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "ranked_walk_tests.rs"]
mod walk_tests;
