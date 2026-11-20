//! BM25 full-text search index for hybrid search.
#![allow(clippy::doc_markdown)]
#![allow(clippy::unwrap_or_default)]
//!
//! This module implements the BM25 (Best Matching 25) algorithm for full-text search,
//! enabling hybrid search combining vector similarity with keyword matching.
//!
//! # Algorithm
//!
//! BM25 score for a document D and query Q:
//! ```text
//! score(D, Q) = Σ IDF(qi) * (f(qi, D) * (k1 + 1)) / (f(qi, D) + k1 * (1 - b + b * |D| / avgdl))
//! ```
//!
//! Where:
//! - `f(qi, D)` = term frequency of qi in D
//! - `|D|` = document length
//! - `avgdl` = average document length
//! - `k1` = 1.2 (term frequency saturation)
//! - `b` = 0.75 (document length normalization)
//!
//! # Performance (v0.9+)
//!
//! - **Adaptive PostingList**: Uses `FxHashSet` for rare terms, `RoaringBitmap` for frequent terms
//! - **Automatic promotion**: Terms with 1000+ docs switch to compressed Roaring representation
//! - **Efficient unions**: O(min(n,m)) for Roaring vs O(n+m) for HashSet
//!
//! # Example
//!
//! ```rust,ignore
//! use velesdb_core::index::Bm25Index;
//!
//! let mut index = Bm25Index::new();
//! index.add_document(1, "rust programming language");
//! index.add_document(2, "python programming");
//!
//! let results = index.search("rust", 10);
//! // Returns [(1, score)] - document 1 matches "rust"
//! ```

use super::bm25_term_dict::{TermDict, TermId};
use super::posting_list::PostingList;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// BM25 tuning parameters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Bm25Params {
    /// Term frequency saturation parameter (default: 1.2)
    pub k1: f32,
    /// Document length normalization parameter (default: 0.75)
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }
}

/// A document stored in the BM25 index.
///
/// Terms are stored as [`TermId`]s resolved against the owning index's
/// [`TermDict`] (#2090) — the term string itself lives once in the
/// dictionary, not once per document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Document {
    /// Term frequencies in this document
    pub(crate) term_freqs: FxHashMap<TermId, u32>,
    /// Total number of terms in the document
    pub(crate) length: u32,
}

/// Serializable full-state snapshot of a [`Bm25Index`].
///
/// Captures the in-memory state in a form that round-trips through
/// postcard. The `inverted_index` is not stored explicitly — it is
/// rebuilt from `documents` + `point_to_doc` on
/// [`Bm25Index::from_snapshot`], which keeps the wire format compact
/// and avoids adding `Serialize` to the adaptive `PostingList` enum.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Bm25Snapshot {
    /// Schema version for forward-compat (bump on breaking changes).
    pub(crate) version: u32,
    pub(crate) params: Bm25Params,
    /// Interned terms indexed by [`TermId`] — the wire form of the
    /// index's [`TermDict`]. Version-1 snapshots (no dictionary,
    /// `String`-keyed documents) are migrated on load by
    /// [`super::bm25_term_dict::parse_snapshot`].
    pub(crate) term_dict: Vec<String>,
    pub(crate) documents: FxHashMap<u64, Document>,
    pub(crate) point_to_doc: FxHashMap<u64, u32>,
    pub(crate) doc_to_point: FxHashMap<u32, u64>,
    pub(crate) free_doc_ids: Vec<u32>,
    pub(crate) next_doc_id: u32,
    pub(crate) doc_count: usize,
    pub(crate) total_doc_length: u64,
}

/// Current [`Bm25Snapshot`] schema version. Bump on breaking changes.
///
/// Version 2 (#2090): terms interned in a `term_dict`, documents keyed
/// by [`TermId`]. Version 1 (`String`-keyed documents) is still readable
/// via [`super::bm25_term_dict::parse_snapshot`].
pub(crate) const BM25_SNAPSHOT_VERSION: u32 = 2;

