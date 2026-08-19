use super::*;

#[test]
fn sparse_parallel_rejects_oversized_nnz() {
    let n = MAX_SPARSE_NNZ + 1;
    let err = SparseVectorInput::Parallel {
        indices: vec![0u32; n],
        values: vec![1.0f32; n],
    }
    .into_sparse_vector()
    .unwrap_err();
    assert!(err.contains("too large"), "expected size error, got: {err}");
}

#[test]
fn sparse_dict_rejects_oversized_nnz() {
    let map: BTreeMap<String, f32> = (0..=MAX_SPARSE_NNZ).map(|i| (i.to_string(), 1.0)).collect();
    let err = SparseVectorInput::Dict(map)
        .into_sparse_vector()
        .unwrap_err();
    assert!(err.contains("too large"), "expected size error, got: {err}");
}

#[test]
fn sparse_parallel_accepts_max_nnz() {
    let n = MAX_SPARSE_NNZ;
    let sv = SparseVectorInput::Parallel {
        indices: (0..u32::try_from(n).unwrap()).collect(),
        values: vec![1.0f32; n],
    }
    .into_sparse_vector();
    assert!(sv.is_ok());
}

#[test]
fn sparse_parallel_rejects_length_mismatch() {
    let err = SparseVectorInput::Parallel {
        indices: vec![0, 1],
        values: vec![1.0],
    }
    .into_sparse_vector()
    .unwrap_err();
    assert!(
        err.contains("mismatch"),
        "expected mismatch error, got: {err}"
    );
}

#[test]
fn sparse_parallel_rejects_non_finite_value() {
    let err = SparseVectorInput::Parallel {
        indices: vec![0],
        values: vec![f32::NAN],
    }
    .into_sparse_vector()
    .unwrap_err();
    assert!(
        err.contains("not finite"),
        "expected finite error, got: {err}"
    );
}
