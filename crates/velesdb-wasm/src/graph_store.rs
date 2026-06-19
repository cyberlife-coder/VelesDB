//! Minimal in-memory graph store for WASM (S4-13).
//!
//! Supports the subset of graph operations that VelesQL demos exercise:
//! insert/delete nodes, insert/delete edges, filter edges, and walk 1- to
//! 2-hop patterns for `MATCH`. No persistence, no schema enforcement —
//! enough for an investor demo, not a substitute for `GraphCollection` in
//! `velesdb-core`.
//!
//! # Data model
//!
//! - Nodes: `id (u64)` → optional JSON payload, optional label list.
//! - Edges: append-only `Vec`, each entry `(id, source, target, label,
//!   payload)`. Auto-assigned edge ids derive from core's canonical
//!   `hash_edge_id(source, target, label)` so they match every other engine.
//!
//! Contention is not a concern because WASM is single-threaded.

use std::collections::HashMap;

/// A single directed edge in the in-memory graph.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WasmEdge {
    /// Monotonic edge identifier.
    pub id: u64,
    /// Source node id.
    pub source: u64,
    /// Target node id.
    pub target: u64,
    /// Edge label / type (e.g. `"KNOWS"`).
    pub label: String,
    /// Optional edge properties, serialized as a JSON object.
    pub payload: Option<serde_json::Value>,
}

/// A node in the in-memory graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WasmGraphNode {
    /// Optional JSON payload attached to the node.
    pub payload: Option<serde_json::Value>,
    /// Labels attached to the node (e.g. `["Person", "Author"]`).
    pub labels: Vec<String>,
}

/// Main in-memory graph store.
#[derive(Debug, Default)]
pub(crate) struct WasmGraphStore {
    nodes: HashMap<u64, WasmGraphNode>,
    edges: Vec<WasmEdge>,
}

