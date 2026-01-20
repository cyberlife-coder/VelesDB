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
pub fn fuse_results(
    all_results: &[Vec<(u64, f32)>],
    strategy: &str,
    rrf_k: u32,
) -> Vec<(u64, f32)> {
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
        _ => fuse_rrf(&ranks, rrf_k),
    };

    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
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

        let fused = fuse_results(&results, "rrf", 60);

        // ID 1 and 2 should be at top (appear in both lists)
        assert!(fused.len() >= 2);
        let top_ids: Vec<u64> = fused.iter().take(2).map(|(id, _)| *id).collect();
        assert!(top_ids.contains(&1) || top_ids.contains(&2));
    }

    #[test]
    fn test_fuse_average() {
        let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

        let fused = fuse_results(&results, "average", 60);

        // Both should have average 0.7
        for (_, score) in &fused {
            assert!((score - 0.7).abs() < 0.01);
        }
    }

    #[test]
    fn test_fuse_maximum() {
        let results = vec![vec![(1, 0.9), (2, 0.5)], vec![(1, 0.3), (2, 0.8)]];

        let fused = fuse_results(&results, "maximum", 60);

        let id1_score = fused.iter().find(|(id, _)| *id == 1).map(|(_, s)| *s);
        let id2_score = fused.iter().find(|(id, _)| *id == 2).map(|(_, s)| *s);

        assert!((id1_score.unwrap() - 0.9).abs() < 0.01);
        assert!((id2_score.unwrap() - 0.8).abs() < 0.01);
    }

    #[test]
    fn test_fuse_empty() {
        let results: Vec<Vec<(u64, f32)>> = vec![];
        let fused = fuse_results(&results, "rrf", 60);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_fuse_single_query() {
        let results = vec![vec![(1, 0.9), (2, 0.8)]];
        let fused = fuse_results(&results, "rrf", 60);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].0, 1); // Higher RRF score (rank 0)
    }
}
