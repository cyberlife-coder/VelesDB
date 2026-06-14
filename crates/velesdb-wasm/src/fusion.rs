//! Result fusion strategies for `VelesDB` WASM.
//!
//! The four score/rank strategies (`average`, `maximum`, `weighted`, `rrf`)
//! delegate to the canonical [`velesdb_core::FusionStrategy`] so the browser
//! engine and the core engine produce identical rankings. The
//! `relative_score` / `rsf` strategy keeps a WASM-local implementation because
//! its N-branch equal-weight averaging semantics differ from core's two-branch
//! (dense + sparse) `RelativeScore` — see [`fuse_relative_score`].

use std::collections::HashMap;

use velesdb_core::FusionStrategy;

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
    match strategy.to_lowercase().as_str() {
        "average" | "avg" => fuse_with_core(all_results, &FusionStrategy::Average),
        "maximum" | "max" => fuse_with_core(all_results, &FusionStrategy::Maximum),
        "weighted" => fuse_with_core(
            all_results,
            &FusionStrategy::Weighted {
                avg_weight: 0.5,
                max_weight: 0.3,
                hit_weight: 0.2,
            },
        ),
        "rrf" => fuse_with_core(all_results, &FusionStrategy::RRF { k: rrf_k }),
        "relative_score" | "rsf" => Ok(fuse_relative_score(all_results)),
        _ => Err(format!(
            "Unknown fusion strategy '{strategy}'. \
             Expected one of: average, avg, maximum, max, weighted, \
             relative_score, rsf, rrf"
        )),
    }
}

/// Delegates to the canonical core fusion and adapts its error to a `String`.
fn fuse_with_core(
    all_results: &[Vec<(u64, f32)>],
    strategy: &FusionStrategy,
) -> Result<Vec<(u64, f32)>, String> {
    strategy
        .fuse(all_results.to_vec())
        .map_err(|e| e.to_string())
}

/// Relative Score Fusion: min-max normalizes each query independently.
///
/// Each query's scores are normalized to `[0, 1]`, then averaged per document
/// across the queries in which the document appears. When all scores in a
/// branch are equal (range < epsilon), the normalized value defaults to 0.5 —
/// consistent with the core engine's `min_max_normalize`.
///
/// **Note:** this is intentionally *not* delegated to
/// [`velesdb_core::FusionStrategy::RelativeScore`]. Core's `RelativeScore` is a
/// two-branch (dense + sparse) weighted sum that zero-fills documents missing
/// from a branch and discards branches beyond index 1. This WASM version
/// averages across N branches with equal weights and skips missing branches,
/// which yields a different ranking; converging it onto core would silently
/// change WASM search results.
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

    let mut fused: Vec<(u64, f32)> = normalized
        .iter()
        .map(|(id, s)| {
            let avg = s.iter().sum::<f32>() / s.len() as f32;
            (*id, avg)
        })
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

/// Returns `(min, max)` scores from a result set.
fn min_max_scores(results: &[(u64, f32)]) -> (f32, f32) {
    results
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &(_, s)| {
            (min.min(s), max.max(s))
        })
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

    /// Equivalence guard: for representative multi-query inputs the four
    /// delegated strategies (`average`, `maximum`, `weighted`, `rrf`) must
    /// return the SAME ordering as the corresponding
    /// [`velesdb_core::FusionStrategy::fuse`] call. `relative_score` is
    /// intentionally excluded: its N-branch averaging semantics differ from
    /// core's two-branch `RelativeScore` (documented on `fuse_relative_score`).
    #[test]
    fn test_fuse_results_matches_core_ordering() {
        let inputs: Vec<Vec<Vec<(u64, f32)>>> = vec![
            vec![
                vec![(1, 0.9), (2, 0.8), (3, 0.7)],
                vec![(2, 1.0), (1, 0.5), (4, 0.3)],
            ],
            vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]],
            vec![vec![(1, 0.9), (2, 0.5)], vec![(1, 0.3), (2, 0.8)]],
            vec![vec![(1, 0.9), (2, 0.8)]],
            vec![
                vec![(10, 0.5), (20, 0.4), (30, 0.9)],
                vec![(30, 0.1), (40, 0.99), (10, 0.2)],
                vec![(20, 0.7), (50, 0.6)],
            ],
        ];

        let cases: [(&str, FusionStrategy); 4] = [
            ("average", FusionStrategy::Average),
            ("maximum", FusionStrategy::Maximum),
            (
                "weighted",
                FusionStrategy::Weighted {
                    avg_weight: 0.5,
                    max_weight: 0.3,
                    hit_weight: 0.2,
                },
            ),
            ("rrf", FusionStrategy::RRF { k: 60 }),
        ];

        // Tie-break deterministically (score desc, then id asc) so that ties —
        // whose order depends on non-deterministic HashMap iteration — do not
        // produce spurious mismatches. Real ranking divergence still shows up
        // as a different score sequence.
        let canonical = |mut v: Vec<(u64, f32)>| -> Vec<u64> {
            v.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0))
            });
            v.into_iter().map(|(id, _)| id).collect()
        };

        for input in &inputs {
            for (name, strategy) in &cases {
                let wasm = canonical(fuse_results(input, name, 60).unwrap());
                let core = canonical(strategy.fuse(input.clone()).unwrap());
                assert_eq!(
                    wasm, core,
                    "strategy '{name}' ordering diverged from core for input {input:?}"
                );
            }
        }
    }
}
