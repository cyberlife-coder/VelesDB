use super::*;
use pyo3::Python;

/// extract_point_id error message includes the batch index.
#[test]
fn test_extract_point_id_missing_includes_index() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        // Dict with no "id" key — simulates a malformed point at position 4237.
        let empty: HashMap<String, Py<PyAny>> = HashMap::new();
        let err = extract_point_id(py, &empty, 4237).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("4237"),
            "error message must contain the batch index; got: {msg}"
        );
        assert!(
            msg.contains("'id'"),
            "error message must mention the missing field; got: {msg}"
        );
    });
}

/// extract_point_id at index 0 still includes the index in the message.
#[test]
fn test_extract_point_id_missing_at_index_zero() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let empty: HashMap<String, Py<PyAny>> = HashMap::new();
        let err = extract_point_id(py, &empty, 0).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("index 0"),
            "error message must contain 'index 0'; got: {msg}"
        );
    });
}

/// A multi-model row reads `vector_score`/`graph_score` from the components
/// core tagged, `None` when a tagged row lacks one, and carries
/// `component_scores` as `search_result_to_dict` does. An untagged row keeps
/// its score as `vector_score`, as core's `ScoreContext` reads it.
#[test]
fn test_multimodel_dict_reports_only_the_tagged_component_scores() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let point = || Point::new(7, vec![1.0, 0.0], None);
        let row = |result: &SearchResult| -> Bound<'_, PyDict> {
            search_result_to_multimodel_dict(py, result)
                .expect("test: convert the row")
                .into_bound(py)
                .cast_into::<PyDict>()
                .expect("test: a row is a dict")
        };
        let field = |dict: &Bound<'_, PyDict>, key: &str| -> Option<f32> {
            dict.get_item(key)
                .expect("test: read a key")
                .expect("test: the key is present")
                .extract()
                .expect("test: an optional float")
        };

        let text = SearchResult::with_component_scores(
            point(),
            0.5,
            [("bm25_score", 0.5)].into_iter().collect(),
        );
        let dict = row(&text);
        assert_eq!(field(&dict, "vector_score"), None);
        assert_eq!(field(&dict, "graph_score"), None);
        assert_eq!(field(&dict, "fused_score"), Some(0.5));
        assert!(dict
            .get_item("component_scores")
            .expect("test: read component_scores")
            .is_some());

        let hybrid = SearchResult::with_component_scores(
            point(),
            0.03,
            [("vector_score", 0.02), ("bm25_score", 0.01)]
                .into_iter()
                .collect(),
        );
        assert_eq!(field(&row(&hybrid), "vector_score"), Some(0.02));

        let untagged = row(&SearchResult::new(point(), 0.9));
        assert_eq!(field(&untagged, "vector_score"), Some(0.9));
        assert_eq!(field(&untagged, "graph_score"), None);
        assert!(untagged
            .get_item("component_scores")
            .expect("test: read component_scores")
            .is_none());
    });
}
