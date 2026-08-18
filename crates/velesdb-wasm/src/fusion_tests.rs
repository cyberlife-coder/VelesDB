use super::*;

#[test]
fn test_fuse_rrf_basic() {
    let results = vec![
        vec![(1, 0.9), (2, 0.8), (3, 0.7)],
        vec![(2, 1.0), (1, 0.5), (4, 0.3)],
    ];

    let fused = fuse_results(&results, "rrf", 60, None).unwrap();

    // ID 1 and 2 should be at top (appear in both lists)
    assert!(fused.len() >= 2);
    let top_ids: Vec<u64> = fused.iter().take(2).map(|(id, _)| *id).collect();
    assert!(top_ids.contains(&1) || top_ids.contains(&2));
}

#[test]
fn test_fuse_average() {
    let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

    let fused = fuse_results(&results, "average", 60, None).unwrap();

    // Both should have average 0.7
    for (_, score) in &fused {
        assert!((score - 0.7).abs() < 0.01);
    }
}

#[test]
fn test_fuse_maximum() {
    let results = vec![vec![(1, 0.9), (2, 0.5)], vec![(1, 0.3), (2, 0.8)]];

    let fused = fuse_results(&results, "maximum", 60, None).unwrap();

    let id1_score = fused.iter().find(|(id, _)| *id == 1).map(|(_, s)| *s);
    let id2_score = fused.iter().find(|(id, _)| *id == 2).map(|(_, s)| *s);

    assert!((id1_score.unwrap() - 0.9).abs() < 0.01);
    assert!((id2_score.unwrap() - 0.8).abs() < 0.01);
}

#[test]
fn test_fuse_empty() {
    let results: Vec<Vec<(u64, f32)>> = vec![];
    let fused = fuse_results(&results, "rrf", 60, None).unwrap();
    assert!(fused.is_empty());
}

#[test]
fn test_fuse_single_query() {
    let results = vec![vec![(1, 0.9), (2, 0.8)]];
    let fused = fuse_results(&results, "rrf", 60, None).unwrap();

    assert_eq!(fused.len(), 2);
    assert_eq!(fused[0].0, 1); // Higher RRF score (rank 0)
}

/// Default `weighted` fusion (no explicit weights) must use core's
/// canonical constants (`avg=0.6, max=0.3, hit=0.1`) — see issue #1545.
#[test]
fn test_fuse_weighted_uses_core_canonical_defaults() {
    let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

    let fused = fuse_results(&results, "weighted", 60, None).unwrap();

    // Both docs appear in 2/2 queries => hit_ratio = 1.0
    // ID 1: avg=0.7, max=0.8 => 0.6*0.7 + 0.3*0.8 + 0.1*1.0 = 0.76
    // ID 2: avg=0.7, max=0.8 => same
    assert_eq!(fused.len(), 2);
    for (_, score) in &fused {
        assert!((score - 0.76).abs() < 0.01, "got {score}, expected ~0.76");
    }
}

/// The `None` default must be byte-for-byte equivalent to explicitly
/// passing core's canonical `DEFAULT_WEIGHTED_*` constants: proves
/// `fuse_results` does not maintain its own hardcoded defaults anymore.
#[test]
fn test_fuse_weighted_default_matches_explicit_core_constants() {
    let results = vec![
        vec![(1, 0.8), (2, 0.6), (3, 0.4)],
        vec![(1, 0.6), (2, 0.8), (3, 0.1)],
    ];

    let via_default = fuse_results(&results, "weighted", 60, None).unwrap();
    let via_explicit_constants = fuse_results(
        &results,
        "weighted",
        60,
        Some((
            velesdb_core::fusion::DEFAULT_WEIGHTED_AVG_WEIGHT,
            velesdb_core::fusion::DEFAULT_WEIGHTED_MAX_WEIGHT,
            velesdb_core::fusion::DEFAULT_WEIGHTED_HIT_WEIGHT,
        )),
    )
    .unwrap();

    // Compare as id->score maps: HashMap-driven fusion internals don't
    // guarantee a stable tie-break order for equal scores, so a
    // positional Vec comparison would be flaky on ties.
    let as_map = |v: Vec<(u64, f32)>| -> HashMap<u64, f32> { v.into_iter().collect() };
    assert_eq!(as_map(via_default), as_map(via_explicit_constants));
}

/// Callers must be able to override the default `weighted` weights
/// (issue #1545: WASM previously hardcoded non-overridable weights).
#[test]
fn test_fuse_weighted_accepts_caller_supplied_weights() {
    let results = vec![vec![(1, 0.8), (2, 0.6)], vec![(1, 0.6), (2, 0.8)]];

    // avg_weight=1.0, others 0 => fused score reduces to the plain average.
    let fused = fuse_results(&results, "weighted", 60, Some((1.0, 0.0, 0.0))).unwrap();
    assert_eq!(fused.len(), 2);
    for (_, score) in &fused {
        assert!((score - 0.7).abs() < 0.01);
    }
}

/// Invalid caller-supplied weights (don't sum to 1.0) must surface as an
/// error rather than silently fusing with a nonsensical strategy.
#[test]
fn test_fuse_weighted_rejects_invalid_caller_weights() {
    let results = vec![vec![(1, 0.8), (2, 0.6)]];
    let err = fuse_results(&results, "weighted", 60, Some((0.9, 0.9, 0.9))).unwrap_err();
    assert!(
        err.contains("sum to 1.0"),
        "expected a weight-sum validation error, got: {err}"
    );
}

