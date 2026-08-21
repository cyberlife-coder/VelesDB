//! Deterministic A/B harness for the resumable adaptive escalation (#2077).
//!
//! Requires `--features internal-bench` (registered with `required-features`
//! in `Cargo.toml`, run by the `quality-deep` workflow next to
//! `cost_crossover`). Uses the process-global distance-evaluation counter,
//! which is bit-for-bit reproducible for a deterministic corpus — a work
//! measure usable on shared CI runners where wall-clock is noise.
//!
//! What is measured, per hard query:
//!
//! - `E_resume`  — evals of `Adaptive { min_ef, max_ef: 2*min_ef }`, the
//!   production path: phase 1 at `min_ef`, then a **resumed** escalation.
//! - `E_phase1`  — evals of `Adaptive { min_ef, max_ef: min_ef }`: the same
//!   phase 1 with escalation structurally disabled (`escalated_ef <= min_ef`
//!   returns phase-1 results).
//! - `E_restart` — evals of `Adaptive { 2*min_ef, 2*min_ef }`: a single
//!   from-scratch pass at the escalated budget — what phase 2 cost before
//!   the resume existed.
//!
//! The pre-change escalation cost was `E_phase1 + E_restart`. The harness
//! asserts the resumed path saves a meaningful fraction of that, and that
//! its recall (against exact ground truth) stays at parity with the
//! from-scratch escalation.

use velesdb_core::internal_bench::{hnsw_distance_evals, reset_hnsw_distance_evals};
use velesdb_core::{DistanceMetric, HnswIndex, SearchQuality, VectorIndex};

const DIM: usize = 256;
const N: u64 = 3_000;
const K: usize = 10;
const NQ: u64 = 40;
const MIN_EF: usize = 32;
const POCKET: u64 = 4;

/// XORshift64-based deterministic unit vector.
fn unit_vector(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut v: Vec<f32> = (0..dim)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let x = (state as f64 / u64::MAX as f64) as f32 - 0.5;
            x
        })
        .collect();
    normalize(&mut v);
    v
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in v {
        *x /= norm;
    }
}

/// Query direction for hard query `q`.
fn query_dir(q: u64) -> Vec<f32> {
    unit_vector(DIM, 7_000_000 + q)
}

/// Builds the corpus: for each query, a tight `POCKET`-vector pocket around
/// its direction (ids `q*POCKET..q*POCKET+POCKET`), then random background.
/// Pockets smaller than `K` force the tail of every top-`K` into the far
/// background, which drives the spread heuristic over its escalation
/// threshold on every query.
fn build_index() -> HnswIndex {
    let index = HnswIndex::new(DIM, DistanceMetric::Cosine).unwrap();
    for q in 0..NQ {
        let dir = query_dir(q);
        for p in 0..POCKET {
            let mut v = dir.clone();
            let noise = unit_vector(DIM, 9_000_000 + q * POCKET + p);
            for (x, n) in v.iter_mut().zip(&noise) {
                *x += 0.05 * n;
            }
            normalize(&mut v);
            index.insert(q * POCKET + p, &v);
        }
    }
    for id in NQ * POCKET..N {
        index.insert(id, &unit_vector(DIM, id + 1));
    }
    index
}

