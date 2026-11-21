//! Primary scores reported by a device traversal are retained by row id.

use super::NativeScanScores;
use crate::collection::Collection;
use crate::index::hnsw::index::IndexRowScores;
use crate::{DistanceMetric, Point, SearchResult};

fn score_rows(points: Vec<Point>, q: &[f32], hits: &[(u64, f32)]) -> Vec<SearchResult> {
    let dir = tempfile::tempdir().expect("collection directory");
    let col = Collection::create(dir.path().to_owned(), q.len(), DistanceMetric::DotProduct)
        .expect("collection");
    col.upsert(points.clone()).expect("rows");
    let mut rows: Vec<_> = points
        .into_iter()
        .map(|p| SearchResult::new(p, 0.0))
        .collect();
    rows.sort_by_key(|r| r.point.id);
    let mut scores = NativeScanScores {
        anchors: None,
        query: q,
        pending: col.snapshot_pending(),
        metric: DistanceMetric::DotProduct,
        index: Some(IndexRowScores::Graph),
        pq: false,
        emitted: None,
    };
    scores.keep_emitted(
        hits.iter()
            .map(|&(id, score)| crate::scored_result::ScoredResult::new(id, score))
            .collect(),
    );
    for chunk in rows.chunks_mut(2) {
        col.apply_native_scan_scores(chunk, &mut scores);
    }
    rows
}

#[test]
fn only_emitted_ids_keep_their_actual_near_score_across_scan_chunks() {
    let q = [1.0];
    let points = [(7, 1.0), (14, 2.0), (31, 3.0), (42, 4.0)]
        .map(|(id, x)| Point::without_payload(id, vec![x]))
        .to_vec();
    let first = f32::from_bits(1.0_f32.to_bits() + 7);
    let last = f32::from_bits(4.0_f32.to_bits() - 3);
    // The search emits reverse id order and an inadmissible id between chunks.
    let rows = score_rows(points, &q, &[(42, last), (29, 7.0), (7, first)]);
    let got: Vec<_> = rows
        .iter()
        .map(|row| (row.point.id, row.score.to_bits()))
        .collect();
    assert_eq!(
        got,
        [
            (7, first.to_bits()),
            (14, 2.0_f32.to_bits()),
            (31, 3.0_f32.to_bits()),
            (42, last.to_bits())
        ]
    );
    for row in rows {
        let components = row.component_scores.expect("dense score");
        assert_eq!(components[0].1.to_bits(), row.score.to_bits());
    }
}

