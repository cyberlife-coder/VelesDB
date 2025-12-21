//! BM25 full-text search index for hybrid search.
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

use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHashSet};

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
#[allow(clippy::cast_precision_loss)] // BM25 scoring uses f32 approximations
pub struct Bm25Index {
    /// BM25 parameters
    params: Bm25Params,
    /// Inverted index: term -> set of document IDs (`FxHashSet` for faster lookup)
    inverted_index: RwLock<FxHashMap<String, FxHashSet<u64>>>,
    /// Document storage: id -> Document
    documents: RwLock<FxHashMap<u64, Document>>,
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
            doc_count: RwLock::new(0),
            total_doc_length: RwLock::new(0),
        }
    }

    /// Tokenizes text into lowercase terms.
    ///
    /// Simple whitespace + punctuation tokenizer.
    fn tokenize(text: &str) -> Vec<String> {
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
    /// * `id` - Unique document identifier
    /// * `text` - Document text to index
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

        #[allow(clippy::cast_possible_truncation)] // Document length won't exceed u32::MAX
        let doc_length = tokens.len() as u32;

        // Create document (move term_freqs, avoid clone)
        let doc = Document {
            term_freqs,
            length: doc_length,
        };

        // Update inverted index (use doc.term_freqs since we moved it)
        {
            let mut inv_idx = self.inverted_index.write();
            for term in doc.term_freqs.keys() {
                inv_idx.entry(term.clone()).or_default().insert(id);
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
    /// # Returns
    ///
    /// `true` if the document was found and removed.
    pub fn remove_document(&self, id: u64) -> bool {
        let doc = {
            let mut docs = self.documents.write();
            docs.remove(&id)
        };

        if let Some(doc) = doc {
            // Remove from inverted index
            {
                let mut inv_idx = self.inverted_index.write();
                for term in doc.term_freqs.keys() {
                    if let Some(doc_set) = inv_idx.get_mut(term) {
                        doc_set.remove(&id);
                        if doc_set.is_empty() {
                            inv_idx.remove(term);
                        }
                    }
                }
            }

            // Update counts
            {
                let mut count = self.doc_count.write();
                *count = count.saturating_sub(1);
            }
            {
                let mut total = self.total_doc_length.write();
                *total = total.saturating_sub(u64::from(doc.length));
            }

            true
        } else {
            false
        }
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
            let n = doc_count as f32;

            // Build IDF cache
            let idf_cache: FxHashMap<&str, f32> = query_terms
                .iter()
                .map(|term| {
                    let df = inv_idx.get(term).map_or(0, FxHashSet::len);
                    let idf_val = if df == 0 {
                        0.0
                    } else {
                        let df_f = df as f32;
                        ((n - df_f + 0.5) / (df_f + 0.5) + 1.0).ln()
                    };
                    (term.as_str(), idf_val)
                })
                .collect();

            // Collect and score candidates in one pass
            let candidates: FxHashSet<u64> = query_terms
                .iter()
                .filter_map(|term| inv_idx.get(term))
                .flat_map(|s| s.iter().copied())
                .collect();

            candidates
                .into_iter()
                .filter_map(|doc_id| {
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
            scores.select_nth_unstable_by(k, |a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            scores.truncate(k);
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
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
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests (TDD)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic functionality tests
    // =========================================================================

    #[test]
    fn test_bm25_index_creation() {
        let index = Bm25Index::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.term_count(), 0);
    }

    #[test]
    fn test_bm25_index_with_custom_params() {
        let params = Bm25Params { k1: 1.5, b: 0.5 };
        let index = Bm25Index::with_params(params);
        assert!((index.params.k1 - 1.5).abs() < f32::EPSILON);
        assert!((index.params.b - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_add_single_document() {
        let index = Bm25Index::new();
        index.add_document(1, "hello world");

        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());
        assert!(index.term_count() >= 2); // "hello" and "world"
    }

    #[test]
    fn test_add_multiple_documents() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming language");
        index.add_document(2, "python programming language");
        index.add_document(3, "java programming");

        assert_eq!(index.len(), 3);
    }

    #[test]
    fn test_remove_document() {
        let index = Bm25Index::new();
        index.add_document(1, "hello world");
        index.add_document(2, "goodbye world");

        assert_eq!(index.len(), 2);

        let removed = index.remove_document(1);
        assert!(removed);
        assert_eq!(index.len(), 1);

        // Removing again should return false
        let removed_again = index.remove_document(1);
        assert!(!removed_again);
    }

    #[test]
    fn test_update_document() {
        let index = Bm25Index::new();
        index.add_document(1, "original text");
        index.add_document(1, "updated text"); // Same ID

        assert_eq!(index.len(), 1); // Still one document
    }

    // =========================================================================
    // Tokenization tests
    // =========================================================================

    #[test]
    fn test_tokenize_basic() {
        let tokens = Bm25Index::tokenize("Hello World");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_punctuation() {
        let tokens = Bm25Index::tokenize("Hello, World! How are you?");
        assert_eq!(tokens, vec!["hello", "world", "how", "are", "you"]);
    }

    #[test]
    fn test_tokenize_single_chars_filtered() {
        let tokens = Bm25Index::tokenize("I am a test");
        // Single characters should be filtered out
        assert!(!tokens.contains(&"i".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
        assert!(tokens.contains(&"am".to_string()));
        assert!(tokens.contains(&"test".to_string()));
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = Bm25Index::tokenize("");
        assert!(tokens.is_empty());
    }

    // =========================================================================
    // Search tests
    // =========================================================================

    #[test]
    fn test_search_single_term() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming language");
        index.add_document(2, "python programming language");
        index.add_document(3, "rust is fast");

        let results = index.search("rust", 10);

        // Documents 1 and 3 should match
        assert_eq!(results.len(), 2);
        let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
    }

    #[test]
    fn test_search_multiple_terms() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming language fast");
        index.add_document(2, "python programming language");
        index.add_document(3, "rust systems programming");

        let results = index.search("rust programming", 10);

        // All docs match "programming", docs 1 and 3 also match "rust"
        assert!(!results.is_empty());

        // Doc 1 should score highest (matches both "rust" and "programming")
        // Actually doc 3 also matches both, let's check they're both high
        let ids: Vec<u64> = results.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
    }

    #[test]
    fn test_search_no_match() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming");
        index.add_document(2, "python programming");

        let results = index.search("javascript", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_query() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming");

        let results = index.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_index() {
        let index = Bm25Index::new();
        let results = index.search("rust", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_limit_k() {
        let index = Bm25Index::new();
        for i in 1..=100 {
            index.add_document(i, &format!("document number {i} about rust"));
        }

        let results = index.search("rust", 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_search_scores_sorted_descending() {
        let index = Bm25Index::new();
        index.add_document(1, "rust");
        index.add_document(2, "rust rust"); // Higher TF
        index.add_document(3, "rust rust rust");

        let results = index.search("rust", 10);

        // Scores should be sorted descending
        for window in results.windows(2) {
            assert!(window[0].1 >= window[1].1);
        }
    }

    // =========================================================================
    // BM25 scoring tests
    // =========================================================================

    #[test]
    fn test_idf_common_term() {
        let index = Bm25Index::new();
        // "programming" appears in all documents
        index.add_document(1, "rust programming");
        index.add_document(2, "python programming");
        index.add_document(3, "java programming");

        // "rust" appears in 1 document
        let results = index.search("rust", 10);
        assert_eq!(results.len(), 1);

        // "programming" appears in all - should have lower IDF but still return results
        let results = index.search("programming", 10);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_longer_documents_normalized() {
        let index = Bm25Index::new();
        // Short document with "rust"
        index.add_document(1, "rust");
        // Long document with "rust" once among many other words
        index.add_document(
            2,
            "rust is a systems programming language that runs blazingly fast",
        );

        let results = index.search("rust", 10);

        // Both should match
        assert_eq!(results.len(), 2);
        // The short document should score higher (more concentrated term)
        assert_eq!(results[0].0, 1);
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_special_characters() {
        let index = Bm25Index::new();
        index.add_document(1, "hello@world.com is an email");

        let results = index.search("hello", 10);
        assert_eq!(results.len(), 1);

        let results = index.search("world", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_numbers_in_text() {
        let index = Bm25Index::new();
        index.add_document(1, "version 2.0 released in 2024");

        let results = index.search("2024", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_unicode_text() {
        let index = Bm25Index::new();
        index.add_document(1, "café résumé naïve");

        let results = index.search("café", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_duplicate_terms_in_query() {
        let index = Bm25Index::new();
        index.add_document(1, "rust programming");

        // Query with duplicate terms
        let results = index.search("rust rust rust", 10);
        assert_eq!(results.len(), 1);
    }

    // =========================================================================
    // Thread safety tests
    // =========================================================================

    #[test]
    fn test_concurrent_reads() {
        use std::sync::Arc;
        use std::thread;

        let index = Arc::new(Bm25Index::new());

        // Add documents
        for i in 1..=100 {
            index.add_document(i, &format!("document {i} about rust programming"));
        }

        // Spawn multiple reader threads
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let idx = Arc::clone(&index);
                thread::spawn(move || {
                    for _ in 0..100 {
                        let results = idx.search("rust", 10);
                        assert!(!results.is_empty());
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}
