//! The memory types and metadata, to and from the dicts and lists Python sees.

use crate::collection::query::convert_params;
use crate::utils::{json_to_python, opt_field};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use std::collections::HashMap;
use velesdb_memory::service::canonical_entity_name;
use velesdb_memory::{
    DatedContext, EntityProfile, EntityRelation, Explanation, FusionOptions, MemoryEdge,
    MemoryNode, Metadata, Recollection,
};

/// Convert a Python metadata/filter dict into the engine's [`Metadata`] map,
/// reusing the crate's `Py` → `serde_json::Value` conversion.
pub(super) fn to_metadata(
    py: Python<'_>,
    map: Option<HashMap<String, Py<PyAny>>>,
) -> PyResult<Option<Metadata>> {
    match map {
        None => Ok(None),
        Some(map) => {
            let converted = convert_params(py, Some(map))?;
            Ok(Some(converted.into_iter().collect()))
        }
    }
}

/// One [`Recollection`] as a Python dict `{id, score, content, metadata}`.
///
/// `metadata` mirrors `Recollection.metadata: Option<Map<String, Value>>` —
/// populated for `recall_where` results, `None` (i.e. Python `None`) for
/// `recall`/`why` (intentional, see the Rust doc on that field). Takes `r` by
/// value so the metadata map can be moved into the `serde_json::Value` instead
/// of cloned.
fn recollection_to_dict(py: Python<'_>, r: Recollection) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item(PyString::intern(py, "id"), r.id)?;
    dict.set_item(PyString::intern(py, "score"), r.score)?;
    dict.set_item(PyString::intern(py, "content"), &r.content)?;
    let metadata = match r.metadata {
        Some(map) => json_to_python(py, &serde_json::Value::Object(map)),
        None => py.None(),
    };
    dict.set_item(PyString::intern(py, "metadata"), metadata)?;
    Ok(dict.into())
}

/// Maps `items` to a Python list, one dict per item.
fn py_list_of<'py, T>(
    py: Python<'py>,
    items: &[T],
    to_dict: impl Fn(Python<'py>, &T) -> PyResult<Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for item in items {
        list.append(to_dict(py, item)?)?;
    }
    Ok(list)
}

/// One [`MemoryNode`] of an explanation subgraph as `{id, content, hop}`.
fn explanation_node_to_dict<'py>(py: Python<'py>, n: &MemoryNode) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(PyString::intern(py, "id"), n.id)?;
    d.set_item(PyString::intern(py, "content"), &n.content)?;
    d.set_item(PyString::intern(py, "hop"), n.hop)?;
    Ok(d)
}

/// One [`MemoryEdge`] of an explanation subgraph as `{from, to, relation}`.
fn explanation_edge_to_dict<'py>(
    py: Python<'py>,
    edge: &MemoryEdge,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(PyString::intern(py, "from"), edge.from)?;
    d.set_item(PyString::intern(py, "to"), edge.to)?;
    d.set_item(PyString::intern(py, "relation"), &edge.relation)?;
    Ok(d)
}

/// An [`Explanation`] as `{nodes: [{id, content, hop}], edges: [{from, to, relation}]}`.
pub(super) fn explanation_to_dict(py: Python<'_>, e: &Explanation) -> PyResult<Py<PyAny>> {
    let nodes = py_list_of(py, &e.nodes, explanation_node_to_dict)?;
    let edges = py_list_of(py, &e.edges, explanation_edge_to_dict)?;
    let out = PyDict::new(py);
    out.set_item(PyString::intern(py, "nodes"), nodes)?;
    out.set_item(PyString::intern(py, "edges"), edges)?;
    out.set_item(PyString::intern(py, "truncated"), e.truncated)?;
    Ok(out.into())
}

