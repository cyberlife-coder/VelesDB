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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Document {
    /// Term frequencies in this document
    pub(crate) term_freqs: FxHashMap<String, u32>,
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
    pub(crate) documents: FxHashMap<u64, Document>,
    pub(crate) point_to_doc: FxHashMap<u64, u32>,
    pub(crate) doc_to_point: FxHashMap<u32, u64>,
    pub(crate) free_doc_ids: Vec<u32>,
    pub(crate) next_doc_id: u32,
    pub(crate) doc_count: usize,
    pub(crate) total_doc_length: u64,
}

/// Current [`Bm25Snapshot`] schema version. Bump on breaking changes.
pub(crate) const BM25_SNAPSHOT_VERSION: u32 = 1;

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
    /// Inverted index: term -> adaptive posting list (auto-promotes to Roaring)
    inverted_index: RwLock<FxHashMap<String, PostingList>>,
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
    pub fn add_document(&self, id: u64, text: &str) {
        let tokens = Self::tokenize(text);
        if tokens.is_empty() {
            return;
        }

        // Count term frequencies
        let mut term_freqs: FxHashMap<String, u32> = FxHashMap::default();
        for token in &tokens {
            *term_freqs.entry(token.clone()).or_insert(0) += 1;
        }

        // Reason: Document token count is bounded by practical text length limits.
        // Even a 1GB document with single-char tokens would have ~1B tokens, fitting in u32.
        #[allow(clippy::cast_possible_truncation)]
        let doc_length = tokens.len() as u32;

        // Create document (move term_freqs, avoid clone)
        let doc = Document {
            term_freqs,
            length: doc_length,
        };

        // Remove previous version of this point from BM25 postings/doc stats
        // while keeping the same internal doc-id mapping for stable updates.
        self.remove_document_internal(id, false);

        // Resolve internal BM25 doc ID (u32) for RoaringBitmap-backed postings.
        let Some(id_u32) = self.get_or_allocate_doc_id(id) else {
            return;
        };

        // Update inverted index with adaptive PostingList.
        // PostingList auto-promotes to Roaring when cardinality exceeds threshold.
        {
            let mut inv_idx = self.inverted_index.write();
            for term in doc.term_freqs.keys() {
                inv_idx
                    .entry(term.clone())
                    .or_insert_with(PostingList::new)
                    .insert(id_u32);
            }
        }

        // Store document
        {
            let mut docs = self.documents.write();
            // If document exists, remove old length from total
            if let Some(old_doc) = docs.get(&id) {
                let mut total = self.total_doc_length.write();
                *total = total.saturating_sub(u64::from(old_doc.length));
            } else {
                let mut count = self.doc_count.write();
                *count += 1;
            }
            docs.insert(id, doc);
        }

        // Update total document length
        {
            let mut total = self.total_doc_length.write();
            *total += u64::from(doc_length);
        }
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
        self.remove_document_internal(id, true)
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
    /// Vector of (`document_id`, score) tuples, sorted by score descending.
    #[allow(clippy::cast_precision_loss)]
    pub fn search(&self, query: &str, k: usize) -> Vec<(u64, f32)> {
        let query_terms = Self::tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let doc_count = *self.doc_count.read();
        if doc_count == 0 {
            return Vec::new();
        }

        let total_length = *self.total_doc_length.read();
        let avgdl = total_length as f32 / doc_count as f32;

        let mut scores = self.score_candidates(&query_terms, doc_count, avgdl);
        Self::top_k_sort(&mut scores, k);
        scores
    }

    /// Scores all candidate documents for the given query terms.
    #[allow(clippy::cast_precision_loss)]
    fn score_candidates(
        &self,
        query_terms: &[String],
        doc_count: usize,
        avgdl: f32,
    ) -> Vec<(u64, f32)> {
        let k1 = self.params.k1;
        let b = self.params.b;

        let inv_idx = self.inverted_index.read();
        let docs = self.documents.read();
        let doc_to_point = self.doc_to_point.read();
        let n = doc_count as f32;

        let idf_cache = Self::build_idf_cache(query_terms, &inv_idx, n);
        let candidate_union = Self::build_candidate_union(query_terms, &inv_idx);

        candidate_union
            .iter()
            .filter_map(|doc_id_u32| {
                let doc_id = *doc_to_point.get(&doc_id_u32)?;
                let doc = docs.get(&doc_id)?;
                let score = Self::score_document_fast(doc, query_terms, &idf_cache, k1, b, avgdl);
                (score > 0.0).then_some((doc_id, score))
            })
            .collect()
    }

    /// Builds an IDF cache for each query term.
    #[allow(clippy::cast_precision_loss)]
    fn build_idf_cache<'a>(
        query_terms: &'a [String],
        inv_idx: &FxHashMap<String, PostingList>,
        n: f32,
    ) -> FxHashMap<&'a str, f32> {
        query_terms
            .iter()
            .map(|term| {
                let df = inv_idx.get(term).map_or(0, PostingList::len);
                let idf_val = if df == 0 {
                    0.0
                } else {
                    let df_f = df as f32;
                    ((n - df_f + 0.5) / (df_f + 0.5) + 1.0).ln()
                };
                (term.as_str(), idf_val)
            })
            .collect()
    }

    /// Builds a union of posting lists for all query terms.
    fn build_candidate_union(
        query_terms: &[String],
        inv_idx: &FxHashMap<String, PostingList>,
    ) -> PostingList {
        let mut candidate_union = PostingList::new();
        for term in query_terms {
            if let Some(posting_list) = inv_idx.get(term) {
                candidate_union = candidate_union.union(posting_list);
            }
        }
        candidate_union
    }

    /// Partial sort + truncate for top-k results (descending by score).
    fn top_k_sort(scores: &mut Vec<(u64, f32)>, k: usize) {
        super::top_k_partial_sort(scores, k, |a, b| b.1.total_cmp(&a.1));
    }

    /// Fast BM25 scoring with pre-computed IDF cache.
    ///
    /// Perf: Avoids lock acquisition per term by using cached IDF values.
    #[allow(clippy::cast_precision_loss)]
    fn score_document_fast(
        doc: &Document,
        query_terms: &[String],
        idf_cache: &FxHashMap<&str, f32>,
        k1: f32,
        b: f32,
        avgdl: f32,
    ) -> f32 {
        let doc_len = doc.length as f32;
        let len_norm = 1.0 - b + b * doc_len / avgdl;

        query_terms
            .iter()
            .map(|term| {
                let tf = doc.term_freqs.get(term).copied().unwrap_or(0) as f32;
                if tf == 0.0 {
                    return 0.0;
                }

                let idf = idf_cache.get(term.as_str()).copied().unwrap_or(0.0);

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

    /// Gets existing internal doc-id or allocates a new one.
    fn get_or_allocate_doc_id(&self, point_id: u64) -> Option<u32> {
        let mut map = self.point_to_doc.write();
        if let Some(existing) = map.get(&point_id).copied() {
            return Some(existing);
        }

        let allocated = if let Some(recycled) = self.free_doc_ids.write().pop() {
            recycled
        } else {
            let mut next = self.next_doc_id.write();
            let current = *next;
            *next = next.checked_add(1)?;
            current
        };

        map.insert(point_id, allocated);
        self.doc_to_point.write().insert(allocated, point_id);
        Some(allocated)
    }

    /// Removes a point from BM25 internals.
    /// If `release_mapping` is true, the internal doc-id is recycled.
    fn remove_document_internal(&self, point_id: u64, release_mapping: bool) -> bool {
        let Some(doc_id_u32) = self.point_to_doc.read().get(&point_id).copied() else {
            return false;
        };

        let doc = {
            let mut docs = self.documents.write();
            docs.remove(&point_id)
        };

        let mut removed = false;
        if let Some(doc) = doc {
            {
                let mut inv_idx = self.inverted_index.write();
                for term in doc.term_freqs.keys() {
                    if let Some(posting_list) = inv_idx.get_mut(term) {
                        posting_list.remove(doc_id_u32);
                        if posting_list.is_empty() {
                            inv_idx.remove(term);
                        }
                    }
                }
            }

            {
                let mut count = self.doc_count.write();
                *count = count.saturating_sub(1);
            }
            {
                let mut total = self.total_doc_length.write();
                *total = total.saturating_sub(u64::from(doc.length));
            }

            removed = true;
        }

        if release_mapping {
            self.point_to_doc.write().remove(&point_id);
            self.doc_to_point.write().remove(&doc_id_u32);
            self.free_doc_ids.write().push(doc_id_u32);
        }

        removed
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
        //   documents → point_to_doc → doc_to_point → free_doc_ids
        //   → next_doc_id → doc_count → total_doc_length.
        // Holding multiple read locks simultaneously is safe: parking_lot
        // RwLock reads never block each other, and no code path takes a
        // write lock *after* a read lock on a lower-positioned field.
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
    /// Silently skips documents missing from `point_to_doc`. Such a
    /// mismatch would indicate a corrupt snapshot and is logged
    /// rather than raised, matching the existing BM25
    /// `remove_document_internal` tolerance for unknown ids.
    #[must_use]
    pub(crate) fn from_snapshot(snapshot: Bm25Snapshot) -> Self {
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
                for term in doc.term_freqs.keys() {
                    inv_idx
                        .entry(term.clone())
                        .or_insert_with(PostingList::new)
                        .insert(doc_id_u32);
                }
            }
        }

        // Install primary state under the same lock order as to_snapshot.
        *index.documents.write() = snapshot.documents;
        *index.point_to_doc.write() = snapshot.point_to_doc;
        *index.doc_to_point.write() = snapshot.doc_to_point;
        *index.free_doc_ids.write() = snapshot.free_doc_ids;
        *index.next_doc_id.write() = snapshot.next_doc_id;
        *index.doc_count.write() = snapshot.doc_count;
        *index.total_doc_length.write() = snapshot.total_doc_length;

        index
    }
}
