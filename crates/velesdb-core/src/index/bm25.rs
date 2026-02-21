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

/// BM25 tuning parameters.
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone)]
struct Document {
    /// Term frequencies in this document
    term_freqs: FxHashMap<String, u32>,
    /// Total number of terms in the document
    length: u32,
}

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

        // SAFETY: Document token count is bounded by practical text length limits.
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
        let id_u32 = self.get_or_allocate_doc_id(id);

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

        // Perf: Single lock acquisition for IDF cache, candidates, AND document data
        // This avoids multiple lock acquisitions and allows efficient scoring.
        let k1 = self.params.k1;
        let b = self.params.b;

        let mut scores: Vec<(u64, f32)> = {
            let inv_idx = self.inverted_index.read();
            let docs = self.documents.read();
            let doc_to_point = self.doc_to_point.read();
            let n = doc_count as f32;

            // Build IDF cache
            let idf_cache: FxHashMap<&str, f32> = query_terms
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
                .collect();

            // Collect candidates using PostingList union (efficient for Roaring)
            let mut candidate_union = PostingList::new();
            for term in &query_terms {
                if let Some(posting_list) = inv_idx.get(term) {
                    candidate_union = candidate_union.union(posting_list);
                }
            }

            candidate_union
                .iter()
                .filter_map(|doc_id_u32| {
                    let doc_id = *doc_to_point.get(&doc_id_u32)?;
                    let doc = docs.get(&doc_id)?;
                    let score =
                        Self::score_document_fast(doc, &query_terms, &idf_cache, k1, b, avgdl);
                    if score > 0.0 {
                        Some((doc_id, score))
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Perf: Use partial_sort for top-k instead of full sort
        if scores.len() > k {
            scores.select_nth_unstable_by(k, |a, b| b.1.total_cmp(&a.1));
            scores.truncate(k);
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        } else {
            scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        }

        scores
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
    fn get_or_allocate_doc_id(&self, point_id: u64) -> u32 {
        let mut map = self.point_to_doc.write();
        if let Some(existing) = map.get(&point_id).copied() {
            return existing;
        }

        let allocated = if let Some(recycled) = self.free_doc_ids.write().pop() {
            recycled
        } else {
            let mut next = self.next_doc_id.write();
            let current = *next;
            *next = next
                .checked_add(1)
                .expect("BM25 internal doc-id space exhausted (u32)");
            current
        };

        map.insert(point_id, allocated);
        self.doc_to_point.write().insert(allocated, point_id);
        allocated
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
