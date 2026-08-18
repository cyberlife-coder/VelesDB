use super::*;

#[test]
fn test_dict_to_node_minimal() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let mut dict = HashMap::new();
        dict.insert("id".to_string(), 1u64.into_pyobject(py).unwrap().into());
        dict.insert(
            "label".to_string(),
            "Person".into_pyobject(py).unwrap().into(),
        );

        let node = dict_to_node(py, &dict).unwrap();
        assert_eq!(node.id(), 1);
        assert_eq!(node.label(), "Person");
    });
}

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

#[test]
fn test_dict_to_node_rejects_nan_vector() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let mut dict = HashMap::new();
        dict.insert("id".to_string(), 1u64.into_pyobject(py).unwrap().into());
        dict.insert(
            "label".to_string(),
            "Person".into_pyobject(py).unwrap().into(),
        );
        let nan_vec = vec![1.0_f32, f32::NAN, 3.0];
        dict.insert(
            "vector".to_string(),
            nan_vec.into_pyobject(py).unwrap().into(),
        );

        let err = dict_to_node(py, &dict).unwrap_err();
        assert!(err.to_string().contains("NaN"));
    });
}

#[test]
fn test_node_to_dict() {
    pyo3::Python::initialize();
    Python::attach(|py| {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "name".to_string(),
            serde_json::Value::String("John".to_string()),
        );
        let node = GraphNode::new(1, "Person").with_properties(props);

        let obj = node_to_dict(py, &node);
        let dict = obj.bind(py).cast::<PyDict>().unwrap();
        assert!(dict.contains("id").unwrap());
        assert!(dict.contains("label").unwrap());
    });
}
