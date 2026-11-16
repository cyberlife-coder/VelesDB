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
    /// Resolves the target collection — any type: vector, graph and metadata
    /// collections share one edge store — and builds a `GraphEdge` from the
    /// statement fields.
    ///
    /// # Errors
    ///
    /// Returns `CollectionNotFound` if no collection has that name, or an
    /// error if the edge insertion fails.
    pub(super) fn execute_insert_edge(
        &self,
        stmt: &crate::velesql::InsertEdgeStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("INSERT_EDGE", &stmt.collection)?;
        let collection = self.resolve_edge_collection(&stmt.collection)?;
        let edge = build_graph_edge(stmt)?;
        collection.add_edge(edge)?;
        Ok(Vec::new())
    }

    /// Executes a DELETE FROM statement: removes the points its WHERE
    /// selects.
    ///
    /// # Errors
    ///
    /// Returns an error if the collection is not found or not writable, a
    /// named id is not a point id, or the WHERE holds a vector or graph
    /// predicate (DML filters on metadata).
    pub(super) fn execute_delete(
        &self,
        stmt: &crate::velesql::DeleteStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("DELETE", &stmt.table)?;
        let collection = self.resolve_writable_collection(&stmt.table)?;
        // `id = N` / `id IN (...)` name the points to remove, whatever they
        // carry (a graph node without a vector included). Any other WHERE
        // removes the rows it selects, as UPDATE selects them.
        let ids = if let Some(named) = point_ids_named(&stmt.where_clause) {
            named?
        } else {
            let filter = Self::build_dml_filter("DELETE", Some(&stmt.where_clause), params)?;
            Self::matching_points(&collection, filter.as_ref())
                .iter()
                .map(|point| point.id)
                .collect()
        };
        collection.delete(&ids)?;
        Ok(Vec::new())
    }

    /// Executes a DELETE EDGE statement.
    ///
    /// Resolves the target collection (any type, see
    /// [`Self::resolve_edge_collection`]) and removes the edge by ID.
    /// Returns a single `SearchResult` with `deleted: true/false` so the
    /// caller knows whether the edge actually existed.
    ///
    /// # Errors
    ///
    /// Returns `CollectionNotFound` if no collection has that name.
    pub(super) fn execute_delete_edge(
        &self,
        stmt: &crate::velesql::DeleteEdgeStatement,
    ) -> Result<Vec<SearchResult>> {
        self.check_dml_mutation("DELETE_EDGE", &stmt.collection)?;
        let collection = self.resolve_edge_collection(&stmt.collection)?;
        let removed = collection.remove_edge(stmt.edge_id);
        let payload = serde_json::json!({
            "deleted": removed,
            "edge_id": stmt.edge_id,
        });
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Executes a SELECT EDGES statement.
    ///
    /// Resolves the target collection (any type, see
    /// [`Self::resolve_edge_collection`]) and retrieves edges based on
    /// optional WHERE filters (source, target, label).
    ///
    /// # Errors
    ///
    /// Returns `CollectionNotFound` if no collection has that name, or a
    /// query error for an unsupported WHERE shape.
    pub(super) fn execute_select_edges(
        &self,
        stmt: &crate::velesql::SelectEdgesStatement,
    ) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_edge_collection(&stmt.collection)?;
        let edges = resolve_edge_query(&collection, stmt.where_clause.as_ref())?;
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
    /// Returns `CollectionNotFound` if no collection has that name,
    /// `GraphNotSupported` if it is not a graph collection (a node payload
    /// there would be a point with no vector), or an error if the payload
    /// storage fails.
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

    /// Resolves the collection an edge statement reads or writes.
    ///
    /// Vector, graph and metadata collections share one edge store (the Rust
    /// API, REST `/relations` and the agent-memory wedge add edges to all
    /// three), so edge statements accept any of them.
    fn resolve_edge_collection(&self, name: &str) -> Result<crate::collection::Collection> {
        self.resolve_collection(name)
    }

    /// Resolves a graph collection by name.
    ///
    /// Returns `CollectionNotFound` if no collection exists with the given
    /// name, and `GraphNotSupported` if one exists but is not a graph
    /// collection — so the error names the real reason.
    fn resolve_graph_collection(&self, name: &str) -> Result<crate::collection::GraphCollection> {
        if let Some(graph) = self.get_graph_collection(name) {
            return Ok(graph);
        }
        let kind = match self.get_any_collection(name) {
            Some(crate::collection::AnyCollection::Vector(_)) => "vector",
            Some(crate::collection::AnyCollection::Metadata(_)) => "metadata",
            Some(crate::collection::AnyCollection::Graph(_)) | None => {
                return Err(Error::CollectionNotFound(name.to_string()));
            }
        };
        Err(Error::GraphNotSupported(format!(
            "'{name}' is a {kind} collection; this statement needs a graph collection"
        )))
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
        .unwrap_or_else(|| crate::wire::hash_edge_id(stmt.source, stmt.target, &stmt.label));
    let edge = crate::collection::GraphEdge::new(edge_id, stmt.source, stmt.target, &stmt.label)?;

    if stmt.properties.is_empty() {
        return Ok(edge);
    }
    let props = resolve_edge_properties(&stmt.properties)?;
    Ok(edge.with_properties(props))
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

/// The point ids a DELETE WHERE names outright — `id = N` or
/// `id IN (N1, N2, ...)` — or `None` for any other WHERE.
///
/// # Errors
///
/// A named id that is not a point id.
fn point_ids_named(condition: &crate::velesql::Condition) -> Option<Result<Vec<u64>>> {
    match condition {
        crate::velesql::Condition::Comparison(cmp)
            if cmp.column == "id" && cmp.operator == crate::velesql::CompareOp::Eq =>
        {
            Some(extract_single_id(cmp))
        }
        crate::velesql::Condition::In(in_cond) if in_cond.column == "id" && !in_cond.negated => {
            Some(extract_in_ids(in_cond))
        }
        _ => None,
    }
}

/// Extracts a single ID from a comparison `id = N`.
fn extract_single_id(cmp: &crate::velesql::Comparison) -> Result<Vec<u64>> {
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
    collection: &crate::collection::Collection,
    where_clause: Option<&crate::velesql::Condition>,
) -> Result<Vec<crate::collection::GraphEdge>> {
    let Some(condition) = where_clause else {
        return Ok(collection.get_all_edges());
    };
    dispatch_edge_condition(collection, condition)
}

/// Dispatches a single WHERE condition to the appropriate edge lookup.
fn dispatch_edge_condition(
    collection: &crate::collection::Collection,
    condition: &crate::velesql::Condition,
) -> Result<Vec<crate::collection::GraphEdge>> {
    match condition {
        crate::velesql::Condition::Comparison(cmp) => dispatch_edge_comparison(collection, cmp),
        crate::velesql::Condition::And(left, right) => dispatch_edge_and(collection, left, right),
        _ => Err(Error::Query(
            "SELECT EDGES WHERE supports: source=N, target=N, label='X', and AND combinations"
                .to_string(),
        )),
    }
}

/// Handles a single comparison: source=N, target=N, or label='X'.
fn dispatch_edge_comparison(
    collection: &crate::collection::Collection,
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
            Ok(collection.get_outgoing_edges(id))
        }
        "target" => {
            let id = value_to_u64(&cmp.value)?;
            Ok(collection.get_incoming_edges(id))
        }
        "label" => {
            let label = extract_string_value(&cmp.value)?;
            Ok(collection.get_edges_by_label(&label))
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
    collection: &crate::collection::Collection,
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
    let mut edges = dispatch_edge_condition(collection, fetch)?;
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