/// BM25 full-text search index.
///
/// Thread-safe inverted index for efficient full-text search.
///
/// # Performance (v0.9+)
///
/// Uses adaptive `PostingList` that automatically switches between:
/// - `FxHashSet` for rare terms (< 1000 docs) - fast insert/lookup
/// - `RoaringBitmap` for frequent terms (≥ 1000 docs) - compressed, fast unions
#[allow(clippy::cast_precision_loss)] // BM25 scoring uses f32 approximations
pub struct Bm25Index {
    /// BM25 parameters
    params: Bm25Params,
    /// Term dictionary: term string -> [`TermId`] (#2090). Grows
    /// monotonically — removing a document never un-interns terms.
    ///
    /// Lock order (stable across all paths): `term_dict` →
    /// `inverted_index` → `documents` → `point_to_doc` → `doc_to_point`
    /// → `free_doc_ids` → `next_doc_id` → `doc_count` →
    /// `total_doc_length`. No path takes a lower-positioned lock while
    /// holding a higher-positioned one.
    ///
    /// A document enters or leaves the postings, `documents` and the corpus
    /// statistics (`doc_count`, `total_doc_length`) under one hold of the
    /// `inverted_index` and `documents` write guards ([`CorpusStats`]), so a
    /// score — every read under the `inverted_index` read guard — weighs the
    /// postings by the corpus that holds them. A point's internal doc id is
    /// allocated and released under that same write guard, so an add and a
    /// removal of one point never interleave.
    term_dict: RwLock<TermDict>,
    /// Inverted index: term id -> adaptive posting list (auto-promotes to Roaring)
    inverted_index: RwLock<FxHashMap<TermId, PostingList>>,
    /// Document storage: id -> Document
    documents: RwLock<FxHashMap<u64, Document>>,
    /// Point ID -> internal BM25 doc ID mapping.
    point_to_doc: RwLock<FxHashMap<u64, u32>>,
    /// Internal BM25 doc ID -> point ID mapping.
    doc_to_point: RwLock<FxHashMap<u32, u64>>,
    /// Recycled internal doc IDs.
    free_doc_ids: RwLock<Vec<u32>>,
    /// Next internal doc ID to allocate.
    next_doc_id: RwLock<u32>,
    /// Total number of documents
    doc_count: RwLock<usize>,
    /// Sum of all document lengths (for avgdl calculation)
    total_doc_length: RwLock<u64>,
}

impl Bm25Index {
    /// Creates a new BM25 index with default parameters.
    #[must_use]
    pub fn new() -> Self {
        Self::with_params(Bm25Params::default())
    }

    /// Creates a new BM25 index with custom parameters.
    #[must_use]
    pub fn with_params(params: Bm25Params) -> Self {
        Self {
            params,
            term_dict: RwLock::new(TermDict::default()),
            inverted_index: RwLock::new(FxHashMap::default()),
            documents: RwLock::new(FxHashMap::default()),
            point_to_doc: RwLock::new(FxHashMap::default()),
            doc_to_point: RwLock::new(FxHashMap::default()),
            free_doc_ids: RwLock::new(Vec::new()),
            next_doc_id: RwLock::new(0),
            doc_count: RwLock::new(0),
            total_doc_length: RwLock::new(0),
        }
    }