#[cfg(feature = "gpu")]
fn traverse_four_nodes(q: &[f32], vectors: &[Vec<f32>], ids: &[u64; 4]) -> Vec<(u64, f32)> {
    use crate::gpu::{gpu_csr::CsrGraph, GpuTraversalContext};
    let graph = CsrGraph {
        offsets: vec![0, 3, 6, 9, 12],
        neighbors: vec![1, 2, 3, 0, 2, 3, 0, 1, 3, 0, 1, 2],
        num_nodes: 4,
        max_degree: 3,
        total_edges: 12,
    };
    let ctx = GpuTraversalContext::global().expect("GPU device and traversal pipelines");
    let raw = ctx.search_layer0(
        &graph,
        &vectors.concat(),
        q,
        0,
        3,
        4,
        q.len(),
        DistanceMetric::DotProduct,
    );
    raw.into_iter()
        .map(|(slot, distance)| (ids[slot], -distance))
        .collect()
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires a working GPU for the HNSW traversal shader"]
fn real_gpu_traversal_scores_survive_ordered_scan_by_id() {
    const DIM: usize = 64;
    let ids = [101, 17, 83, 56];
    let q = vec![1.0; DIM];
    let mut cancellation = vec![1.0; DIM];
    cancellation[0] = 16_777_216.0;
    cancellation[DIM - 1] = -16_777_216.0;
    let vectors = [
        vec![0.0; DIM],
        cancellation,
        vec![2.0; DIM],
        vec![-2.0; DIM],
    ];
    let hits = traverse_four_nodes(&q, &vectors, &ids);
    assert_eq!(
        hits.len(),
        3,
        "three emitted scores and one admitted row outside the graph answer"
    );
    let host = DistanceMetric::DotProduct.calculate(&q, &vectors[1]);
    let device = hits
        .iter()
        .find(|(id, _)| *id == ids[1])
        .expect("cancellation hit from shader")
        .1;
    assert_ne!(
        device.to_bits(),
        host.to_bits(),
        "fixture must distinguish the two reductions"
    );
    let points = ids
        .into_iter()
        .zip(vectors)
        .map(|(id, v)| Point::without_payload(id, v))
        .collect();
    for row in score_rows(points, &q, &hits) {
        let expected = hits.iter().find(|(id, _)| *id == row.point.id).map_or_else(
            || DistanceMetric::DotProduct.calculate(&q, &row.point.vector),
            |hit| hit.1,
        );
        assert_eq!(
            row.score.to_bits(),
            expected.to_bits(),
            "row {}",
            row.point.id
        );
    }
}

fn native_pair_oracle(col: &Collection, predicate: &str, hint: &str) -> (bool, Vec<u64>) {
    use super::super::options::{QueryFinalizationContext, QuerySearchOptions};
    use crate::guardrails::{QueryContext, QueryLimits};
    let params = std::collections::HashMap::from([("q".to_owned(), serde_json::json!([1.0, 0.0]))]);
    let sql = crate::velesql::Parser::parse(&format!(
        "SELECT * FROM rows WHERE {predicate} ORDER BY similarity(vector, $q) DESC, id LIMIT 2 OFFSET 1 {hint}"
    )).expect("SQL");
    let stmt = &sql.select;
    let (_, fetch) = Collection::compute_fetch_limit(stmt);
    let unplanned = Collection::extract_query_components(stmt, &params).expect("unplanned");
    let mut opts =
        QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref()).expect("options");
    let (_, rows) = col
        .dispatch_main_select(stmt, &params, &unplanned, fetch, &opts)
        .expect("native rows");
    assert!(!rows.is_empty(), "{predicate} {hint}");
    let mut planned = Collection::extract_query_components(stmt, &params).expect("planned");
    col.select_fetch_window(stmt, &mut planned, &params, fetch, &mut opts);
    let context = QueryFinalizationContext {
        stmt,
        params: &params,
        limit: 2,
        extracted: &planned,
        ctx: &QueryContext::new(QueryLimits::new()),
        let_bindings: &[],
    };
    let pairs = col
        .ordered_device_hits(&context, &[1.0, 0.0], &opts, fetch)
        .expect("native pairs");
    let observed: Vec<_> = pairs
        .iter()
        .map(|hit| (hit.id, hit.score.to_bits()))
        .collect();
    let expected: Vec<_> = rows
        .iter()
        .map(|hit| (hit.point.id, hit.score.to_bits()))
        .collect();
    assert_eq!(observed, expected, "{predicate} {hint}");
    let used_bitmap = planned
        .ordered_prefilter
        .as_ref()
        .is_some_and(|p| p.bitmap.is_some());
    (used_bitmap, pairs.into_iter().map(|hit| hit.id).collect())
}

#[test]
fn native_score_projection_matches_rows_across_hints_bitmaps_and_thresholds() {
    let dir = tempfile::tempdir().expect("directory");
    let col =
        Collection::create(dir.path().to_owned(), 2, DistanceMetric::Cosine).expect("collection");
    let points = (1..=120u16)
        .map(|id| {
            let payload = serde_json::json!({"indexed": id % 3, "unindexed": id % 3});
            Point::new(
                u64::from(id),
                vec![f32::from(121 - id), 10.0],
                Some(payload),
            )
        })
        .collect::<Vec<_>>();
    col.upsert(points).expect("points");
    col.create_index("indexed").expect("prefilter");
    for hint in ["", "WITH (rerank=false)", "WITH (ef_search=64)"] {
        let (used, indexed) = native_pair_oracle(&col, "vector NEAR $q AND indexed = 0", hint);
        assert!(used);
        let (used, unindexed) = native_pair_oracle(&col, "vector NEAR $q AND unindexed = 0", hint);
        assert!(!used);
        assert!(indexed.iter().chain(&unindexed).all(|id| id % 3 == 0));
        let (_, near) = native_pair_oracle(&col, "vector NEAR $q", hint);
        assert_eq!(near.len(), 3);
        let (_, threshold) = native_pair_oracle(
            &col,
            "vector NEAR $q AND similarity(vector, $q) < 0.8 AND id > 3",
            hint,
        );
        assert!(
            threshold.iter().all(|id| *id > 100),
            "selectivity forces expansion beyond the first 30"
        );
    }
}
