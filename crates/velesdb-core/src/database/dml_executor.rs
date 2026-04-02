//! DML mutation executor for `VelesQL` (INSERT EDGE, DELETE, DELETE EDGE,
//! SELECT EDGES, INSERT NODE).
//!
//! Extracted from `ddl_executor.rs` to keep each file under the 500 NLOC
//! limit.  DDL operations (CREATE/DROP/ALTER/ANALYZE/TRUNCATE) remain in
//! `ddl_executor.rs`.

use crate::{Error, Result, SearchResult};

use super::Database;

impl Database {
    /// Executes an INSERT EDGE statement.
    ///
    /// Resolves the target graph collection and builds a `GraphEdge`
    /// from the statement fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not a graph collection,
    /// or if the edge insertion fails.
    pub(super) fn execute_insert_edge(
        &self,
        stmt: &crate::velesql::InsertEdgeStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("INSERT_EDGE", &stmt.collection)?;
        let graph = self.resolve_graph_collection(&stmt.collection)?;
        let edge = build_graph_edge(stmt)?;
        graph.add_edge(edge)?;
        Ok(Vec::new())
    }

    /// Executes a DELETE FROM statement.
    ///
    /// Extracts point IDs from the WHERE clause and deletes them
    /// from the resolved collection.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not found, or the WHERE
    /// clause does not match a supported ID pattern.
    pub(super) fn execute_delete(
        &self,
        stmt: &crate::velesql::DeleteStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("DELETE", &stmt.table)?;
        let ids = extract_delete_ids(&stmt.where_clause)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;
        collection.delete(&ids)?;
        Ok(Vec::new())
    }

