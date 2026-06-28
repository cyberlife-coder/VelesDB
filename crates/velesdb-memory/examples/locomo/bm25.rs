//! Minimal in-memory BM25 over the fact corpus, to complement dense recall with
//! a lexical signal. The ceiling diagnostic showed multi-hop gold facts sit in
//! the dense top-64 but below the top-8 budget — a ranking miss that a literal
//! token match (a name, a number, a rare word the embedder under-weights) can
//! fix. Fused with the dense ranking by Reciprocal Rank Fusion.

use std::collections::HashMap;

/// BM25 saturation and length-normalisation constants (the standard defaults).
const K1: f64 = 1.2;
const B: f64 = 0.75;

/// An inverted index over fact texts, scoring a query with Okapi BM25.
pub struct Bm25Index {
    /// term → list of (fact id, term frequency in that fact).
    postings: HashMap<String, Vec<(u64, u32)>>,
    /// fact id → token count (document length).
    doc_len: HashMap<u64, u32>,
    /// mean document length, for length normalisation.
    avgdl: f64,
    /// number of indexed facts.
    n_docs: f64,
}

impl Bm25Index {
    /// Build the index from `(fact id, text)` pairs (facts only — no entity hubs).
    pub fn build<'a>(docs: impl Iterator<Item = (u64, &'a str)>) -> Self {
        let mut term_docs: HashMap<String, HashMap<u64, u32>> = HashMap::new();
        let mut doc_len: HashMap<u64, u32> = HashMap::new();
        let mut total_len: u64 = 0;
        let mut n: u64 = 0;
        for (id, text) in docs {
            let tokens = tokenize(text);
            let len = u32::try_from(tokens.len()).unwrap_or(u32::MAX);
            doc_len.insert(id, len);
            total_len += u64::from(len);
            n += 1;
            for token in tokens {
                *term_docs.entry(token).or_default().entry(id).or_insert(0) += 1;
            }
        }
        let postings = term_docs
            .into_iter()
            .map(|(term, docs)| (term, docs.into_iter().collect()))
            .collect();
        let n_docs = f64::from(u32::try_from(n).unwrap_or(u32::MAX));
        // benchmark harness: corpus length within f64 exact-integer range
        #[allow(clippy::cast_precision_loss)]
        let avgdl = if n == 0 {
            0.0
        } else {
            total_len as f64 / n_docs
        };
        Self {
            postings,
            doc_len,
            avgdl,
            n_docs,
        }
    }

    /// Fact ids ranked by BM25 relevance to `query`, best first (only facts that
    /// share at least one query term appear).
    pub fn search(&self, query: &str) -> Vec<u64> {
        if self.avgdl == 0.0 {
            return Vec::new();
        }
        let mut scores: HashMap<u64, f64> = HashMap::new();
        for term in tokenize(query) {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = f64::from(u32::try_from(postings.len()).unwrap_or(u32::MAX));
            // Okapi idf with the +1 guard so it never goes negative.
            let idf = ((self.n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(id, tf) in postings {
                let tf = f64::from(tf);
                let dl = f64::from(self.doc_len.get(&id).copied().unwrap_or(0));
                let denom = tf + K1 * (1.0 - B + B * dl / self.avgdl);
                *scores.entry(id).or_insert(0.0) += idf * (tf * (K1 + 1.0)) / denom;
            }
        }
        let mut ranked: Vec<(u64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.into_iter().map(|(id, _)| id).collect()
    }
}

/// Lowercase alphanumeric tokenisation, dropping 1-char tokens (matches the
/// spirit of the entity tagging — short fragments carry no lexical signal).
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(ToString::to_string)
        .collect()
}