    /// Tokenizes text into lowercase terms.
    ///
    /// Simple whitespace + punctuation tokenizer.
    pub(crate) fn tokenize(text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty() && s.len() > 1) // Skip single chars
            .map(String::from)
            .collect()
    }

    /// Adds a document to the index.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique point identifier
    /// * `text` - Document text to index
    ///
    /// Text with no indexable token (`"N/A"`, `"-"`, `"7"`) replaces the
    /// previous version with nothing: the point leaves the index, as on
    /// [`Self::remove_document`]. Returning without removing it kept the old
    /// terms searchable under a payload that no longer held them.
    pub fn add_document(&self, id: u64, text: &str) {
        let tokens = Self::tokenize(text);
        if tokens.is_empty() {
            self.remove_document(id);
            return;
        }

        let term_freqs = self.intern_term_freqs(&tokens);

        // Reason: Document token count is bounded by practical text length limits.
        // Even a 1GB document with single-char tokens would have ~1B tokens, fitting in u32.
        #[allow(clippy::cast_possible_truncation)]
        let doc_length = tokens.len() as u32;

        let doc = Document {
            term_freqs,
            length: doc_length,
        };

        // The previous version leaves and this one enters under one hold of
        // the postings, with the statistics a score weighs them by
        // ([`Self::score_candidates`]) and the internal doc id it is posted
        // under: a removal of the point, which takes the same guard first,
        // cannot release that id in between.
        let mut postings = self.inverted_index.write();
        let mut docs = self.documents.write();
        // A point already indexed keeps its id, so an update is stable.
        let Some(id_u32) = self.get_or_allocate_doc_id(id) else {
            return;
        };
        let old = docs.insert(id, doc);
        CorpusStats::write(self).replace(old.as_ref().map(|old| old.length), Some(doc_length));
        if let Some(old) = &old {
            unpost(&mut postings, old, id_u32);
        }
        if let Some(new) = docs.get(&id) {
            post(&mut postings, new, id_u32);
        }
    }

    /// Counts term frequencies, interning each token into the dictionary.
    ///
    /// A token already in the corpus vocabulary (the common case once the
    /// index has ingested more than a handful of documents) costs one hash
    /// lookup and zero allocations; the term string is copied only on its
    /// first appearance in the whole index. The dictionary write lock is
    /// scoped to this function and released before any other index lock is
    /// taken (lock order: `term_dict` first).
    fn intern_term_freqs(&self, tokens: &[String]) -> FxHashMap<TermId, u32> {
        let mut term_freqs: FxHashMap<TermId, u32> = FxHashMap::default();
        let mut dict = self.term_dict.write();
        for token in tokens {
            // Dictionary full (`u32::MAX` distinct terms): skip the term,
            // mirroring the doc-id overflow contract of `add_document`.
            let Some(term_id) = dict.intern(token) else {
                continue;
            };
            *term_freqs.entry(term_id).or_insert(0) += 1;
        }
        drop(dict);
        term_freqs
    }

    /// Removes a document from the index.
    ///
    /// # Arguments
    ///
    /// * `id` - Point identifier
    ///
    /// # Returns
    ///
    /// `true` if the document was found and removed.
    ///
    pub fn remove_document(&self, id: u64) -> bool {
        // The document leaves the postings, the statistics and its id mapping
        // under one hold of the postings, as `add_document` puts it in: two
        // removals of one point are serialized, and only the one that takes
        // the mapping out recycles the id — once. Each used to push the id it
        // had read before the other released it, so the free list handed it
        // to two later points and left one unsearchable.
        let mut postings = self.inverted_index.write();
        let removed = self.documents.write().remove(&id);
        let released = self.release_doc_id(id);
        if let (Some(doc), Some(doc_id)) = (&removed, released) {
            CorpusStats::write(self).replace(Some(doc.length), None);
            unpost(&mut postings, doc, doc_id);
        }
        removed.is_some()
    }

    /// Takes `point_id`'s internal doc id out of both mappings and onto the
    /// free list; the id, or `None` when the point has none.
    fn release_doc_id(&self, point_id: u64) -> Option<u32> {
        let released = self.point_to_doc.write().remove(&point_id)?;
        self.doc_to_point.write().remove(&released);
        self.free_doc_ids.write().push(released);
        Some(released)
    }

    /// Searches the index for documents matching the query.
    ///
    /// # Arguments
    ///
    /// * `query` - Search query text
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (`document_id`, score) tuples, sorted by score descending,
    /// a tie by ascending id.
    pub fn search(&self, query: &str, k: usize) -> Vec<(u64, f32)> {
        let mut scores = self.score_all(query);
        Self::top_k_sort(&mut scores, k);
        scores
    }

    /// Every document matching `query` with its score, in no particular
    /// order: what [`Self::search`] cuts to its top `k`, for a caller that
    /// decides which documents may take a place before the cut — a
    /// collection admitting only its rows (`collection/search/ranked.rs`).
    pub(crate) fn score_all(&self, query: &str) -> Vec<(u64, f32)> {
        let query_terms = Self::tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        // Resolve query terms to ids once per query. A term absent from the
        // dictionary appears in no document: its BM25 contribution is
        // exactly +0.0 (tf == 0 for every candidate), so dropping it here
        // leaves every score bit-identical. Duplicate query terms are kept —
        // they contribute once each, as before.
        let query_ids: Vec<TermId> = {
            let dict = self.term_dict.read();
            query_terms
                .iter()
                .filter_map(|term| dict.get(term))
                .collect()
        };
        if query_ids.is_empty() {
            return Vec::new();
        }
        #[cfg(test)]
        terms_resolved();

        self.score_candidates(&query_ids)
    }

    /// Scores all candidate documents for the given query term ids, from one
    /// state of the index: the postings, the documents and the corpus
    /// statistics are all read under the postings' read guard, which every
    /// change to any of them holds for writing ([`CorpusStats`]).
    ///
    /// LOCK ORDER: `inverted_index` → `documents` → `doc_to_point` →
    /// `doc_count` → `total_doc_length`, all read.
    #[allow(clippy::cast_precision_loss)]
    fn score_candidates(&self, query_ids: &[TermId]) -> Vec<(u64, f32)> {
        let k1 = self.params.k1;
        let b = self.params.b;

        let inv_idx = self.inverted_index.read();
        let docs = self.documents.read();
        let doc_to_point = self.doc_to_point.read();
        let doc_count = *self.doc_count.read();
        let total_length = *self.total_doc_length.read();
        if doc_count == 0 {
            return Vec::new();
        }
        let n = doc_count as f32;
        let avgdl = total_length as f32 / n;

        let idf_cache = Self::build_idf_cache(query_ids, &inv_idx, n);
        let candidate_union = Self::build_candidate_union(query_ids, &inv_idx);

        let scored = candidate_union
            .iter()
            .filter_map(|doc_id_u32| {
                let doc_id = *doc_to_point.get(&doc_id_u32)?;
                let doc = docs.get(&doc_id)?;
                let score = Self::score_document_fast(doc, query_ids, &idf_cache, k1, b, avgdl);
                (score > 0.0).then_some((doc_id, score))
            })
            .collect();
        drop(doc_to_point);
        drop(docs);
        drop(inv_idx);
        scored
    }

    /// Builds an IDF cache for each query term id.
    #[allow(clippy::cast_precision_loss)]
    fn build_idf_cache(
        query_ids: &[TermId],
        inv_idx: &FxHashMap<TermId, PostingList>,
        n: f32,
    ) -> FxHashMap<TermId, f32> {
        query_ids
            .iter()
            .map(|&id| {
                let df = inv_idx.get(&id).map_or(0, PostingList::len);
                let idf_val = if df == 0 {
                    0.0
                } else {
                    // Never more documents than the corpus holds: the
                    // statistics are read with the postings
                    // (`score_candidates`), so this only guards the formula,
                    // whose idf would turn negative past `n`.
                    let df_f = (df as f32).min(n);
                    ((n - df_f + 0.5) / (df_f + 0.5) + 1.0).ln()
                };
                (id, idf_val)
            })
            .collect()
    }

    /// Builds a union of posting lists for all query term ids.
    fn build_candidate_union(
        query_ids: &[TermId],
        inv_idx: &FxHashMap<TermId, PostingList>,
    ) -> PostingList {
        let mut candidate_union = PostingList::new();
        for id in query_ids {
            if let Some(posting_list) = inv_idx.get(id) {
                candidate_union = candidate_union.union(posting_list);
            }
        }
        candidate_union
    }

    /// Partial sort + truncate for top-k results: descending by score, a tie
    /// by ascending id and a NaN last ([`crate::distance::hit_rank_key`]).
    fn top_k_sort(scores: &mut Vec<(u64, f32)>, k: usize) {
        let rank = |&(id, score): &(u64, f32)| crate::distance::hit_rank_key(score, id);
        super::top_k_partial_sort(scores, k, |a, b| rank(b).cmp(&rank(a)));
    }

    /// Fast BM25 scoring with pre-computed IDF cache.
    ///
    /// Perf: Avoids lock acquisition per term by using cached IDF values.
    #[allow(clippy::cast_precision_loss)]
    fn score_document_fast(
        doc: &Document,
        query_ids: &[TermId],
        idf_cache: &FxHashMap<TermId, f32>,
        k1: f32,
        b: f32,
        avgdl: f32,
    ) -> f32 {
        let doc_len = doc.length as f32;
        let len_norm = 1.0 - b + b * doc_len / avgdl;

        query_ids
            .iter()
            .map(|id| {
                let tf = doc.term_freqs.get(id).copied().unwrap_or(0) as f32;
                if tf == 0.0 {
                    return 0.0;
                }

                let idf = idf_cache.get(id).copied().unwrap_or(0.0);

                // BM25 term score (optimized: len_norm pre-computed)
                let numerator = tf * (k1 + 1.0);
                let denominator = tf + k1 * len_norm;

                idf * numerator / denominator
            })
            .sum()
    }

    /// Returns the number of documents in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        *self.doc_count.read()
    }

    /// Returns `true` if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of unique terms in the index.
    #[must_use]
    pub fn term_count(&self) -> usize {
        self.inverted_index.read().len()
    }

    /// Gets existing internal doc-id or allocates a new one. Called under
    /// the postings' write guard, as every release of an id is
    /// ([`Self::release_doc_id`]).
    fn get_or_allocate_doc_id(&self, point_id: u64) -> Option<u32> {
        let mut map = self.point_to_doc.write();
        if let Some(existing) = map.get(&point_id).copied() {
            return Some(existing);
        }

        // Bound: the `if let` would hold the `free_doc_ids` WRITE guard across
        // the `else` branch, which takes a second write lock (`next_doc_id`).
        let recycled = self.free_doc_ids.write().pop();
        let allocated = if let Some(recycled) = recycled {
            recycled
        } else {
            let mut next = self.next_doc_id.write();
            let current = *next;
            *next = next.checked_add(1)?;
            current
        };

        map.insert(point_id, allocated);
        // The forward map is written; releasing it before the reverse-map lock
        // un-nests the two. Readers of `doc_to_point` never take `point_to_doc`,
        // so the pair was never atomic for them to begin with.
        drop(map);
        self.doc_to_point.write().insert(allocated, point_id);
        Some(allocated)
    }
}

