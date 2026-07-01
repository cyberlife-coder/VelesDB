use super::*;

fn fact(id: u64, score: f64, graph_weight: f64) -> RetrievedFact {
    RetrievedFact {
        id,
        text: format!("fact {id}"),
        dia_ids: vec![format!("D1:{id}")],
        score,
        graph_weight,
        ts: 0,
    }
}

fn cfg() -> EvalCfg {
    EvalCfg {
        k: 8,
        graph_boost: 0.15,
        hops: 2,
        multihop_only: false,
        idf_weight: false,
        seed_breadth: 1,
        date_context: false,
        date_routed: false,
        temporal_scaffold: false,
        cot: false,
        bm25: false,
        claude_judge: false,
        claude_gen: false,
    }
}

#[test]
fn raw_if_wanted_off_returns_empty() {
    let pool = vec![fact(1, 0.9, 0.0)];
    let reached = vec![fact(2, 0.0, 0.5)];
    assert!(raw_if_wanted(&pool, &reached, false).is_empty());
}

#[test]
fn raw_if_wanted_dedups_a_fact_both_vector_ranked_and_graph_reached() {
    // Same id (7) present in both pool and reached — the raw capture must
    // list it once, not twice with split scores (the bug this test guards).
    let pool = vec![fact(1, 0.9, 0.0), fact(7, 0.6, 0.0)];
    let reached = vec![fact(7, 0.0, 0.8), fact(9, 0.0, 0.3)];
    let raw = raw_if_wanted(&pool, &reached, true);
    let ids: Vec<u64> = raw.iter().map(|f| f.id).collect();
    assert_eq!(ids.len(), 3, "1, 7, 9 — id 7 must not repeat");
    assert_eq!(ids.iter().filter(|&&id| id == 7).count(), 1);
}

#[test]
fn fuse_syncs_graph_weight_onto_a_fact_present_in_both_pool_and_reached() {
    // Fact 7 is a strong vector hit (pool) AND graph-reached (weight 0.8).
    // Its surviving copy (from the pool) must report that real weight, not
    // the vector pool's hardcoded 0.0 — otherwise --dump can't tell a
    // graph-promoted fact from a pure vector hit.
    let pool = vec![fact(1, 0.9, 0.0), fact(7, 0.6, 0.0)];
    let reached = vec![fact(7, 0.0, 0.8)];
    let fused = fuse(pool, &reached, cfg());
    let seven = fused.iter().find(|f| f.id == 7).expect("fact 7 present");
    assert!(
        (seven.graph_weight - 0.8).abs() < f64::EPSILON,
        "synced from the reached weight, got {}",
        seven.graph_weight
    );
}

#[test]
fn fuse_leaves_a_pure_vector_hit_at_zero_weight() {
    let pool = vec![fact(1, 0.9, 0.0)];
    let reached: Vec<RetrievedFact> = vec![];
    let fused = fuse(pool, &reached, cfg());
    assert!(fused[0].graph_weight.abs() < f64::EPSILON);
}
