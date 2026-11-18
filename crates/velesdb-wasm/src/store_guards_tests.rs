//! The checks a browser `VectorStore` runs on vectors, as core runs them on a
//! collection: a metadata-only store refuses a vector search, a query or an
//! ingested vector needs the store's dimension and finite components, and a
//! hybrid search ranks a distance metric nearest-first.

use super::*;
use crate::store_insert::insert_with_payload;
use crate::store_state::StoreState;

fn code(result: Result<(), crate::wasm_error::WasmError>) -> &'static str {
    result.expect_err("refused").code()
}

#[test]
fn a_metadata_only_store_refuses_every_vector_search() {
    assert_eq!(code(validate_query_vector(&[], 0)), "VELES-015");
    // The batch shapes too: `0 vectors × dimension 0` used to pass the
    // length check and reach `chunks_exact(0)`, which panics.
    assert_eq!(code(validate_query_vectors(&[], 1, 0)), "VELES-015");
}

#[test]
fn a_query_needs_the_store_dimension_and_finite_components() {
    assert_eq!(code(validate_query_vector(&[1.0, 0.0], 3)), "VELES-004");
    assert_eq!(
        code(validate_query_vector(&[1.0, f32::NAN, 0.0], 3)),
        "VELES-005"
    );
    assert_eq!(
        code(validate_query_vectors(
            &[1.0, 0.0, f32::INFINITY, 0.0],
            2,
            2
        )),
        "VELES-005"
    );
    assert!(validate_query_vector(&[1.0, 0.0, 0.0], 3).is_ok());
    assert!(validate_query_vectors(&[1.0, 0.0, 0.0, 1.0], 2, 2).is_ok());
}

#[test]
fn an_ingested_vector_needs_the_store_dimension_and_finite_components() {
    assert_eq!(code(validate_ingest_vector(&[1.0], 2)), "VELES-004");
    assert_eq!(
        code(validate_ingest_vector(&[f32::NAN, 0.0], 2)),
        "VELES-005"
    );
    assert!(validate_ingest_vector(&[1.0, 0.0], 2).is_ok());
}

#[test]
fn hybrid_search_ranks_a_distance_metric_nearest_first() {
    for metric in ["euclidean", "hamming"] {
        let mut store = StoreState::new(2, metric).expect("test: store");
        let payload = Some(serde_json::json!({"t": "x"}));
        insert_with_payload(&mut store, 1, &[0.0, 0.0], payload.clone());
        insert_with_payload(&mut store, 2, &[9.0, 9.0], payload);
        let ids: Vec<u64> = store
            .hybrid_search_scored(&[0.0, 0.0], "zzz", 2, None)
            .expect("test: hybrid search")
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            ids,
            vec![1, 2],
            "{metric}: the exact match ranks first and is kept"
        );
    }
}

/// With the vector branch weighted 0 the search is text-only, under every
/// metric: a row the text does not match has nothing ranking it. A distance
/// metric kept such rows at score 0, filling `k` with non-matches, while a
/// similarity metric dropped them (Devin, #2830).
#[test]
fn hybrid_search_with_a_zero_vector_weight_returns_only_text_matches() {
    for metric in ["cosine", "euclidean", "hamming"] {
        let mut store = StoreState::new(2, metric).expect("test: store");
        insert_with_payload(
            &mut store,
            1,
            &[1.0, 0.0],
            Some(serde_json::json!({"t": "rust"})),
        );
        insert_with_payload(
            &mut store,
            2,
            &[1.0, 0.0],
            Some(serde_json::json!({"t": "go"})),
        );
        insert_with_payload(&mut store, 3, &[0.0, 1.0], None);
        let ids: Vec<u64> = store
            .hybrid_search_scored(&[1.0, 0.0], "rust", 3, Some(0.0))
            .expect("test: hybrid search")
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids, vec![1], "{metric}: only the text match");
    }
}

/// The blend weight is core's: `NaN.clamp(0.0, 1.0)` stayed `NaN`, which
/// emptied a similarity metric's result (no `NaN` score passes the relevance
/// floor) and handed a distance metric `NaN` scores in arbitrary order. The
/// store now refuses it, with core's `Config` code, under every metric.
#[test]
fn hybrid_search_refuses_a_non_finite_vector_weight() {
    for metric in ["cosine", "euclidean"] {
        let mut store = StoreState::new(2, metric).expect("test: store");
        insert_with_payload(
            &mut store,
            1,
            &[1.0, 0.0],
            Some(serde_json::json!({"t": "rust"})),
        );
        for weight in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let refused = store.hybrid_search_scored(&[1.0, 0.0], "rust", 1, Some(weight));
            assert_eq!(
                refused.map_err(|e| e.code()),
                Err("VELES-009"),
                "{metric}, {weight}"
            );
        }
    }
}