impl WasmGraphStore {
    /// Creates an empty store.
    pub(crate) fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
        }
    }

    // --- Nodes -------------------------------------------------------------

    /// Upserts a node with the given id, optional payload, and optional
    /// labels. Idempotent: re-inserting the same id overwrites the previous
    /// payload/labels.
    pub(crate) fn upsert_node(
        &mut self,
        id: u64,
        payload: Option<serde_json::Value>,
        labels: Vec<String>,
    ) {
        self.nodes.insert(id, WasmGraphNode { payload, labels });
    }

    /// Returns the node with the given id, or `None` when absent.
    pub(crate) fn get_node(&self, id: u64) -> Option<&WasmGraphNode> {
        self.nodes.get(&id)
    }

    /// Returns every node id that carries the given label.
    pub(crate) fn nodes_with_label(&self, label: &str) -> Vec<u64> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.labels.iter().any(|l| l == label))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Returns every registered node id (irrespective of label).
    pub(crate) fn all_node_ids(&self) -> Vec<u64> {
        self.nodes.keys().copied().collect()
    }

    // --- Edges -------------------------------------------------------------

    /// Inserts a directed edge. If `explicit_id` is `Some`, uses it; else
    /// derives the id via core's canonical [`velesdb_core::hash_edge_id`]
    /// over (source, target, label) so the same logical edge matches the id
    /// produced by every other VelesDB engine. Returns the final edge id.
    ///
    /// # Errors
    ///
    /// Returns `Err` when `explicit_id` collides with an edge already in the
    /// store. Without this check, `delete_edge_by_id(n)` would delete every
    /// duplicate at once — a data-integrity risk for user SQL like
    /// `INSERT EDGE (id = 1, ...)` executed twice (Devin Review Finding J).
    pub(crate) fn insert_edge(
        &mut self,
        explicit_id: Option<u64>,
        source: u64,
        target: u64,
        label: String,
        payload: Option<serde_json::Value>,
    ) -> Result<u64, String> {
        if let Some(eid) = explicit_id {
            if self.edges.iter().any(|e| e.id == eid) {
                return Err(format!(
                    "Edge id {eid} already exists; explicit edge ids must be unique"
                ));
            }
        }
        let id = explicit_id.unwrap_or_else(|| {
            // Delegate to core's canonical edge-id derivation so the same
            // logical edge gets the same id across every VelesDB engine.
            velesdb_core::hash_edge_id(source, target, &label)
        });
        self.edges.push(WasmEdge {
            id,
            source,
            target,
            label,
            payload,
        });
        Ok(id)
    }

    /// Deletes an edge by id. Returns `true` if an edge was removed.
    pub(crate) fn delete_edge_by_id(&mut self, id: u64) -> bool {
        let before = self.edges.len();
        self.edges.retain(|e| e.id != id);
        before != self.edges.len()
    }

    /// Deletes all edges that satisfy `predicate`. Returns the count.
    #[allow(dead_code)] // Retained for future DELETE EDGE WHERE syntax.
    pub(crate) fn delete_edges_where<F>(&mut self, predicate: F) -> u64
    where
        F: Fn(&WasmEdge) -> bool,
    {
        let before = self.edges.len();
        self.edges.retain(|e| !predicate(e));
        (before - self.edges.len()) as u64
    }

    /// Returns every edge (immutable view).
    #[allow(dead_code)] // Used by tests + prepared for DESCRIBE GRAPH.
    pub(crate) fn edges(&self) -> &[WasmEdge] {
        &self.edges
    }

    /// Returns edges that match the given optional source / target / label
    /// filters. `None` filters accept everything on that axis.
    pub(crate) fn filter_edges<'a>(
        &'a self,
        source: Option<u64>,
        target: Option<u64>,
        label: Option<&'a str>,
    ) -> impl Iterator<Item = &'a WasmEdge> + 'a {
        self.edges.iter().filter(move |e| {
            source.is_none_or(|s| e.source == s)
                && target.is_none_or(|t| e.target == t)
                && label.is_none_or(|l| e.label == l)
        })
    }

    // --- MATCH helpers -----------------------------------------------------

    /// Returns every node id that either carries the given label or (if
    /// `label_filter` is None) exists in the store at all.
    pub(crate) fn candidate_nodes(&self, label_filter: Option<&str>) -> Vec<u64> {
        match label_filter {
            Some(l) => self.nodes_with_label(l),
            None => self.all_node_ids(),
        }
    }

    /// Removes every node and edge from the store.
    /// Used by `TRUNCATE COLLECTION` so the surrounding collection name
    /// keeps its identity but the graph data is wiped.
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_node_sets_labels() {
        let mut g = WasmGraphStore::new();
        g.upsert_node(
            1,
            Some(serde_json::json!({"name": "Alice"})),
            vec!["Person".to_string()],
        );
        let node = g.get_node(1).expect("test: node");
        assert_eq!(node.labels, vec!["Person".to_string()]);
    }

    #[test]
    fn test_insert_edge_auto_id_matches_core_canonical_hash() {
        let mut g = WasmGraphStore::new();
        let a = g
            .insert_edge(None, 1, 2, "KNOWS".to_string(), None)
            .expect("test: insert a");
        let b = g
            .insert_edge(None, 2, 3, "KNOWS".to_string(), None)
            .expect("test: insert b");
        // Auto-assigned ids come from core's canonical derivation, so the
        // same (source, target, label) triple yields the same id everywhere.
        assert_eq!(a, velesdb_core::hash_edge_id(1, 2, "KNOWS"));
        assert_eq!(b, velesdb_core::hash_edge_id(2, 3, "KNOWS"));
        assert_ne!(a, b, "distinct triples must yield distinct ids");
    }

    #[test]
    fn test_delete_edge_returns_true_on_match() {
        let mut g = WasmGraphStore::new();
        let id = g
            .insert_edge(None, 1, 2, "KNOWS".to_string(), None)
            .expect("test: insert");
        assert!(g.delete_edge_by_id(id));
        assert!(!g.delete_edge_by_id(id));
    }

    #[test]
    fn test_filter_edges_by_label() {
        let mut g = WasmGraphStore::new();
        g.insert_edge(None, 1, 2, "KNOWS".to_string(), None)
            .expect("test: knows");
        g.insert_edge(None, 2, 3, "LIKES".to_string(), None)
            .expect("test: likes");
        let hits: Vec<_> = g.filter_edges(None, None, Some("KNOWS")).collect();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn test_delete_edges_where() {
        let mut g = WasmGraphStore::new();
        g.insert_edge(None, 1, 2, "KNOWS".to_string(), None)
            .expect("test: e1");
        g.insert_edge(None, 1, 3, "KNOWS".to_string(), None)
            .expect("test: e2");
        g.insert_edge(None, 2, 3, "LIKES".to_string(), None)
            .expect("test: e3");
        let n = g.delete_edges_where(|e| e.source == 1);
        assert_eq!(n, 2);
        assert_eq!(g.edges().len(), 1);
    }

    #[test]
    fn test_nodes_with_label() {
        let mut g = WasmGraphStore::new();
        g.upsert_node(1, None, vec!["Person".to_string()]);
        g.upsert_node(2, None, vec!["Animal".to_string()]);
        g.upsert_node(3, None, vec!["Person".to_string()]);
        let people = g.nodes_with_label("Person");
        assert_eq!(people.len(), 2);
    }

    #[test]
    fn test_auto_id_is_independent_of_prior_explicit_id() {
        let mut g = WasmGraphStore::new();
        g.insert_edge(Some(100), 1, 2, "X".to_string(), None)
            .expect("test: explicit id");
        // A following auto insert derives its id from its own triple, not
        // from any monotonic counter influenced by the explicit id.
        let next = g
            .insert_edge(None, 2, 3, "Y".to_string(), None)
            .expect("test: next");
        assert_eq!(next, velesdb_core::hash_edge_id(2, 3, "Y"));
    }

    // --- Finding J: duplicate explicit edge id rejection -----------------

    #[test]
    fn test_insert_edge_with_duplicate_explicit_id_returns_error() {
        let mut g = WasmGraphStore::new();
        g.insert_edge(Some(1), 1, 2, "KNOWS".to_string(), None)
            .expect("test: first insert");
        let err = g.insert_edge(Some(1), 3, 4, "KNOWS".to_string(), None);
        assert!(err.is_err(), "duplicate explicit id must be rejected");
        let msg = err.expect_err("test: err");
        assert!(
            msg.contains("already exists") && msg.contains('1'),
            "error should mention existing id, got: {msg}"
        );
        // Store unchanged: only the first edge should exist.
        assert_eq!(g.edges().len(), 1);
    }

    #[test]
    fn test_insert_edge_with_auto_assigned_id_never_collides() {
        // Auto-assigned ids derive from the canonical (source, target, label)
        // hash; distinct triples yield distinct ids, so mixing one explicit
        // edge with several distinct auto edges stays collision-free.
        let mut g = WasmGraphStore::new();
        g.insert_edge(Some(42), 1, 2, "KNOWS".to_string(), None)
            .expect("test: explicit");
        for src in 10..20u64 {
            g.insert_edge(None, src, src + 1, "R".to_string(), None)
                .expect("test: auto");
        }
        // 1 explicit + 10 auto = 11 edges, all distinct ids.
        assert_eq!(g.edges().len(), 11);
        let mut ids: Vec<u64> = g.edges().iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 11, "every id must be unique");
    }

    #[test]
    fn test_insert_edge_after_delete_can_reuse_same_explicit_id() {
        let mut g = WasmGraphStore::new();
        g.insert_edge(Some(7), 1, 2, "KNOWS".to_string(), None)
            .expect("test: first");
        assert!(g.delete_edge_by_id(7));
        // Once freed, the explicit id is reusable.
        g.insert_edge(Some(7), 5, 6, "KNOWS".to_string(), None)
            .expect("test: reuse after delete");
        assert_eq!(g.edges().len(), 1);
        assert_eq!(g.edges()[0].source, 5);
    }
}