/// Adds the internal doc id `doc_id` to the postings of `doc`'s terms.
/// `PostingList` auto-promotes to Roaring when its cardinality exceeds its
/// threshold; `TermId` is `Copy`, so `entry()` allocates nothing.
fn post(postings: &mut FxHashMap<TermId, PostingList>, doc: &Document, doc_id: u32) {
    for term_id in doc.term_freqs.keys() {
        postings
            .entry(*term_id)
            .or_insert_with(PostingList::new)
            .insert(doc_id);
    }
}

/// Removes the internal doc id `doc_id` from the postings of `doc`'s terms,
/// and a term's postings once empty.
fn unpost(postings: &mut FxHashMap<TermId, PostingList>, doc: &Document, doc_id: u32) {
    for term in doc.term_freqs.keys() {
        if let Some(posting_list) = postings.get_mut(term) {
            posting_list.remove(doc_id);
            if posting_list.is_empty() {
                postings.remove(term);
            }
        }
    }
}

/// The corpus statistics a score weighs a term by — how many documents the
/// index holds and their total length — held for writing. Every change to
/// them is made under the postings' write guard, with the postings and the
/// documents it describes, so a score — which takes the postings' read
/// guard before anything else — reads one state: a term is never counted in
/// more documents than the corpus holds (which made its idf negative and
/// dropped every hit).
///
/// LOCK ORDER: `doc_count` → `total_doc_length`, the last two of the
/// index's own order, taken under `inverted_index`.
struct CorpusStats<'a> {
    count: parking_lot::RwLockWriteGuard<'a, usize>,
    total_length: parking_lot::RwLockWriteGuard<'a, u64>,
}