#[test]
fn test_fuse_relative_score() {
    let results = vec![vec![(1, 0.9), (2, 0.1)], vec![(1, 0.5), (2, 0.5)]];

    let fused = fuse_results(&results, "relative_score", 60, None).unwrap();

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
    let fused = fuse_results(&results, "rsf", 60, None).unwrap();
    // "rsf" should behave like "relative_score"
    assert_eq!(fused.len(), 2);
}

/// Pins the *intentional* divergence between this WASM N-branch
/// `relative_score` and core's two-branch
/// [`velesdb_core::FusionStrategy::RelativeScore`] for the production
/// arity (`multi_query_search` fuses one branch per query vector, so N is
/// the user-supplied query count — genuinely N, not a fixed dense+sparse 2).
///
/// Core's `RelativeScore` only consumes branches 0 and 1 (dense + sparse)
/// and discards the rest; this WASM path averages all N normalized branches.
/// They MUST keep producing a different ranking — if a future change tries
/// to "also delegate rsf to core", this test fails loudly instead of
/// silently corrupting multi-query WASM search results.
#[test]
fn test_rsf_n_branch_diverges_from_core_relative_score() {
    // 3 homogeneous dense-query branches (N > 2).
    let input: Vec<Vec<(u64, f32)>> = vec![
        vec![(1, 0.9), (2, 0.1), (3, 0.5)],
        vec![(1, 0.2), (2, 0.8)],
        vec![(3, 1.0), (1, 0.0)],
    ];

    let wasm_order: Vec<u64> = fuse_results(&input, "relative_score", 60, None)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    // WASM averages every branch: id3 wins (0.75), id2 (0.5), id1 (0.333).
    assert_eq!(
        wasm_order,
        vec![3, 2, 1],
        "WASM rsf must average all N branches"
    );

    // Core discards branch index >= 2, so id3 (only present in branch 2)
    // collapses to its dense contribution alone and sinks to the bottom.
    let core_strategy = FusionStrategy::relative_score(0.5, 0.5).unwrap();
    let core_order: Vec<u64> = core_strategy
        .fuse(input.clone())
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        core_order.last().copied(),
        Some(3),
        "core RelativeScore ranks the discarded-branch id last"
    );

    // WASM ranks id3 first, core ranks it last: the orderings genuinely
    // diverge, so forcing convergence onto core would be a silent
    // multi-query ranking regression. The split is deliberate.
    assert_ne!(
        wasm_order, core_order,
        "N-branch rsf must diverge from core's 2-branch RelativeScore"
    );
}

/// BUG regression (PR #556): when all scores in a branch are equal
/// (range ~ 0), the normalized value must be 0.5 — consistent with
/// the core engine's `min_max_normalize`.
#[test]
fn test_fuse_relative_score_equal_scores_default_half() {
    // All scores identical within each branch → range ≈ 0
    let results = vec![vec![(1, 0.7), (2, 0.7)], vec![(1, 0.3), (2, 0.3)]];

    let fused = fuse_results(&results, "relative_score", 60, None).unwrap();

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
    let err = fuse_results(&results, "typo_strategy", 60, None).unwrap_err();
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
        ("weighted", FusionStrategy::weighted_default()),
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
            let wasm = canonical(fuse_results(input, name, 60, None).unwrap());
            let core = canonical(strategy.fuse(input.clone()).unwrap());
            assert_eq!(
                wasm, core,
                "strategy '{name}' ordering diverged from core for input {input:?}"
            );
        }
    }
}

/// Single-sourcing guard (issue #1545): a *single-branch* `relative_score`
/// fusion has no other branch to average against, so its output must be
/// exactly the per-id normalization core's public
/// [`velesdb_core::fusion::min_max_normalize`] produces for that branch.
/// If `fuse_relative_score` ever stops delegating to it — e.g. a future
/// edit reintroduces a WASM-local copy of the min-max math — this test
/// fails the moment the two computations diverge, and it fails to even
/// *compile* if `min_max_normalize` is no longer exported from core.
#[test]
fn test_relative_score_normalization_delegates_to_core_min_max_normalize() {
    let branches: [&[(u64, f32)]; 3] = [
        &[(1, 0.9), (2, 0.1), (3, 0.5)],
        &[(1, 0.7), (2, 0.7)], // range < epsilon -> defaults to 0.5
        &[(1, 42.0)],          // single element -> range == 0 -> 0.5
    ];

    for branch in branches {
        let expected = velesdb_core::fusion::min_max_normalize(branch);

        let wasm_single_branch =
            fuse_results(&[branch.to_vec()], "relative_score", 60, None).unwrap();

        assert_eq!(
            wasm_single_branch.len(),
            expected.len(),
            "branch {branch:?}: id count mismatch between WASM rsf and core min_max_normalize"
        );
        for (id, score) in &wasm_single_branch {
            let core_score = expected[id];
            assert!(
                (score - core_score).abs() < 1e-6,
                "branch {branch:?}, id {id}: WASM normalized {score} but core's \
                 min_max_normalize produced {core_score} — normalization has drifted \
                 out of single-sourcing"
            );
        }
    }
}
