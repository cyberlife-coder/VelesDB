//! Graph DML dispatch for the WASM VelesQL executor (S4-13).
//!
//! Implements a small subset of the graph surface:
//! - `INSERT NODE INTO g (id = N, payload = '…')`
//! - `INSERT EDGE INTO g (source = …, target = …, label = 'X', …)`
//! - `DELETE EDGE <id> FROM g`
//! - `SELECT EDGES FROM g [WHERE source=N | target=N | label='L']`
//!
//! MATCH execution lives in the sibling [`crate::velesql_match`] module so
//! each file stays within the project's 500-NLOC ceiling.

use velesdb_core::velesql::{
    CompareOp, Comparison, Condition, DeleteEdgeStatement, InsertEdgeStatement,
    InsertNodeStatement, Query, SelectEdgesStatement,
};

use crate::database::DatabaseInner;
use crate::graph_store::WasmEdge;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::{resolve_value, Params};

/// Delegates MATCH execution to the dedicated module.
pub(crate) fn execute_match(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    crate::velesql_match::execute_match(db, query, params)
}

// ---------------------------------------------------------------------------
// DML
// ---------------------------------------------------------------------------

/// Executes `INSERT NODE INTO g (id = N, payload = '…')`.
pub(crate) fn insert_node(
    db: &mut DatabaseInner,
    stmt: &InsertNodeStatement,
) -> Result<(), String> {
    let store = db.graph_store(&stmt.collection);
    let (payload, labels) = split_node_payload_and_labels(&stmt.payload);
    store
        .borrow_mut()
        .upsert_node(stmt.node_id, payload, labels);
    Ok(())
}

/// Splits a user-supplied node payload into `(value_payload, labels)`.
///
/// When the payload is a JSON object with a `labels` array, those strings
/// are extracted and the remaining fields form the value payload. Any
/// other shape is stored verbatim with no labels.
fn split_node_payload_and_labels(
    payload: &serde_json::Value,
) -> (Option<serde_json::Value>, Vec<String>) {
    let serde_json::Value::Object(obj) = payload else {
        return (Some(payload.clone()), Vec::new());
    };
    let mut labels = Vec::new();
    let mut remainder = serde_json::Map::new();
    for (k, v) in obj {
        if k == "labels" {
            extend_labels_from_value(v, &mut labels);
            continue;
        }
        remainder.insert(k.clone(), v.clone());
    }
    let payload = if remainder.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(remainder))
    };
    (payload, labels)
}

fn extend_labels_from_value(v: &serde_json::Value, labels: &mut Vec<String>) {
    let Some(arr) = v.as_array() else {
        return;
    };
    for item in arr {
        if let Some(s) = item.as_str() {
            labels.push(s.to_string());
        }
    }
}

/// Executes `INSERT EDGE INTO g (source = …, target = …, label = '…', …)`.
pub(crate) fn insert_edge(
    db: &mut DatabaseInner,
    stmt: &InsertEdgeStatement,
    params: &Params,
) -> Result<(), String> {
    let store = db.graph_store(&stmt.collection);
    let payload = build_edge_payload(&stmt.properties, params)?;
    store.borrow_mut().insert_edge(
        stmt.edge_id,
        stmt.source,
        stmt.target,
        stmt.label.clone(),
        payload,
    );
    Ok(())
}

fn build_edge_payload(
    properties: &[(String, velesdb_core::velesql::Value)],
    params: &Params,
) -> Result<Option<serde_json::Value>, String> {
    if properties.is_empty() {
        return Ok(None);
    }
    let mut map = serde_json::Map::new();
    for (k, v) in properties {
        map.insert(k.clone(), resolve_value(v, params)?);
    }
    Ok(Some(serde_json::Value::Object(map)))
}

/// Executes `DELETE EDGE <id> FROM g`.
pub(crate) fn delete_edge(
    db: &mut DatabaseInner,
    stmt: &DeleteEdgeStatement,
) -> Result<u32, String> {
    let Some(store) = db.get_graph_store(&stmt.collection) else {
        return Err(format!("Graph '{}' not found", stmt.collection));
    };
    let removed = store.borrow_mut().delete_edge_by_id(stmt.edge_id);
    Ok(u32::from(removed))
}

/// Executes `SELECT EDGES FROM g [WHERE source=N | target=N | label='L']`.
pub(crate) fn select_edges(
    db: &DatabaseInner,
    stmt: &SelectEdgesStatement,
    _params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let store = db
        .get_graph_store(&stmt.collection)
        .ok_or_else(|| format!("Graph '{}' not found", stmt.collection))?;
    let borrowed = store.borrow();
    let filter = extract_edge_filter(stmt.where_clause.as_ref());

    let mut out = Vec::new();
    let limit = stmt.limit.unwrap_or(u64::MAX);
    for edge in borrowed.filter_edges(filter.source, filter.target, filter.label.as_deref()) {
        if (out.len() as u64) >= limit {
            break;
        }
        out.push(edge_to_row(edge)?);
    }
    Ok(out)
}

/// Simple edge filter extracted from a flat WHERE clause (AND chain only).
#[derive(Default)]
struct EdgeFilter {
    source: Option<u64>,
    target: Option<u64>,
    label: Option<String>,
}

fn extract_edge_filter(cond: Option<&Condition>) -> EdgeFilter {
    let mut f = EdgeFilter::default();
    if let Some(c) = cond {
        fill_filter_from_condition(c, &mut f);
    }
    f
}

fn fill_filter_from_condition(cond: &Condition, f: &mut EdgeFilter) {
    match cond {
        Condition::Comparison(c) => apply_comparison_to_filter(c, f),
        Condition::And(l, r) => {
            fill_filter_from_condition(l, f);
            fill_filter_from_condition(r, f);
        }
        Condition::Group(inner) => fill_filter_from_condition(inner, f),
        _ => {}
    }
}

fn apply_comparison_to_filter(c: &Comparison, f: &mut EdgeFilter) {
    if !matches!(c.operator, CompareOp::Eq) {
        return;
    }
    match c.column.as_str() {
        "source" => f.source = value_to_u64(&c.value),
        "target" => f.target = value_to_u64(&c.value),
        "label" => {
            if let velesdb_core::velesql::Value::String(s) = &c.value {
                f.label = Some(s.clone());
            }
        }
        _ => {}
    }
}

fn value_to_u64(v: &velesdb_core::velesql::Value) -> Option<u64> {
    match v {
        velesdb_core::velesql::Value::Integer(i) => u64::try_from(*i).ok(),
        velesdb_core::velesql::Value::UnsignedInteger(u) => Some(*u),
        _ => None,
    }
}

fn edge_to_row(edge: &WasmEdge) -> Result<QueryResultRow, String> {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::json!(edge.id));
    map.insert("source".to_string(), serde_json::json!(edge.source));
    map.insert("target".to_string(), serde_json::json!(edge.target));
    map.insert("label".to_string(), serde_json::json!(edge.label.clone()));
    if let Some(serde_json::Value::Object(obj)) = &edge.payload {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    QueryResultRow::synthetic(serde_json::Value::Object(map))
}

#[cfg(test)]
#[path = "velesql_graph_unit_tests.rs"]
mod unit_tests;
