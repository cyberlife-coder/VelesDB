//! Graph bindings for VelesDB Python.
//!
//! Provides PyO3 wrappers for graph operations (nodes, edges, traversal).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::collections::HashMap;

use crate::utils::{json_to_python, to_pyobject};
use velesdb_core::collection::graph::{GraphEdge, GraphNode, TraversalResult};

/// Convert a Python dict to a GraphNode.
pub fn dict_to_node(py: Python<'_>, dict: &HashMap<String, PyObject>) -> PyResult<GraphNode> {
    let id: u64 = dict
        .get("id")
        .ok_or_else(|| PyValueError::new_err("Node missing 'id' field"))?
        .extract(py)?;

    let label: String = dict
        .get("label")
        .map(|l| l.extract(py))
        .transpose()?
        .unwrap_or_else(|| "Node".to_string());

    let node = GraphNode::new(id, &label);

    let node = if let Some(props) = dict.get("properties") {
        let props_dict: HashMap<String, PyObject> = props.extract(py)?;
        let mut properties = std::collections::HashMap::new();
        for (key, value) in props_dict {
            if let Ok(s) = value.extract::<String>(py) {
                properties.insert(key, serde_json::Value::String(s));
            } else if let Ok(i) = value.extract::<i64>(py) {
                properties.insert(key, serde_json::Value::Number(i.into()));
            } else if let Ok(f) = value.extract::<f64>(py) {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    properties.insert(key, serde_json::Value::Number(n));
                }
            } else if let Ok(b) = value.extract::<bool>(py) {
                properties.insert(key, serde_json::Value::Bool(b));
            }
        }
        node.with_properties(properties)
    } else {
        node
    };

    let node = if let Some(vector) = dict.get("vector") {
        let vec: Vec<f32> = vector.extract(py)?;
        node.with_vector(vec)
    } else {
        node
    };

    Ok(node)
}

/// Convert a Python dict to a GraphEdge.
pub fn dict_to_edge(py: Python<'_>, dict: &HashMap<String, PyObject>) -> PyResult<GraphEdge> {
    let id: u64 = dict
        .get("id")
        .ok_or_else(|| PyValueError::new_err("Edge missing 'id' field"))?
        .extract(py)?;

    let source: u64 = dict
        .get("source")
        .ok_or_else(|| PyValueError::new_err("Edge missing 'source' field"))?
        .extract(py)?;

    let target: u64 = dict
        .get("target")
        .ok_or_else(|| PyValueError::new_err("Edge missing 'target' field"))?
        .extract(py)?;

    let label: String = dict
        .get("label")
        .map(|l| l.extract(py))
        .transpose()?
        .unwrap_or_else(|| "RELATED_TO".to_string());

    let edge = GraphEdge::new(id, source, target, &label)
        .map_err(|e| PyValueError::new_err(format!("Invalid edge: {e}")))?;

    let edge = if let Some(props) = dict.get("properties") {
        let props_dict: HashMap<String, PyObject> = props.extract(py)?;
        let mut properties = std::collections::HashMap::new();
        for (key, value) in props_dict {
            if let Ok(s) = value.extract::<String>(py) {
                properties.insert(key, serde_json::Value::String(s));
            } else if let Ok(i) = value.extract::<i64>(py) {
                properties.insert(key, serde_json::Value::Number(i.into()));
            } else if let Ok(f) = value.extract::<f64>(py) {
                if let Some(n) = serde_json::Number::from_f64(f) {
                    properties.insert(key, serde_json::Value::Number(n));
                }
            } else if let Ok(b) = value.extract::<bool>(py) {
                properties.insert(key, serde_json::Value::Bool(b));
            }
        }
        edge.with_properties(properties)
    } else {
        edge
    };

    Ok(edge)
}

/// Convert a GraphNode to a Python dict.
pub fn node_to_dict(py: Python<'_>, node: &GraphNode) -> HashMap<String, PyObject> {
    let mut result = HashMap::new();
    result.insert("id".to_string(), to_pyobject(py, node.id()));
    result.insert("label".to_string(), to_pyobject(py, node.label()));

    let props = node.properties();
    if !props.is_empty() {
        let props_dict = PyDict::new(py);
        for (k, v) in props {
            props_dict
                .set_item(k, json_to_python(py, v))
                .unwrap_or_default();
        }
        result.insert("properties".to_string(), props_dict.into());
    }

    if let Some(vec) = node.vector() {
        result.insert("vector".to_string(), to_pyobject(py, vec.to_vec()));
    }

    result
}

/// Convert a GraphEdge to a Python dict.
pub fn edge_to_dict(py: Python<'_>, edge: &GraphEdge) -> HashMap<String, PyObject> {
    let mut result = HashMap::new();
    result.insert("id".to_string(), to_pyobject(py, edge.id()));
    result.insert("source".to_string(), to_pyobject(py, edge.source()));
    result.insert("target".to_string(), to_pyobject(py, edge.target()));
    result.insert("label".to_string(), to_pyobject(py, edge.label()));

    let props = edge.properties();
    if !props.is_empty() {
        let props_dict = PyDict::new(py);
        for (k, v) in props {
            props_dict
                .set_item(k, json_to_python(py, v))
                .unwrap_or_default();
        }
        result.insert("properties".to_string(), props_dict.into());
    }

    result
}

/// Convert a TraversalResult to a Python dict.
pub fn traversal_to_dict(py: Python<'_>, result: &TraversalResult) -> HashMap<String, PyObject> {
    let mut dict = HashMap::new();
    dict.insert("target_id".to_string(), to_pyobject(py, result.target_id));
    dict.insert("path".to_string(), to_pyobject(py, result.path.clone()));
    dict.insert("depth".to_string(), to_pyobject(py, result.depth));
    dict
}

use pyo3::types::PyDict;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dict_to_node_minimal() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
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
    fn test_node_to_dict() {
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            let mut props = std::collections::HashMap::new();
            props.insert(
                "name".to_string(),
                serde_json::Value::String("John".to_string()),
            );
            let node = GraphNode::new(1, "Person").with_properties(props);

            let dict = node_to_dict(py, &node);
            assert!(dict.contains_key("id"));
            assert!(dict.contains_key("label"));
        });
    }
}