/// An [`EntityProfile`] lookup as
/// `{found, id, name, attributes, relations, relations_in,
/// relations_truncated, relations_in_truncated}`, where each edge
/// is `{predicate, target_id, target}`.
///
/// `relations` are the edges LEAVING the entity, `relations_in` those
/// pointing AT it — in that list, `target_id`/`target` name the far end the
/// edge comes FROM. Both are present on a MISS too, as empty lists: a shape
/// that changed with the outcome would force every caller to branch, and the
/// parity guard cannot catch it, since it reads a declaration and never an
/// execution path.
///
/// Takes the *queried* name, not just the outcome, because a miss carries no
/// name of its own: echoing the query back through
/// [`canonical_entity_name`] — the very function the service keys hubs by —
/// lets a caller running several lookups pair each answer with its question
/// (mirrors the MCP `entity` tool's `from_lookup`).
///
/// Built as a `serde_json::Value` and converted once by [`json_to_python`],
/// rather than field-by-field: that converter already renders a `u64` past
/// `i64::MAX` as an exact Python int (an entity id is a content hash, so it
/// routinely is one), and going through it keeps this shaping free of the
/// per-field fallible calls the graph-shaped helpers above still carry.
pub(super) fn entity_profile_to_value(
    queried: &str,
    profile: Option<EntityProfile>,
) -> serde_json::Value {
    let Some(profile) = profile else {
        return serde_json::json!({
            "found": false,
            "id": 0,
            "name": canonical_entity_name(queried),
            "attributes": {},
            "relations": [],
            "relations_in": [],
            "relations_truncated": false,
            "relations_in_truncated": false,
        });
    };
    serde_json::json!({
        "found": true,
        "id": profile.id,
        "name": profile.name,
        "attributes": serde_json::Value::Object(profile.attributes),
        "relations": entity_relations_to_value(profile.relations),
        "relations_in": entity_relations_to_value(profile.relations_in),
        "relations_truncated": profile.relations_truncated,
        "relations_in_truncated": profile.relations_in_truncated,
    })
}

/// One side of an entity's edges, as `[{predicate, target_id, target}, …]`.
///
/// Shared by both directions so the two lists can never drift into different
/// shapes — the failure a caller would meet as "the same edge looks different
/// depending on which end I asked from".
fn entity_relations_to_value(relations: Vec<EntityRelation>) -> Vec<serde_json::Value> {
    relations
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "predicate": r.predicate,
                "target_id": r.target_id,
                "target": r.target,
            })
        })
        .collect()
}

/// The shared `Vec<Recollection>` → Python list-of-dicts marshalling of every
/// recall path (`recall`, `recall_where`, `recall_fused`).
pub(super) fn recollections_to_list(
    py: Python<'_>,
    hits: Vec<Recollection>,
) -> PyResult<Bound<'_, PyList>> {
    let list = PyList::empty(py);
    for hit in hits {
        list.append(recollection_to_dict(py, hit)?)?;
    }
    Ok(list)
}

/// The `date_field` shape of `recall_fused`: the memories plus the
/// chronological timeline and its "now" anchor.
pub(super) fn dated_recall_to_dict(
    py: Python<'_>,
    memories: &Bound<'_, PyList>,
    ctx: DatedContext,
) -> PyResult<Py<PyAny>> {
    let out = PyDict::new(py);
    out.set_item(PyString::intern(py, "memories"), memories)?;
    out.set_item(PyString::intern(py, "dated_context"), ctx.timeline)?;
    out.set_item(PyString::intern(py, "now"), ctx.now)?;
    Ok(out.into())
}

/// Build [`FusionOptions`] from an optional Python `{hops?, graph_boost?, pool?}`
/// dict, routed through [`FusionOptions::from_knobs`] so the defaults and clamps
/// stay identical to the MCP tool. An absent dict (or one omitting a key) uses
/// the proven defaults.
pub(super) fn fusion_options_from_dict(
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<FusionOptions> {
    let Some(dict) = options else {
        return Ok(FusionOptions::from_knobs(None, None, None));
    };
    let hops = opt_field(dict, "hops")?;
    let graph_boost = opt_field(dict, "graph_boost")?;
    let pool = opt_field(dict, "pool")?;
    Ok(FusionOptions::from_knobs(hops, graph_boost, pool))
}
