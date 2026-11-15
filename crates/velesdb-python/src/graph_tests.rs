use super::*;

#[test]
fn test_dict_to_edge_minimal() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let mut dict = HashMap::new();
        dict.insert("id".to_string(), 100u64.into_pyobject(py).unwrap().into());
        dict.insert("source".to_string(), 1u64.into_pyobject(py).unwrap().into());
        dict.insert("target".to_string(), 2u64.into_pyobject(py).unwrap().into());
        dict.insert(
            "label".to_string(),
            "KNOWS".into_pyobject(py).unwrap().into(),
        );

        let edge = dict_to_edge(py, &dict).unwrap();
        assert_eq!(edge.id(), 100);
        assert_eq!(edge.source(), 1);
        assert_eq!(edge.target(), 2);
        assert_eq!(edge.label(), "KNOWS");
    });
}
