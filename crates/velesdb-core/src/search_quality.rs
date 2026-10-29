//! The search quality profile: the recall/latency tradeoff a search asks for.
//!
//! Its own module, outside the `persistence`-gated `index`, because naming a
//! quality is not an index concern: `velesdb-wasm`, built without
//! `persistence`, reads the same `mode` strings through
//! [`crate::api_types::parse_search_mode`] instead of a copy of its grammar.
//! `index::hnsw` re-exports it at its historical path.

use serde::{Deserialize, Serialize};

/// Search quality profile controlling the recall/latency tradeoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SearchQuality {
    /// Fast search with `ef_search=96`: 97.4% recall@10 in `recall_benchmark`
    /// (10K random 128-D points), lowest latency.
    Fast,
    /// Balanced search with `ef_search=160`: 99.8% recall@10 in
    /// `recall_benchmark`, production default.
    #[default]
    Balanced,
    /// Accurate search with `ef_search=512`: 100% recall@10 in
    /// `recall_benchmark`, 0.98 on SIFT1M's 1M (`docs/BENCHMARKS.md`).
    Accurate,
    /// Exhaustive unless the index's exact-distance features are off: every
    /// stored vector is scored, so it returns the exact top-k under the index's
    /// own distance, ties aside. This variant leaves the graph —
    /// `try_search_special_quality` routes it straight to `search_brute_force`
    /// before `ef_search` is ever consulted, so [`Self::ef_search`]'s
    /// `4096.max(k * 100)` is not the number this mode runs at. The scan is
    /// O(n / cores). An index built with those features off (`new_fast_insert`,
    /// `with_params_full(.., false)`) still keeps its vectors in the graph, but
    /// there `search_brute_force` falls back to a graph search at `Accurate`'s
    /// scaled ef.
    ///
    /// **Guarded, and the guard is the reason to read this.** A collection
    /// larger than `limits.max_perfect_mode_vectors` (default 500 000) makes
    /// the search fail with [`crate::error::Error::GuardRail`] rather than run
    /// a linear scan over it. Verified in
    /// `perfect_quality_is_refused_above_the_configured_cap`.
    ///
    /// This paragraph said the opposite until #2238 — "tunes the HNSW graph's
    /// effort and is not exhaustive". Its ~0.9994 figure at 1M was measured
    /// through this very scan, on `HnswIndex` directly, which has no cap
    /// (#1225); a collection that size is refused. The paragraph was read off
    /// `ef_search()` without checking which arm runs.
    Perfect,
    /// Custom `ef_search` value.
    Custom(usize),
    /// Adaptive `ef_search` that starts low and doubles once if the query is
    /// "hard".
    ///
    /// Uses a two-phase approach:
    /// 1. Search at `max(min_ef, k)`
    /// 2. If the result spread (the first-to-last score gap over the lower
    ///    score's distance from the metric's floor, or over the smaller
    ///    absolute score on an unbounded metric) reaches 2.0, search once more
    ///    at twice its ef, capped at `max_ef`, when that exceeds the first ef
    ///
    /// Easy queries (dense cluster hits) stop after phase 1. No recorded run
    /// measures its latency or recall against a fixed ef yet (#2266).
    ///
    /// # Phase 2 is out of reach on Cosine
    ///
    /// The spread is divided by the lower score's distance from the metric's
    /// floor, and Cosine's floor is `-1`. So `spread = (first - last) /
    /// (last + 1)`, and reaching 2.0 with a perfect top hit needs
    /// `last <= -1/3`: the **k-th** result must be anti-correlated with the
    /// query. Ordinary data does not do that — in 256 dimensions the best of
    /// 3,000 random vectors sits near `+0.17`, not below `-0.33`.
    ///
    /// Measured on one 3,000 x 256 corpus built to be hard (clusters smaller
    /// than `k`, so every top-`k` tail falls into the far background),
    /// phase 1 at `min_ef = 32`, `k = 10`:
    ///
    /// | metric | first | last | spread | queries over 2.0 |
    /// | --- | --- | --- | --- | --- |
    /// | Cosine | 0.9988 | 0.16 to 0.18 | 0.70 to 0.72 | 0 / 40 |
    /// | Euclidean | 0.049 | 1.28 to 1.29 | 24.9 to 25.1 | 40 / 40 |
    ///
    /// On a Cosine collection this variant is therefore a single pass at
    /// `min_ef`, whatever `max_ef` says, and so is [`Self::AutoTune`], which
    /// delegates to the same algorithm. Whether the threshold or the Cosine
    /// baseline should change is a behaviour question, open on #2266: moving
    /// either one trades latency for recall on every Cosine query, and no run
    /// measures that trade yet.
    Adaptive {
        /// Minimum `ef_search` (starting point). Default: 32.
        min_ef: usize,
        /// Maximum `ef_search` (cap). Default: 512.
        max_ef: usize,
    },
    /// Auto-tuned adaptive search based on collection statistics.
    ///
    /// Computes `min_ef` / `max_ef` from the collection's current size and
    /// vector dimension, then delegates to the same two-phase adaptive
    /// algorithm used by [`SearchQuality::Adaptive`]. `min_ef` grows by tiers:
    ///
    /// - up to 1K vectors: `k * 2`
    /// - 1K–10K: `k * 4`
    /// - 10K–100K: `k * 8`
    /// - more than 100K: `k * 12`
    /// - above 512 dimensions: times 1.5
    ///
    /// `min_ef` is at least `k`, and `max_ef` is `4 * min_ef`. Some search paths
    /// run one pass, scan exactly or ignore the mode instead: the Search Modes
    /// guide names the main ones ("When the two phases run"), and #2268
    /// tracks the rest. No recorded run measures its latency or recall (#2266).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use velesdb_core::SearchQuality;
    /// let results = index.search_with_quality(&query, 10, SearchQuality::AutoTune);
    /// ```
    AutoTune,
}

