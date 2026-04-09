//! Result fusion strategies for `VelesDB` WASM.
//!
//! Provides different strategies for combining results from multiple queries:
//! - Average: Mean score across all queries
//! - Maximum: Highest score from any query  
//! - RRF: Reciprocal Rank Fusion (position-based)

use std::collections::HashMap;

/// Fuses results from multiple queries using the specified strategy.
///
/// # Arguments
///
/// * `all_results` - Results from each query as (id, score) pairs
/// * `strategy` - Fusion strategy: "average", "maximum", or "rrf"
/// * `rrf_k` - RRF k parameter (typically 60)
///
/// # Returns
///
/// Fused results sorted by combined score (descending).
/// # Errors
///
/// Returns an error if `strategy` is not one of the recognised names:
/// `"average"` / `"avg"`, `"maximum"` / `"max"`, `"weighted"`,
/// `"relative_score"` / `"rsf"`, `"rrf"`.
pub fn fuse_results(
    all_results: &[Vec<(u64, f32)>],
    strategy: &str,
    rrf_k: u32,
) -> Result<Vec<(u64, f32)>, String> {
    let mut scores: HashMap<u64, Vec<f32>> = HashMap::new();
    let mut ranks: HashMap<u64, Vec<usize>> = HashMap::new();

    for (query_idx, results) in all_results.iter().enumerate() {
        for (rank, (id, score)) in results.iter().enumerate() {
            scores.entry(*id).or_default().push(*score);
            ranks
                .entry(*id)
                .or_insert_with(|| vec![usize::MAX; all_results.len()])[query_idx] = rank;
        }
    }

    let mut fused: Vec<(u64, f32)> = match strategy.to_lowercase().as_str() {
        "average" | "avg" => fuse_average(&scores),
        "maximum" | "max" => fuse_maximum(&scores),
        "weighted" => fuse_weighted(&scores, all_results.len()),
        "relative_score" | "rsf" => fuse_relative_score(all_results),
        "rrf" => fuse_rrf(&ranks, rrf_k),
        // FIXME(PRE-SEED): New fusion strategies must be added here explicitly.
        _ => {
            return Err(format!(
                "Unknown fusion strategy '{strategy}'. \
                 Expected one of: average, avg, maximum, max, weighted, \
                 relative_score, rsf, rrf"
            ));
        }
    };

    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(fused)
}

/// Average fusion: mean score across all queries.
fn fuse_average(scores: &HashMap<u64, Vec<f32>>) -> Vec<(u64, f32)> {
    scores
        .iter()
        .map(|(id, s)| {
            let avg = s.iter().sum::<f32>() / s.len() as f32;
            (*id, avg)
        })
        .collect()
}

/// Maximum fusion: highest score from any query.
fn fuse_maximum(scores: &HashMap<u64, Vec<f32>>) -> Vec<(u64, f32)> {
    scores
        .iter()
        .map(|(id, s)| {
            let max = s.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            (*id, max)
        })
        .collect()
}

/// Weighted fusion: combines average score, max score, and hit ratio.
///
/// `score = 0.5 * avg + 0.3 * max + 0.2 * (hits / total_queries)`
fn fuse_weighted(scores: &HashMap<u64, Vec<f32>>, total_queries: usize) -> Vec<(u64, f32)> {
    scores
        .iter()
        .map(|(id, s)| {
            let avg = s.iter().sum::<f32>() / s.len() as f32;
            let max = s.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let hit_ratio = s.len() as f32 / total_queries.max(1) as f32;
            (*id, 0.5 * avg + 0.3 * max + 0.2 * hit_ratio)
        })
        .collect()
}

/// Relative Score Fusion: min-max normalizes each query independently.
///
/// Each query's scores are normalized to `[0, 1]`, then averaged per document.
/// When all scores in a branch are equal (range < epsilon), the normalized
/// value defaults to 0.5 — consistent with the core engine's
/// `min_max_normalize` in `crates/velesdb-core/src/fusion/strategy.rs`.
///
/// **Note:** This WASM implementation is a simplified approximation of the
/// core `RelativeScore` strategy. The core version is designed for exactly
/// two branches (dense + sparse) with explicit weights. This WASM version
/// averages across N branches with equal weights.
fn fuse_relative_score(all_results: &[Vec<(u64, f32)>]) -> Vec<(u64, f32)> {
    let mut normalized: HashMap<u64, Vec<f32>> = HashMap::new();
    for results in all_results {
        let (min_s, max_s) = min_max_scores(results);
        let range = max_s - min_s;
        for &(id, score) in results {
            let norm = if range > f32::EPSILON {
                (score - min_s) / range
            } else {
                0.5
            };
            normalized.entry(id).or_default().push(norm);
        }
    }
    fuse_average(&normalized)
}

/// Returns `(min, max)` scores from a result set.
fn min_max_scores(results: &[(u64, f32)]) -> (f32, f32) {
    results
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &(_, s)| {
            (min.min(s), max.max(s))
        })
}

