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