impl<'a> CorpusStats<'a> {
    fn write(index: &'a Bm25Index) -> Self {
        let count = index.doc_count.write();
        let total_length = index.total_doc_length.write();
        Self {
            count,
            total_length,
        }
    }

    /// The corpus's document of length `old`, if any, replaced by one of
    /// length `new`, if any.
    fn replace(&mut self, old: Option<u32>, new: Option<u32>) {
        if let Some(old) = old {
            *self.count = self.count.saturating_sub(1);
            *self.total_length = self.total_length.saturating_sub(u64::from(old));
        }
        if let Some(new) = new {
            *self.count += 1;
            *self.total_length += u64::from(new);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Run once, the next time a score on this thread has resolved its
    /// query's terms ([`on_terms_resolved`]).
    static ON_TERMS_RESOLVED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `hook` once, the next time [`Bm25Index::score_all`] on this thread
/// has resolved its query's terms, before it reads the postings and the
/// statistics it scores them by: where a write landing between two of those
/// reads would tear them.
#[cfg(test)]
pub(super) fn on_terms_resolved(hook: impl FnOnce() + 'static) {
    ON_TERMS_RESOLVED.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn terms_resolved() {
    if let Some(hook) = ON_TERMS_RESOLVED.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Snapshot serialization (for `bm25_persistence` module)
// ---------------------------------------------------------------------------

impl Bm25Index {
    /// Captures the current index state as a [`Bm25Snapshot`].
    ///
    /// Takes a snapshot under read locks — callers that need a
    /// coherent picture across mutations must serialise writes
    /// externally (the intended caller is `Collection::flush_full`,
    /// which already holds the collection-level write lock when it
    /// snapshots the BM25 index).
    ///
    /// ## Wire format
    ///
    /// Only the primary state (documents, mapping tables, counters)
    /// is serialised. The `inverted_index` is rebuilt from
    /// `documents` on [`Self::from_snapshot`]. This keeps the wire
    /// format compact and avoids adding `Serialize` to the adaptive
    /// `PostingList` enum (which would fix its internal representation
    /// and block future tuning).
    #[must_use]
    pub(crate) fn to_snapshot(&self) -> Bm25Snapshot {
        // Lock acquisition order (stable across to_snapshot / load paths):
        //   term_dict → documents → point_to_doc → doc_to_point
        //   → free_doc_ids → next_doc_id → doc_count → total_doc_length.
        // Holding multiple read locks simultaneously is safe: parking_lot
        // RwLock reads never block each other, and no code path takes a
        // write lock *after* a read lock on a lower-positioned field.
        let term_dict = self.term_dict.read();
        let documents = self.documents.read();
        let point_to_doc = self.point_to_doc.read();
        let doc_to_point = self.doc_to_point.read();
        let free_doc_ids = self.free_doc_ids.read();
        let next_doc_id = *self.next_doc_id.read();
        let doc_count = *self.doc_count.read();
        let total_doc_length = *self.total_doc_length.read();
        Bm25Snapshot {
            version: BM25_SNAPSHOT_VERSION,
            params: self.params,
            term_dict: term_dict.to_strings(),
            documents: documents.clone(),
            point_to_doc: point_to_doc.clone(),
            doc_to_point: doc_to_point.clone(),
            free_doc_ids: free_doc_ids.clone(),
            next_doc_id,
            doc_count,
            total_doc_length,
        }
    }

    /// Rebuilds an index from a [`Bm25Snapshot`].
    ///
    /// The inverted index is reconstructed from `snapshot.documents`
    /// and `snapshot.point_to_doc` — iterating each document's
    /// `term_freqs` keys and inserting the internal doc-id into the
    /// corresponding posting list. This mirrors exactly what
    /// `add_document` does for the inverted-index update, so the
    /// rebuilt postings are bitwise-identical to the original for
    /// any fixed insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`Error::IndexCorrupted`] when `snapshot.version` does not match
    /// [`BM25_SNAPSHOT_VERSION`], protecting against loading an incompatible
    /// on-disk format. Document-length counters (`doc_count`,
    /// `total_doc_length`) are recomputed from the authoritative `documents`
    /// map and rejected if the recomputed `doc_count` would leave
    /// `total_doc_length == 0` while `doc_count > 0` (which would make `avgdl`
    /// produce inf/NaN BM25 scores).
    ///
    /// Silently skips documents missing from `point_to_doc`. Such a
    /// mismatch would indicate a corrupt snapshot and is logged
    /// rather than raised, matching the existing BM25
    /// `remove_document` tolerance for unknown ids.
    ///
    /// [`Error::IndexCorrupted`]: crate::Error::IndexCorrupted
    pub(crate) fn from_snapshot(mut snapshot: Bm25Snapshot) -> crate::Result<Self> {
        Self::validate_snapshot(&mut snapshot)?;
        let index = Self::with_params(snapshot.params);

        // Rebuild inverted index from `documents` + `point_to_doc`.
        // We use a single write-lock scope on `inverted_index` for
        // efficiency — there is no other reader during construction.
        {
            let mut inv_idx = index.inverted_index.write();
            for (point_id, doc) in &snapshot.documents {
                let Some(&doc_id_u32) = snapshot.point_to_doc.get(point_id) else {
                    tracing::warn!(
                        "BM25 snapshot: document for point_id {point_id} missing from point_to_doc — skipping inverted-index reconstruction for this doc"
                    );
                    continue;
                };
                for term_id in doc.term_freqs.keys() {
                    inv_idx
                        .entry(*term_id)
                        .or_insert_with(PostingList::new)
                        .insert(doc_id_u32);
                }
            }
        }

        // Install primary state under the same lock order as to_snapshot.
        *index.term_dict.write() = TermDict::from_strings(std::mem::take(&mut snapshot.term_dict));
        *index.documents.write() = snapshot.documents;
        *index.point_to_doc.write() = snapshot.point_to_doc;
        *index.doc_to_point.write() = snapshot.doc_to_point;
        *index.free_doc_ids.write() = snapshot.free_doc_ids;
        *index.next_doc_id.write() = snapshot.next_doc_id;
        *index.doc_count.write() = snapshot.doc_count;
        *index.total_doc_length.write() = snapshot.total_doc_length;

        Ok(index)
    }

    /// Validates an untrusted [`Bm25Snapshot`] and repairs its counters in place.
    ///
    /// Rejects version mismatches, then recomputes `doc_count` /
    /// `total_doc_length` from the **scorable** corpus — the documents that are
    /// also present in `point_to_doc` and therefore added to the inverted index
    /// by [`Self::from_snapshot`]. Counting all of `documents` (including ones
    /// skipped during reconstruction) would inflate `N`/`avgdl` and skew every
    /// IDF and length-normalization, yielding wrong (but finite) scores. This
    /// also prevents a tampered header from installing inconsistent counters
    /// that would make `avgdl == 0` (inf/NaN scores).
    fn validate_snapshot(snapshot: &mut Bm25Snapshot) -> crate::Result<()> {
        if snapshot.version != BM25_SNAPSHOT_VERSION {
            return Err(crate::Error::IndexCorrupted(format!(
                "BM25 snapshot version mismatch: got {}, expected {BM25_SNAPSHOT_VERSION}",
                snapshot.version
            )));
        }

        let scorable = |point_id| snapshot.point_to_doc.contains_key(point_id);
        let real_count = snapshot.documents.keys().filter(|id| scorable(id)).count();
        let real_total: u64 = snapshot
            .documents
            .iter()
            .filter(|(id, _)| scorable(id))
            .map(|(_, d)| u64::from(d.length))
            .sum();

        // `avgdl = total_doc_length / doc_count` must never divide by a positive
        // count with a zero numerator (→ 0 → division by avgdl yields inf/NaN).
        if real_count > 0 && real_total == 0 {
            return Err(crate::Error::IndexCorrupted(
                "BM25 snapshot: doc_count > 0 but total document length is 0".to_string(),
            ));
        }

        snapshot.doc_count = real_count;
        snapshot.total_doc_length = real_total;
        Ok(())
    }
}