/// Exact top-`K` ids by brute-force cosine (vectors regenerated from seeds).
fn ground_truth(query: &[f32]) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = (0..N)
        .map(|id| {
            let v = corpus_vector(id);
            let dot: f32 = query.iter().zip(&v).map(|(a, b)| a * b).sum();
            (id, dot)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(K);
    scored.into_iter().map(|(id, _)| id).collect()
}

/// Regenerates the corpus vector for `id` exactly as `build_index` inserted it.
fn corpus_vector(id: u64) -> Vec<f32> {
    if id < NQ * POCKET {
        let q = id / POCKET;
        let p = id % POCKET;
        let mut v = query_dir(q);
        let noise = unit_vector(DIM, 9_000_000 + q * POCKET + p);
        for (x, n) in v.iter_mut().zip(&noise) {
            *x += 0.05 * n;
        }
        normalize(&mut v);
        v
    } else {
        unit_vector(DIM, id + 1)
    }
}

fn hits(results: &[velesdb_core::ScoredResult], truth: &[u64]) -> usize {
    results.iter().filter(|r| truth.contains(&r.id)).count()
}

fn measured_search(
    index: &HnswIndex,
    query: &[f32],
    quality: SearchQuality,
) -> (Vec<velesdb_core::ScoredResult>, u64) {
    reset_hnsw_distance_evals();
    let results = index.search_with_quality(query, K, quality).unwrap();
    (results, hnsw_distance_evals())
}

#[test]
fn resumed_escalation_saves_evals_at_recall_parity() {
    let index = build_index();

    let resume_q = SearchQuality::Adaptive {
        min_ef: MIN_EF,
        max_ef: MIN_EF * 2,
    };
    let phase1_q = SearchQuality::Adaptive {
        min_ef: MIN_EF,
        max_ef: MIN_EF,
    };
    let restart_q = SearchQuality::Adaptive {
        min_ef: MIN_EF * 2,
        max_ef: MIN_EF * 2,
    };

    let mut sum_resume = 0u64;
    let mut sum_old = 0u64;
    let mut escalated_queries = 0u64;
    let mut hits_resume = 0usize;
    let mut hits_restart = 0usize;
    let mut hits_max = 0usize;

    for q in 0..NQ {
        let query = query_dir(q);
        let truth = ground_truth(&query);

        let (r_resume, e_resume) = measured_search(&index, &query, resume_q);
        let (_r_phase1, e_phase1) = measured_search(&index, &query, phase1_q);
        let (r_restart, e_restart) = measured_search(&index, &query, restart_q);

        // Determinism: the counter and the ids must reproduce exactly.
        let (r_resume2, e_resume2) = measured_search(&index, &query, resume_q);
        assert_eq!(
            e_resume, e_resume2,
            "eval count drifted between runs (q={q})"
        );
        let ids: Vec<u64> = r_resume.iter().map(|r| r.id).collect();
        let ids2: Vec<u64> = r_resume2.iter().map(|r| r.id).collect();
        assert_eq!(ids, ids2, "result ids drifted between runs (q={q})");

        if e_resume > e_phase1 {
            escalated_queries += 1;
        }
        sum_resume += e_resume;
        sum_old += e_phase1 + e_restart;
        hits_resume += hits(&r_resume, &truth);
        hits_restart += hits(&r_restart, &truth);
        hits_max += K;
    }

    #[allow(clippy::cast_precision_loss)]
    {
        eprintln!(
            "adaptive resume A/B over {NQ} hard queries (k={K}, min_ef={MIN_EF}):\n\
             escalated: {escalated_queries}/{NQ}\n\
             evals: resume={sum_resume}  old(phase1+restart)={sum_old}  saved={:.1}%\n\
             hits@{K}: resume={hits_resume}  restart={hits_restart}  (max {hits_max})",
            100.0 * (1.0 - sum_resume as f64 / sum_old as f64),
        );
    }

    // The corpus must actually exercise the escalation path.
    assert!(
        escalated_queries * 4 >= NQ * 3,
        "corpus failed to trigger escalation: {escalated_queries}/{NQ} queries escalated"
    );

    // Work: the resumed escalation must beat the old restart total by a
    // clear margin (expected ~1/3 saved; asserted conservatively at 10%).
    assert!(
        sum_resume * 10 <= sum_old * 9,
        "resume saved too little: resume={sum_resume} evals vs old restart total={sum_old}"
    );

    // Recall parity with the from-scratch escalation: a resumed pass cannot
    // reconsider phase-1's pruned-visited nodes, so allow a 2% absolute
    // slack of the total expected hits — measured headroom is far smaller,
    // but the bound must not flake if the corpus generator shifts.
    let slack = hits_max / 50; // 2%
    assert!(
        hits_resume + slack >= hits_restart,
        "resumed escalation lost recall beyond slack: resume {hits_resume}, restart {hits_restart} (of {hits_max})"
    );
}