impl SearchQuality {
    /// Returns the `ef_search` value for this quality profile.
    ///
    /// # Large-scale optimization (v0.9+)
    ///
    /// - **Accurate**: 512 base (was 256), scales with k×16 to hold recall at 100K+
    /// - **Perfect**: 4096 base, scaled with k×100 — the ef a graph traversal
    ///   would use. With its exact-distance features on, `HnswIndex` does not
    ///   traverse for `Perfect`: it scans exhaustively (#2238), so this is not
    ///   what `Perfect` costs there.
    /// - **Adaptive**: returns `min_ef` (first phase); caller handles second phase
    #[must_use]
    pub fn ef_search(&self, k: usize) -> usize {
        match self {
            Self::Fast => 96.max(k * 3),
            // AutoTune: resolved at search time via auto_ef_range(); fallback
            // to Balanced for contexts that call ef_search() without collection info.
            Self::Balanced | Self::AutoTune => 160.max(k * 5),
            // Increased from 256 to 512 for better recall at 100K+ scale
            Self::Accurate => 512.max(k * 16),
            // The traversal ef for Perfect-class recall, for a caller that walks the
            // graph; `HnswIndex` scans exhaustively instead (#2238) unless its
            // exact-distance features are off.
            Self::Perfect => 4096.max(k * 100),
            Self::Custom(ef) => (*ef).max(k),
            // Adaptive: start with min_ef (first phase)
            Self::Adaptive { min_ef, .. } => (*min_ef).max(k),
        }
    }

    /// Returns `ef_search` scaled to dataset size.
    ///
    /// For datasets > 10K, the HNSW graph is deeper and requires a slightly
    /// larger search budget. Uses sqrt-based scaling (gentler than log2) to
    /// avoid excessive latency when the graph is already well-constructed
    /// with appropriate M and `ef_construction` parameters.
    ///
    /// Capped at 2x the base ef to prevent latency explosion.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn ef_search_for_scale(&self, k: usize, dataset_size: usize) -> usize {
        let base = self.ef_search(k);
        if dataset_size <= 10_000 {
            return base;
        }
        // sqrt scaling: gentle increase that doesn't destroy latency.
        // At 100K: sqrt(10) ≈ 3.16 → capped at 2.0 → ef * 2.
        // At 1M: sqrt(100) = 10 → capped at 2.0 → ef * 2.
        // Reason: dataset_size and base are small enough that f64 is exact.
        let ratio = dataset_size as f64 / 10_000.0;
        let scale = ratio.sqrt().min(2.0);
        let scaled = (base as f64 * scale) as usize;
        scaled.min(base * 2)
    }

    /// Returns `true` if this quality profile uses two-phase adaptive search.
    #[must_use]
    pub const fn is_adaptive(&self) -> bool {
        matches!(self, Self::Adaptive { .. } | Self::AutoTune)
    }

    /// Returns the maximum ef for adaptive search, or `None` for fixed profiles.
    #[must_use]
    pub const fn adaptive_max_ef(&self) -> Option<usize> {
        match self {
            Self::Adaptive { max_ef, .. } => Some(*max_ef),
            _ => None,
        }
    }
}
