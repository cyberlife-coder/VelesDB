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
    DeleteEdgeStatement, EdgeQuery, InsertEdgeStatement, InsertNodeStatement, Query,
    SelectEdgesStatement,
};

use crate::database::DatabaseInner;
use crate::graph_store::WasmEdge;
use crate::observer::WasmQueryOperationKind;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Delegates MATCH execution to the dedicated module.
pub(crate) fn execute_match(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    Ok(match_candidates_gated(db, query, params, None)?
        .into_iter()
        .map(|c| c.row)
        .collect())
}

/// Runs a MATCH behind its read gate and returns the ordered, limited
/// candidates. Both `executeQuery` and `executeMatch` read through here, so
/// one gate covers every MATCH surface.
pub(crate) fn match_candidates_gated(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
    scoring: Option<&crate::velesql_match_score::MatchScoring>,
) -> Result<Vec<crate::velesql_match_orderby::MatchCandidate>, String> {
    // Read-path gate (audit F-5.4, #1392), asked about every collection the
    // MATCH reads. Zero-overhead without an observer.
    for collection in match_gate_collections(query, params)? {
        db.check_query_access(&collection, WasmQueryOperationKind::GraphTraversal)?;
    }
    crate::velesql_match::match_candidates(db, query, params, scoring)
}

/// Every collection a MATCH reads, for its read gate, in first-read order:
/// the graph it walks ([`crate::velesql_match::match_target`], the rule the
/// executor uses), then each `@collection` a node reads properties from.
/// A MATCH without patterns reads nothing and is asked about `""`.
fn match_gate_collections(query: &Query, params: &Params) -> Result<Vec<String>, String> {
    let Some(clause) = query.match_clause.as_ref() else {
        return Ok(vec![String::new()]);
    };
    if clause.patterns.is_empty() {
        return Ok(vec![String::new()]);
    }
    let mut collections = vec![crate::velesql_match::match_target(params)?];
    for name in clause
        .patterns
        .iter()
        .flat_map(|p| p.nodes.iter())
        .filter_map(|n| n.collection.as_deref())
    {
        if !collections.iter().any(|c| c == name) {
            collections.push(name.to_string());
        }
    }
    Ok(collections)
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
/// When the payload is a JSON object with a `_labels` array (core's key) or
/// a `labels` array (accepted for payloads written before), those strings
/// become the node's labels. `_labels` also stays in the payload, as it does
/// on a collection point and in core, so `RETURN n._labels` and `IS NULL`
/// read it; the legacy `labels` key is dropped, `labels` being the reserved
/// key a MATCH row writes the node's labels under. Any other shape is stored
/// verbatim with no labels.
fn split_node_payload_and_labels(
    payload: &serde_json::Value,
) -> (Option<serde_json::Value>, Vec<String>) {
    let serde_json::Value::Object(obj) = payload else {
        return (Some(payload.clone()), Vec::new());
    };
    let mut labels = Vec::new();
    let mut remainder = serde_json::Map::new();
    for (k, v) in obj {
        if k == "labels" || k == crate::velesql_match_nodes::LABELS_KEY {
            extend_labels_from_value(v, &mut labels);
            if k == "labels" {
                continue;
            }
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
///
/// Propagates the uniqueness error from
/// [`crate::graph_store::WasmGraphStore::insert_edge`] when the caller
/// supplies an explicit `id` that collides with an existing edge (Devin
/// Review Finding J).
pub(crate) fn insert_edge(
    db: &mut DatabaseInner,
    stmt: &InsertEdgeStatement,
    _params: &Params,
) -> Result<(), String> {
    let store = db.graph_store(&stmt.collection);
    let payload = if stmt.properties.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(
            EdgeQuery::literal_properties(&stmt.properties).map_err(|e| e.to_string())?,
        ))
    };
    store.borrow_mut().insert_edge(
        stmt.edge_id,
        stmt.source,
        stmt.target,
        stmt.label.clone(),
        payload,
    )?;
    Ok(())
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

/// Executes `SELECT EDGES FROM g [WHERE …]` as core does: the WHERE is read
/// by core's [`EdgeQuery`] (an unsupported shape is refused, never ignored)
/// and each edge answers core's row shape.
pub(crate) fn select_edges(
    db: &DatabaseInner,
    stmt: &SelectEdgesStatement,
    _params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    // Read-path gate (audit F-5.4, #1392) — zero-overhead without an observer.
    db.check_query_access(&stmt.collection, WasmQueryOperationKind::GraphTraversal)?;
    let store = db
        .get_graph_store(&stmt.collection)
        .ok_or_else(|| format!("Graph '{}' not found", stmt.collection))?;
    let query = EdgeQuery::from_where(stmt.where_clause.as_ref()).map_err(|e| e.to_string())?;
    let borrowed = store.borrow();

    let limit = usize::try_from(stmt.limit.unwrap_or(u64::MAX)).unwrap_or(usize::MAX);
    borrowed
        .filter_edges(query.source, query.target, query.label.as_deref())
        .filter(|edge| query.matches(edge.source, edge.target, &edge.label))
        .take(limit)
        .map(edge_to_row)
        .collect()
}

fn edge_to_row(edge: &WasmEdge) -> Result<QueryResultRow, String> {
    let no_properties = serde_json::Value::Object(serde_json::Map::new());
    QueryResultRow::synthetic(EdgeQuery::row(
        edge.id,
        edge.source,
        edge.target,
        &edge.label,
        edge.payload.as_ref().unwrap_or(&no_properties),
    ))
}

#[cfg(test)]
#[path = "velesql_graph_unit_tests.rs"]
mod unit_tests;
