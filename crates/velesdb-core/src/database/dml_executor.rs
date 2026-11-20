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
        // The shape first, then the parameters, bound once: `id = $id` names
        // a point once `$id` is bound, and an unbound `$name` refuses the
        // statement (VELES-038) before any row is read.
        crate::velesql::check_dml_where("DELETE", Some(&stmt.where_clause))?;
        let where_clause = crate::velesql::bind_condition(&stmt.where_clause, params)?;
        let ids = Self::delete_target_ids(&collection, &where_clause)?;
        collection.delete(&ids)?;
        Ok(Vec::new())
    }

    /// The ids a DELETE with the bound `where_clause` removes: those
    /// `id = N` / `id IN (...)` name, whatever they carry (a graph node
    /// without a vector included); for any other WHERE, the rows it selects,
    /// as UPDATE selects them (`Collection::rows_where`).
    fn delete_target_ids(
        collection: &crate::collection::Collection,
        where_clause: &crate::velesql::Condition,
    ) -> Result<Vec<u64>> {
        if let Some(named) = point_ids_named(where_clause) {
            return named;
        }
        let filter = Self::build_dml_filter("DELETE", where_clause)?;
        Ok(collection
            .rows_where(Some(&filter))
            .iter()
            .map(|point| point.id)
            .collect())
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
            "edge_id": crate::api_types::serde_id::id_to_json(stmt.edge_id),
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
    pub(super) fn check_dml_mutation(&self, operation: &str, collection: &str) -> Result<()> {
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
    let props = crate::velesql::EdgeQuery::literal_properties(&stmt.properties)?;
    Ok(edge.with_properties(props.into_iter().collect()))
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
    let id = crate::velesql::literal_id(&cmp.value)?;
    Ok(vec![id])
}

/// Extracts multiple IDs from an IN condition `id IN (N1, N2, ...)`.
fn extract_in_ids(in_cond: &crate::velesql::InCondition) -> Result<Vec<u64>> {
    in_cond
        .values
        .iter()
        .map(crate::velesql::literal_id)
        .collect()
}

// ---------------------------------------------------------------------------
// SELECT EDGES helpers
// ---------------------------------------------------------------------------

/// Resolves edges from a graph collection based on an optional WHERE clause
/// ([`crate::velesql::EdgeQuery`]: `source = N`, `target = N`, `label = 'X'`
/// and AND combinations; anything else is refused).
///
/// The most selective constraint drives the lookup (source, then target,
/// then label); the others filter its edges.
fn resolve_edge_query(
    collection: &crate::collection::Collection,
    where_clause: Option<&crate::velesql::Condition>,
) -> Result<Vec<crate::collection::GraphEdge>> {
    let query = crate::velesql::EdgeQuery::from_where(where_clause)?;
    let mut edges = if let Some(source) = query.source {
        collection.get_outgoing_edges(source)
    } else if let Some(target) = query.target {
        collection.get_incoming_edges(target)
    } else if let Some(label) = &query.label {
        collection.get_edges_by_label(label)
    } else {
        collection.get_all_edges()
    };
    edges.retain(|e| query.matches(e.source(), e.target(), e.label()));
    Ok(edges)
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
            let payload = crate::velesql::EdgeQuery::row(
                edge.id(),
                edge.source(),
                edge.target(),
                edge.label(),
                &serde_json::json!(edge.properties()),
            );
            SearchResult::new(crate::Point::metadata_only(edge.id(), payload), 0.0)
        })
        .collect()
}