    /// Executes a DELETE EDGE statement.
    ///
    /// Resolves the target graph collection and removes the edge by ID.
    /// Returns a single `SearchResult` with `deleted: true/false` so the
    /// caller knows whether the edge actually existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not a graph collection.
    pub(super) fn execute_delete_edge(
        &self,
        stmt: &crate::velesql::DeleteEdgeStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("DELETE_EDGE", &stmt.collection)?;
        let graph = self.resolve_graph_collection(&stmt.collection)?;
        let removed = graph.remove_edge(stmt.edge_id);
        let payload = serde_json::json!({
            "deleted": removed,
            "edge_id": stmt.edge_id,
        });
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Executes a SELECT EDGES statement.
    ///
    /// Resolves the target graph collection and retrieves edges based on
    /// optional WHERE filters (source, target, label).
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not a graph collection.
    pub(super) fn execute_select_edges(
        &self,
        stmt: &crate::velesql::SelectEdgesStatement,
    ) -> Result<Vec<SearchResult>> {
        let graph = self.resolve_graph_collection(&stmt.collection)?;
        let edges = resolve_edge_query(&graph, stmt.where_clause.as_ref())?;
        let limit = resolve_edge_limit(stmt.limit, edges.len());
        Ok(edges_to_results(&edges[..limit]))
    }

    /// Executes an INSERT NODE statement.
    ///
    /// Resolves the target graph collection and stores or updates the
    /// node payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not a graph collection,
    /// or if the payload storage fails.
    pub(super) fn execute_insert_node(
        &self,
        stmt: &crate::velesql::InsertNodeStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("INSERT_NODE", &stmt.collection)?;
        let graph = self.resolve_graph_collection(&stmt.collection)?;
        graph.upsert_node_payload(stmt.node_id, &stmt.payload)?;
        Ok(Vec::new())
    }

    /// Checks the RBAC observer for DML mutation permission.
    ///
    /// Returns `Ok(())` if no observer is configured or if the observer
    /// allows the operation. Propagates the observer's error otherwise.
    fn check_dml_mutation(&self, operation: &str, collection: &str) -> Result<()> {
        if let Some(ref observer) = self.observer {
            observer.on_dml_mutation_request(operation, collection)?;
        }
        Ok(())
    }

    /// Resolves a graph collection by name.
    ///
    /// Returns `CollectionNotFound` if no graph collection exists with the given name.
    fn resolve_graph_collection(&self, name: &str) -> Result<crate::collection::GraphCollection> {
        self.get_graph_collection(name)
            .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Private helper functions
// ---------------------------------------------------------------------------

/// Builds a `GraphEdge` from an `InsertEdgeStatement`.
///
/// Resolves the edge ID (explicit or hashed), creates the edge, and
/// attaches properties if any are present.
fn build_graph_edge(
    stmt: &crate::velesql::InsertEdgeStatement,
) -> Result<crate::collection::GraphEdge> {
    let edge_id = stmt
        .edge_id
        .unwrap_or_else(|| hash_edge_id(stmt.source, stmt.target, &stmt.label));
    let edge = crate::collection::GraphEdge::new(edge_id, stmt.source, stmt.target, &stmt.label)?;

    if stmt.properties.is_empty() {
        return Ok(edge);
    }
    let props = resolve_edge_properties(&stmt.properties)?;
    Ok(edge.with_properties(props))
}

/// Generates a deterministic edge ID from (source, target, label) using FNV-1a.
///
/// # Determinism
///
/// The same (source, target, label) triple always produces the same ID.
/// This means re-inserting the same edge without an explicit `id` is
/// idempotent (overwrites the existing edge). To create multiple edges
/// with the same (source, target, label), provide explicit `id` values
/// in the SQL:
///
/// ```sql
/// INSERT EDGE INTO kg (id = 100, source = 1, target = 2, label = 'KNOWS');
/// INSERT EDGE INTO kg (id = 101, source = 1, target = 2, label = 'KNOWS');
/// ```
pub(super) fn hash_edge_id(source: u64, target: u64, label: &str) -> u64 {
    // FNV-1a offset basis and prime for u64
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    // Mix source bytes
    for byte in source.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    // Mix target bytes
    for byte in target.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    // Mix label bytes
    for byte in label.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Resolves AST `Value` properties into a `HashMap<String, serde_json::Value>`.
///
/// # Errors
///
/// Returns an error if a property value type is unsupported.
fn resolve_edge_properties(
    properties: &[(String, crate::velesql::Value)],
) -> Result<std::collections::HashMap<String, serde_json::Value>> {
    properties
        .iter()
        .map(|(key, val)| {
            match val {
                crate::velesql::Value::Parameter(_)
                | crate::velesql::Value::Temporal(_)
                | crate::velesql::Value::Subquery(_) => {
                    return Err(Error::Query(
                        "Edge properties must be literal values".to_string(),
                    ));
                }
                _ => {}
            }
            Ok((key.clone(), val.to_json()))
        })
        .collect()
}

/// Extracts point IDs from a DELETE WHERE clause.
///
/// Supports two patterns:
/// - `id = N` (single ID)
/// - `id IN (N1, N2, ...)` (multi-ID)
///
/// # Errors
///
/// Returns an error if the WHERE clause does not match a supported pattern.
fn extract_delete_ids(condition: &crate::velesql::Condition) -> Result<Vec<u64>> {
    match condition {
        crate::velesql::Condition::Comparison(cmp) if cmp.column == "id" => extract_single_id(cmp),
        crate::velesql::Condition::In(in_cond) if in_cond.column == "id" && !in_cond.negated => {
            extract_in_ids(in_cond)
        }
        _ => Err(Error::Query(
            "DELETE WHERE must use 'id = N' or 'id IN (N1, N2, ...)'".to_string(),
        )),
    }
}

/// Extracts a single ID from a comparison `id = N`.
fn extract_single_id(cmp: &crate::velesql::Comparison) -> Result<Vec<u64>> {
    if cmp.operator != crate::velesql::CompareOp::Eq {
        return Err(Error::Query(
            "DELETE WHERE id must use '=' operator".to_string(),
        ));
    }
    let id = value_to_u64(&cmp.value)?;
    Ok(vec![id])
}

/// Extracts multiple IDs from an IN condition `id IN (N1, N2, ...)`.
fn extract_in_ids(in_cond: &crate::velesql::InCondition) -> Result<Vec<u64>> {
    in_cond.values.iter().map(value_to_u64).collect()
}

/// Converts a `VelesQL` `Value` to a `u64` point ID.
///
/// # Errors
///
/// Returns an error if the value is not a non-negative integer.
fn value_to_u64(val: &crate::velesql::Value) -> Result<u64> {
    match val {
        crate::velesql::Value::Integer(v) => {
            u64::try_from(*v).map_err(|_| Error::Query(format!("ID must be non-negative, got {v}")))
        }
        crate::velesql::Value::UnsignedInteger(v) => Ok(*v),
        _ => Err(Error::Query("ID values must be integers".to_string())),
    }
}

// ---------------------------------------------------------------------------
// SELECT EDGES helpers
// ---------------------------------------------------------------------------

/// Resolves edges from a graph collection based on an optional WHERE clause.
///
/// Supports: `source = N`, `target = N`, `label = 'X'`, and AND combinations.
fn resolve_edge_query(
    graph: &crate::collection::GraphCollection,
    where_clause: Option<&crate::velesql::Condition>,
) -> Result<Vec<crate::collection::GraphEdge>> {
    let Some(condition) = where_clause else {
        return Ok(graph.get_edges(None));
    };
    dispatch_edge_condition(graph, condition)
}

/// Dispatches a single WHERE condition to the appropriate edge lookup.
fn dispatch_edge_condition(
    graph: &crate::collection::GraphCollection,
    condition: &crate::velesql::Condition,
) -> Result<Vec<crate::collection::GraphEdge>> {
    match condition {
        crate::velesql::Condition::Comparison(cmp) => dispatch_edge_comparison(graph, cmp),
        crate::velesql::Condition::And(left, right) => dispatch_edge_and(graph, left, right),
        _ => Err(Error::Query(
            "SELECT EDGES WHERE supports: source=N, target=N, label='X', and AND combinations"
                .to_string(),
        )),
    }
}

/// Handles a single comparison: source=N, target=N, or label='X'.
fn dispatch_edge_comparison(
    graph: &crate::collection::GraphCollection,
    cmp: &crate::velesql::Comparison,
) -> Result<Vec<crate::collection::GraphEdge>> {
    if cmp.operator != crate::velesql::CompareOp::Eq {
        return Err(Error::Query(
            "SELECT EDGES WHERE only supports '=' operator".to_string(),
        ));
    }
    match cmp.column.as_str() {
        "source" => {
            let id = value_to_u64(&cmp.value)?;
            Ok(graph.get_outgoing(id))
        }
        "target" => {
            let id = value_to_u64(&cmp.value)?;
            Ok(graph.get_incoming(id))
        }
        "label" => {
            let label = extract_string_value(&cmp.value)?;
            Ok(graph.get_edges(Some(&label)))
        }
        col => Err(Error::Query(format!(
            "SELECT EDGES WHERE does not support column '{col}'. Use: source, target, label"
        ))),
    }
}

/// Handles AND: (source=N AND label='X') or (target=N AND label='X').
///
/// Prefers the most selective condition (source > target > label) as
/// the fetch path, regardless of left/right ordering in the SQL.
/// This means `label = 'KNOWS' AND source = 1` is as efficient as
/// `source = 1 AND label = 'KNOWS'`.
fn dispatch_edge_and(
    graph: &crate::collection::GraphCollection,
    left: &crate::velesql::Condition,
    right: &crate::velesql::Condition,
) -> Result<Vec<crate::collection::GraphEdge>> {
    // Prefer fetching by source/target (most selective) as the primary
    // condition. If the right side has source/target and left does not,
    // swap so the more selective side drives the index lookup.
    let (fetch, filter_side) = if condition_selectivity(right) > condition_selectivity(left) {
        (right, left)
    } else {
        (left, right)
    };
    let mut edges = dispatch_edge_condition(graph, fetch)?;
    let filter = extract_and_filter(filter_side)?;
    edges.retain(|e| edge_matches_filter(e, &filter));
    Ok(edges)
}

/// Returns a selectivity score for a condition (higher = more selective).
///
/// `source` lookups are the most selective (indexed by node), followed
/// by `target`, then `label` (which may match many edges).
fn condition_selectivity(condition: &crate::velesql::Condition) -> u8 {
    match condition {
        crate::velesql::Condition::Comparison(cmp)
            if cmp.operator == crate::velesql::CompareOp::Eq =>
        {
            match cmp.column.as_str() {
                "source" => 3,
                "target" => 2,
                "label" => 1,
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// A simple filter derived from one side of an AND condition.
enum EdgeFilter {
    Source(u64),
    Target(u64),
    Label(String),
}

/// Extracts a filter from a condition for use in AND filtering.
fn extract_and_filter(condition: &crate::velesql::Condition) -> Result<EdgeFilter> {
    match condition {
        crate::velesql::Condition::Comparison(cmp)
            if cmp.operator == crate::velesql::CompareOp::Eq =>
        {
            match cmp.column.as_str() {
                "source" => Ok(EdgeFilter::Source(value_to_u64(&cmp.value)?)),
                "target" => Ok(EdgeFilter::Target(value_to_u64(&cmp.value)?)),
                "label" => Ok(EdgeFilter::Label(extract_string_value(&cmp.value)?)),
                col => Err(Error::Query(format!(
                    "SELECT EDGES WHERE does not support column '{col}'"
                ))),
            }
        }
        _ => Err(Error::Query(
            "SELECT EDGES AND condition must be a simple comparison (source=N, target=N, label='X')"
                .to_string(),
        )),
    }
}

/// Tests whether an edge matches a filter criterion.
fn edge_matches_filter(edge: &crate::collection::GraphEdge, filter: &EdgeFilter) -> bool {
    match filter {
        EdgeFilter::Source(id) => edge.source() == *id,
        EdgeFilter::Target(id) => edge.target() == *id,
        EdgeFilter::Label(lbl) => edge.label() == lbl,
    }
}

/// Extracts a string value from a `VelesQL` `Value`.
fn extract_string_value(val: &crate::velesql::Value) -> Result<String> {
    match val {
        crate::velesql::Value::String(s) => Ok(s.clone()),
        _ => Err(Error::Query("Expected a string value".to_string())),
    }
}

/// Resolves the effective LIMIT for a SELECT EDGES query.
fn resolve_edge_limit(limit: Option<u64>, total: usize) -> usize {
    match limit {
        Some(n) => usize::try_from(n).unwrap_or(usize::MAX).min(total),
        None => total,
    }
}

/// Converts a slice of `GraphEdge` into `SearchResult` items.
///
/// Each edge is represented as a `Point` with a JSON payload containing
/// `edge_id`, `source`, `target`, `label`, and `properties`.
fn edges_to_results(edges: &[crate::collection::GraphEdge]) -> Vec<SearchResult> {
    edges
        .iter()
        .map(|edge| {
            let payload = serde_json::json!({
                "edge_id": edge.id(),
                "source": edge.source(),
                "target": edge.target(),
                "label": edge.label(),
                "properties": edge.properties(),
            });
            SearchResult::new(crate::Point::metadata_only(edge.id(), payload), 0.0)
        })
        .collect()
}