/// Reciprocal Rank Fusion: position-based scoring.
fn fuse_rrf(ranks: &HashMap<u64, Vec<usize>>, rrf_k: u32) -> Vec<(u64, f32)> {
    ranks
        .iter()
        .map(|(id, r)| {
            let rrf_score: f32 = r
                .iter()
                .filter(|&&rank| rank != usize::MAX)
                .map(|&rank| 1.0 / (rrf_k as f32 + rank as f32 + 1.0))
                .sum();
            (*id, rrf_score)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuse_rrf_basic() {
        let results = vec![
            vec![(1, 0.9), (2, 0.8), (3, 0.7)],
            vec![(2, 1.0), (1, 0.5), (4, 0.3)],
        ];

        let fused = fuse_results(&results, "rrf", 60).unwrap();

        // ID 1 and 2 should be at top (appear in both lists)
        assert!(fused.len() >= 2);
        let top_ids: Vec<u64> = fused.iter().take(2).map(|(id, _)| *id).collect();
        assert!(top_ids.contains(&1) || top_ids.contains(&2));
    }

    #[test]
    fn test_fuse_average() {
        let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

        let fused = fuse_results(&results, "average", 60).unwrap();

        // Both should have average 0.7
        for (_, score) in &fused {
            assert!((score - 0.7).abs() < 0.01);
        }
    }

    #[test]
    fn test_fuse_maximum() {
        let results = vec![vec![(1, 0.9), (2, 0.5)], vec![(1, 0.3), (2, 0.8)]];

        let fused = fuse_results(&results, "maximum", 60).unwrap();

        let id1_score = fused.iter().find(|(id, _)| *id == 1).map(|(_, s)| *s);
        let id2_score = fused.iter().find(|(id, _)| *id == 2).map(|(_, s)| *s);

        assert!((id1_score.unwrap() - 0.9).abs() < 0.01);
        assert!((id2_score.unwrap() - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_fuse_empty() {
        let results: Vec<Vec<(u64, f32)>> = vec![];
        let fused = fuse_results(&results, "rrf", 60).unwrap();
        assert!(fused.is_empty());
    }

    #[test]
    fn test_fuse_single_query() {
        let results = vec![vec![(1, 0.9), (2, 0.8)]];
        let fused = fuse_results(&results, "rrf", 60).unwrap();

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, 1); // Higher RRF score (rank 0)
    }

    #[test]
    fn test_fuse_weighted() {
        let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

        let fused = fuse_results(&results, "weighted", 60).unwrap();

        // Both docs appear in 2/2 queries => hit_ratio = 1.0
        // ID 1: avg=0.7, max=0.8 => 0.5*0.7 + 0.3*0.8 + 0.2*1.0 = 0.79
        // ID 2: avg=0.7, max=0.8 => same
        assert_eq!(fused.len(), 2);
        for (_, score) in &fused {
            assert!((score - 0.79).abs() < 0.01);
        }
    }

    #[test]
    fn test_fuse_relative_score() {
        let results = vec![vec![(1, 0.9), (2, 0.1)], vec![(1, 0.5), (2, 0.5)]];

        let fused = fuse_results(&results, "relative_score", 60).unwrap();

        // Query 0: range=0.8, ID 1 norm=(0.9-0.1)/0.8=1.0, ID 2 norm=0.0
        // Query 1: range=0, both get 0.5 (default when range==0, matches core)
        // ID 1: avg(1.0, 0.5)=0.75;  ID 2: avg(0.0, 0.5)=0.25
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, 1);
        assert!((fused[0].1 - 0.75).abs() < 0.01);
        assert_eq!(fused[1].0, 2);
        assert!((fused[1].1 - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_fuse_rsf_alias() {
        let results = vec![vec![(1, 0.9), (2, 0.1)]];
        let fused = fuse_results(&results, "rsf", 60).unwrap();
        // "rsf" should behave like "relative_score"
        assert_eq!(fused.len(), 2);
    }

    /// BUG regression (PR #556): when all scores in a branch are equal
    /// (range ~ 0), the normalized value must be 0.5 — consistent with
    /// the core engine's `min_max_normalize`.
    #[test]
    fn test_fuse_relative_score_equal_scores_default_half() {
        // All scores identical within each branch → range ≈ 0
        let results = vec![vec![(1, 0.7), (2, 0.7)], vec![(1, 0.3), (2, 0.3)]];

        let fused = fuse_results(&results, "relative_score", 60).unwrap();

        // Both branches collapse to 0.5 per document → avg = 0.5
        assert_eq!(fused.len(), 2);
        for (_, score) in &fused {
            assert!(
                (score - 0.5).abs() < 0.01,
                "equal-score branch must normalize to 0.5, got {score}"
            );
        }
    }

    #[test]
    fn test_fuse_unknown_strategy_returns_error() {
        let results = vec![vec![(1, 0.9), (2, 0.8)]];
        let err = fuse_results(&results, "typo_strategy", 60).unwrap_err();
        assert!(
            err.contains("Unknown fusion strategy"),
            "expected descriptive error, got: {err}"
        );
    }
}
