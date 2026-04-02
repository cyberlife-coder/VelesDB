//! Graph bindings for VelesDB Python.
//!
//! Provides PyO3 wrappers for graph operations (nodes, edges, traversal).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};
use std::collections::HashMap;

use crate::utils::{json_to_python, python_to_json};
use velesdb_core::collection::graph::{GraphEdge, GraphNode, TraversalResult};

/// Parse a Python properties dict into a JSON properties map.
///
/// Used by both [`dict_to_node`] and [`dict_to_edge`] to avoid duplicating
/// the `PyObject` -> `HashMap<String, serde_json::Value>` conversion.
fn parse_properties(
    py: Python<'_>,
    props_obj: &PyObject,
) -> PyResult<HashMap<String, serde_json::Value>> {
    let props_dict: HashMap<String, PyObject> = props_obj.extract(py)?;
    props_dict
        .into_iter()
        .map(|(k, v)| python_to_json(py, &v).map(|jv| (k, jv)))
        .collect()
}

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
        node.with_properties(parse_properties(py, props)?)
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
        edge.with_properties(parse_properties(py, props)?)
    } else {
        edge
    };

    Ok(edge)
}

/// Convert a `GraphNode` to a Python dict, bypassing `HashMap` intermediary.
///
/// Uses `PyDict::new()` + `PyString::intern()` for static keys to avoid
/// repeated string allocation.
pub fn node_to_dict(py: Python<'_>, node: &GraphNode) -> PyObject {
    let dict = PyDict::new(py);
    let _ = dict.set_item(PyString::intern(py, "id"), node.id());
    let _ = dict.set_item(PyString::intern(py, "label"), node.label());

    let props = node.properties();
    if !props.is_empty() {
        let props_dict = PyDict::new(py);
        for (k, v) in props {
            let _ = props_dict.set_item(k.as_str(), json_to_python(py, v));
        }
        let _ = dict.set_item(PyString::intern(py, "properties"), props_dict);
    }

    if let Some(vec) = node.vector() {
        let np_vector = numpy::PyArray1::from_slice(py, vec);
        let _ = dict.set_item(PyString::intern(py, "vector"), np_vector);
    }

    dict.into_any().unbind()
}

/// Convert a `GraphEdge` to a Python dict, bypassing `HashMap` intermediary.
///
/// Uses `PyDict::new()` + `PyString::intern()` for static keys to avoid
/// repeated string allocation.
pub fn edge_to_dict(py: Python<'_>, edge: &GraphEdge) -> PyObject {
    let dict = PyDict::new(py);
    let _ = dict.set_item(PyString::intern(py, "id"), edge.id());
    let _ = dict.set_item(PyString::intern(py, "source"), edge.source());
    let _ = dict.set_item(PyString::intern(py, "target"), edge.target());
    let _ = dict.set_item(PyString::intern(py, "label"), edge.label());

    let props = edge.properties();
    if !props.is_empty() {
        let props_dict = PyDict::new(py);
        for (k, v) in props {
            let _ = props_dict.set_item(k.as_str(), json_to_python(py, v));
        }
        let _ = dict.set_item(PyString::intern(py, "properties"), props_dict);
    }

    dict.into_any().unbind()
}

/// Convert a `TraversalResult` to a Python dict, bypassing `HashMap` intermediary.
///
/// Uses `PyDict::new()` + `PyString::intern()` for static keys to avoid
/// repeated string allocation.
pub fn traversal_to_dict(py: Python<'_>, result: &TraversalResult) -> PyObject {
    let dict = PyDict::new(py);
    let _ = dict.set_item(PyString::intern(py, "target_id"), result.target_id);
    let _ = dict.set_item(PyString::intern(py, "path"), result.path.to_vec());
    let _ = dict.set_item(PyString::intern(py, "depth"), result.depth);
    dict.into_any().unbind()
}

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

            let obj = node_to_dict(py, &node);
            let dict = obj.bind(py).downcast::<PyDict>().unwrap();
            assert!(dict.contains("id").unwrap());
            assert!(dict.contains("label").unwrap());
        });
    }
}
