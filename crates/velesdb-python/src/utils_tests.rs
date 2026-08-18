use super::*;

#[test]
fn test_reject_nan_vector_clean() {
    assert!(reject_nan_vector(&[1.0, 2.0, 3.0]).is_ok());
}

#[test]
fn test_reject_nan_vector_contains_nan() {
    let err = reject_nan_vector(&[1.0, f32::NAN, 3.0]).unwrap_err();
    assert!(err.to_string().contains("non-finite"));
}

#[test]
fn test_reject_nan_vector_empty() {
    assert!(reject_nan_vector(&[]).is_ok());
}

#[test]
fn test_reject_nan_vector_positive_infinity() {
    let err = reject_nan_vector(&[1.0, f32::INFINITY, 3.0]).unwrap_err();
    assert!(err.to_string().contains("non-finite"));
}

#[test]
fn test_reject_nan_vector_negative_infinity() {
    let err = reject_nan_vector(&[1.0, f32::NEG_INFINITY, 3.0]).unwrap_err();
    assert!(err.to_string().contains("non-finite"));
}

#[test]
fn test_extract_vector_rejects_nan_list() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let list = vec![1.0_f32, f32::NAN, 3.0];
        let obj: Py<PyAny> = list
            .into_pyobject(py)
            .expect("test: convert Vec<f32> to Python list")
            .into();
        let err = extract_vector(py, &obj).unwrap_err();
        assert!(err.to_string().contains("non-finite"));
    });
}

#[test]
fn test_parse_metric_cosine() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_metric("cosine").expect("test: 'cosine' is a valid metric"),
            DistanceMetric::Cosine
        ));
        assert!(matches!(
            parse_metric("COSINE").expect("test: 'COSINE' is a valid metric (case-insensitive)"),
            DistanceMetric::Cosine
        ));
    });
}

#[test]
fn test_parse_metric_euclidean() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_metric("euclidean").expect("test: 'euclidean' is a valid metric"),
            DistanceMetric::Euclidean
        ));
        assert!(matches!(
            parse_metric("l2").expect("test: 'l2' is an alias for euclidean"),
            DistanceMetric::Euclidean
        ));
    });
}

#[test]
fn test_parse_metric_dot() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_metric("dot").expect("test: 'dot' is a valid metric"),
            DistanceMetric::DotProduct
        ));
        assert!(matches!(
            parse_metric("dotproduct").expect("test: 'dotproduct' is an alias for dot"),
            DistanceMetric::DotProduct
        ));
        assert!(matches!(
            parse_metric("ip").expect("test: 'ip' (inner product) is an alias for dot"),
            DistanceMetric::DotProduct
        ));
    });
}

#[test]
fn test_parse_metric_hamming() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_metric("hamming").expect("test: 'hamming' is a valid metric"),
            DistanceMetric::Hamming
        ));
    });
}

#[test]
fn test_parse_metric_jaccard() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_metric("jaccard").expect("test: 'jaccard' is a valid metric"),
            DistanceMetric::Jaccard
        ));
    });
}

#[test]
fn test_parse_metric_invalid() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(parse_metric("invalid").is_err());
    });
}

#[test]
fn test_parse_storage_mode_full() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_storage_mode("full").expect("test: 'full' is a valid storage mode"),
            StorageMode::Full
        ));
        assert!(matches!(
            parse_storage_mode("f32").expect("test: 'f32' is an alias for full"),
            StorageMode::Full
        ));
    });
}

#[test]
fn test_parse_storage_mode_sq8() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_storage_mode("sq8").expect("test: 'sq8' is a valid storage mode"),
            StorageMode::SQ8
        ));
        assert!(matches!(
            parse_storage_mode("int8").expect("test: 'int8' is an alias for sq8"),
            StorageMode::SQ8
        ));
    });
}

#[test]
fn test_parse_storage_mode_binary() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_storage_mode("binary").expect("test: 'binary' is a valid storage mode"),
            StorageMode::Binary
        ));
        assert!(matches!(
            parse_storage_mode("bit").expect("test: 'bit' is an alias for binary"),
            StorageMode::Binary
        ));
    });
}

#[test]
fn test_parse_storage_mode_pq() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_storage_mode("pq").expect("test: 'pq' is a valid storage mode"),
            StorageMode::ProductQuantization
        ));
        assert!(matches!(
            parse_storage_mode("product_quantization")
                .expect("test: 'product_quantization' is an alias for pq"),
            StorageMode::ProductQuantization
        ));
        // Case-insensitive (delegates to core `StorageMode::from_str`).
        assert!(matches!(
            parse_storage_mode("PQ").expect("test: 'PQ' is case-insensitive alias for pq"),
            StorageMode::ProductQuantization
        ));
    });
}

#[test]
fn test_parse_storage_mode_rabitq() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(matches!(
            parse_storage_mode("rabitq").expect("test: 'rabitq' is a valid storage mode"),
            StorageMode::RaBitQ
        ));
        // Case-insensitive (delegates to core `StorageMode::from_str`).
        assert!(matches!(
            parse_storage_mode("RaBitQ")
                .expect("test: 'RaBitQ' is case-insensitive alias for rabitq"),
            StorageMode::RaBitQ
        ));
        assert!(matches!(
            parse_storage_mode("RABITQ")
                .expect("test: 'RABITQ' is case-insensitive alias for rabitq"),
            StorageMode::RaBitQ
        ));
    });
}

#[test]
fn test_parse_storage_mode_invalid() {
    pyo3::Python::initialize();
    Python::attach(|_py| {
        assert!(parse_storage_mode("invalid").is_err());
    });
}
