//! Graph bindings for VelesDB Mobile (UniFFI).
//!
//! Provides UniFFI bindings for graph operations on iOS and Android.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

/// A graph node for knowledge graph construction.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileGraphNode {
    /// Unique identifier.
    pub id: u64,
    /// Node type/label.
    pub label: String,
    /// JSON properties as string.
    pub properties_json: Option<String>,
    /// Optional vector embedding.
    pub vector: Option<Vec<f32>>,
}

/// A graph edge representing a relationship.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MobileGraphEdge {
    /// Unique identifier.
    pub id: u64,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Relationship type.
    pub label: String,
    /// JSON properties as string.
    pub properties_json: Option<String>,
}

/// Traversal result from BFS/DFS.
///
/// FFI projection of [`velesdb_core::TraversalResult`]. The mobile field
/// `node_id` corresponds to core's `target_id` (the node reached); `path`
/// and `depth` mirror core's fields one-for-one. See the `From` impls below
/// for the canonical mapping — they make any future core field drift a
/// compile error rather than silent divergence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraversalResult {
    /// Target node ID reached (core: `target_id`).
    pub node_id: u64,
    /// Edge IDs along the path from the source to this node (core: `path`).
    pub path: Vec<u64>,
    /// Depth from source (number of hops).
    pub depth: u32,
}

/// Serializes a core property map to the mobile `properties_json` shape.
///
/// Returns `None` for an empty map (no properties) and a JSON object string
/// otherwise. A serialization failure also yields `None` so the projection is
/// total (FFI conversions cannot return a `Result`).
fn properties_to_json(
    properties: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    if properties.is_empty() {
        return None;
    }
    serde_json::to_string(properties).ok()
}

impl From<velesdb_core::GraphNode> for MobileGraphNode {
    fn from(node: velesdb_core::GraphNode) -> Self {
        Self {
            id: node.id(),
            label: node.label().to_string(),
            properties_json: properties_to_json(node.properties()),
            vector: node.vector().cloned(),
        }
    }
}

impl From<velesdb_core::GraphEdge> for MobileGraphEdge {
    fn from(edge: velesdb_core::GraphEdge) -> Self {
        Self {
            id: edge.id(),
            source: edge.source(),
            target: edge.target(),
            label: edge.label().to_string(),
            properties_json: properties_to_json(edge.properties()),
        }
    }
}

impl From<velesdb_core::TraversalResult> for TraversalResult {
    fn from(result: velesdb_core::TraversalResult) -> Self {
        Self {
            node_id: result.target_id,
            path: result.path,
            depth: result.depth,
        }
    }
}

/// In-memory graph store for mobile knowledge graphs.
#[derive(uniffi::Object)]
pub struct MobileGraphStore {
    nodes: RwLock<HashMap<u64, MobileGraphNode>>,
    edges: RwLock<HashMap<u64, MobileGraphEdge>>,
    outgoing: RwLock<HashMap<u64, Vec<u64>>>,
    incoming: RwLock<HashMap<u64, Vec<u64>>>,
}

#[uniffi::export]
impl MobileGraphStore {
    /// Creates a new empty graph store.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            nodes: RwLock::new(HashMap::new()),
            edges: RwLock::new(HashMap::new()),
            outgoing: RwLock::new(HashMap::new()),
            incoming: RwLock::new(HashMap::new()),
        })
    }

    /// Adds a node to the graph.
    pub fn add_node(&self, node: MobileGraphNode) {
        let mut nodes = self.nodes.write();
        nodes.insert(node.id, node);
    }

    /// Adds an edge to the graph.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing → incoming
    /// WITHOUT dropping between operations to ensure atomicity.
    /// This prevents race conditions with concurrent remove_node() calls.
    pub fn add_edge(&self, edge: MobileGraphEdge) -> Result<(), crate::VelesError> {
        // CRITICAL FIX: Acquire all locks BEFORE any mutation
        // and hold them until the operation is complete.
        // Lock order: edges → outgoing → incoming (consistent with remove_node)
        let mut edges = self.edges.write();
        let mut outgoing = self.outgoing.write();
        let mut incoming = self.incoming.write();

        if edges.contains_key(&edge.id) {
            return Err(crate::VelesError::Database {
                message: format!("Edge with ID {} already exists", edge.id),
            });
        }

        let source = edge.source;
        let target = edge.target;
        let id = edge.id;

        // All mutations happen while holding all locks
        edges.insert(id, edge);
        outgoing.entry(source).or_default().push(id);
        incoming.entry(target).or_default().push(id);

        // Locks are released here (all at once) when guards go out of scope
        Ok(())
    }

    /// Gets a node by ID.
    pub fn get_node(&self, id: u64) -> Option<MobileGraphNode> {
        let nodes = self.nodes.read();
        nodes.get(&id).cloned()
    }

    /// Gets an edge by ID.
    pub fn get_edge(&self, id: u64) -> Option<MobileGraphEdge> {
        let edges = self.edges.read();
        edges.get(&id).cloned()
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> u64 {
        let nodes = self.nodes.read();
        nodes.len() as u64
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> u64 {
        let edges = self.edges.read();
        edges.len() as u64
    }

    /// Gets outgoing edges from a node.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing
    /// to prevent ABBA deadlock with write operations.
    pub fn get_outgoing(&self, node_id: u64) -> Vec<MobileGraphEdge> {
        self.get_edges_from_index(node_id, &self.outgoing)
    }

    /// Gets incoming edges to a node.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → incoming
    /// to prevent ABBA deadlock with write operations.
    pub fn get_incoming(&self, node_id: u64) -> Vec<MobileGraphEdge> {
        self.get_edges_from_index(node_id, &self.incoming)
    }

    /// Gets outgoing edges filtered by label.
    pub fn get_outgoing_by_label(&self, node_id: u64, label: String) -> Vec<MobileGraphEdge> {
        self.get_outgoing(node_id)
            .into_iter()
            .filter(|e| e.label == label)
            .collect()
    }

    /// Gets neighbors reachable from a node (1-hop).
    pub fn get_neighbors(&self, node_id: u64) -> Vec<u64> {
        self.get_outgoing(node_id)
            .into_iter()
            .map(|e| e.target)
            .collect()
    }

    /// Performs BFS traversal from a source node.
    ///
    /// # Arguments
    ///
    /// * `source_id` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn bfs_traverse(&self, source_id: u64, max_depth: u32, limit: u32) -> Vec<TraversalResult> {
        self.bfs_traverse_parallel(vec![source_id], max_depth, limit)
    }

    /// Performs multi-source BFS traversal with deduplication.
    ///
    /// Starts BFS from multiple source nodes simultaneously and deduplicates
    /// results by target node ID (first-seen wins).
    ///
    /// # Arguments
    ///
    /// * `source_ids` - Starting node IDs
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn bfs_traverse_parallel(
        &self,
        source_ids: Vec<u64>,
        max_depth: u32,
        limit: u32,
    ) -> Vec<TraversalResult> {
        use std::collections::{HashSet, VecDeque};

        let mut results: Vec<TraversalResult> = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut queue: VecDeque<(u64, u32, Vec<u64>)> = VecDeque::new();

        for &source_id in &source_ids {
            if visited.insert(source_id) {
                queue.push_back((source_id, 0, Vec::new()));
            }
        }

        while let Some((node_id, depth, path)) = queue.pop_front() {
            if results.len() >= limit as usize {
                break;
            }

            if depth > 0 {
                results.push(TraversalResult {
                    node_id,
                    path: path.clone(),
                    depth,
                });
            }

            self.enqueue_neighbors(node_id, depth, max_depth, &path, &mut visited, &mut queue);
        }

        results
    }

    /// Removes a node and all connected edges.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing → incoming → nodes
    /// to prevent deadlock with concurrent add_edge() calls.
    pub fn remove_node(&self, node_id: u64) {
        // CRITICAL: Acquire locks in consistent order (edges → outgoing → incoming → nodes)
        // to prevent deadlock with add_edge() which uses (edges → outgoing → incoming)
        let mut edges = self.edges.write();
        let mut outgoing = self.outgoing.write();
        let mut incoming = self.incoming.write();
        let mut nodes = self.nodes.write();

        nodes.remove(&node_id);

        let outgoing_ids: Vec<u64> = outgoing.remove(&node_id).unwrap_or_default();
        for edge_id in outgoing_ids {
            if let Some(edge) = edges.remove(&edge_id) {
                if let Some(ids) = incoming.get_mut(&edge.target) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }

        let incoming_ids: Vec<u64> = incoming.remove(&node_id).unwrap_or_default();
        for edge_id in incoming_ids {
            if let Some(edge) = edges.remove(&edge_id) {
                if let Some(ids) = outgoing.get_mut(&edge.source) {
                    ids.retain(|&id| id != edge_id);
                }
            }
        }
    }

    /// Removes an edge by ID.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing → incoming
    /// WITHOUT dropping between operations to ensure atomicity.
    pub fn remove_edge(&self, edge_id: u64) {
        // CRITICAL FIX: Acquire all locks BEFORE any mutation
        let mut edges = self.edges.write();
        let mut outgoing = self.outgoing.write();
        let mut incoming = self.incoming.write();

        if let Some(edge) = edges.remove(&edge_id) {
            if let Some(ids) = outgoing.get_mut(&edge.source) {
                ids.retain(|&id| id != edge_id);
            }
            if let Some(ids) = incoming.get_mut(&edge.target) {
                ids.retain(|&id| id != edge_id);
            }
        }
        // All locks released here
    }

    /// Clears all nodes and edges.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing → incoming → nodes
    pub fn clear(&self) {
        // Consistent lock order: edges → outgoing → incoming → nodes
        let mut edges = self.edges.write();
        let mut outgoing = self.outgoing.write();
        let mut incoming = self.incoming.write();
        let mut nodes = self.nodes.write();

        edges.clear();
        outgoing.clear();
        incoming.clear();
        nodes.clear();
    }

    /// Performs DFS traversal from a source node.
    ///
    /// # Arguments
    ///
    /// * `source_id` - Starting node ID
    /// * `max_depth` - Maximum traversal depth
    /// * `limit` - Maximum number of results
    pub fn dfs_traverse(&self, source_id: u64, max_depth: u32, limit: u32) -> Vec<TraversalResult> {
        use std::collections::HashSet;

        let mut results: Vec<TraversalResult> = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut stack: Vec<(u64, u32, Vec<u64>)> = vec![(source_id, 0, Vec::new())];

        while let Some((node_id, depth, path)) = stack.pop() {
            if results.len() >= limit as usize {
                break;
            }

            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if depth > 0 {
                results.push(TraversalResult {
                    node_id,
                    path: path.clone(),
                    depth,
                });
            }

            if depth < max_depth {
                let neighbors: Vec<_> = self
                    .get_outgoing(node_id)
                    .into_iter()
                    .filter(|e| !visited.contains(&e.target))
                    .collect();

                for edge in neighbors.into_iter().rev() {
                    let mut next_path = path.clone();
                    next_path.push(edge.id);
                    stack.push((edge.target, depth + 1, next_path));
                }
            }
        }

        results
    }

    /// Checks if a node exists.
    pub fn has_node(&self, id: u64) -> bool {
        let nodes = self.nodes.read();
        nodes.contains_key(&id)
    }

    /// Checks if an edge exists.
    pub fn has_edge(&self, id: u64) -> bool {
        let edges = self.edges.read();
        edges.contains_key(&id)
    }

    /// Gets the out-degree (number of outgoing edges) of a node.
    #[allow(clippy::cast_possible_truncation)]
    pub fn out_degree(&self, node_id: u64) -> u32 {
        let outgoing = self.outgoing.read();
        // Safe: graph degree unlikely to exceed u32::MAX (4 billion edges from one node)
        outgoing.get(&node_id).map_or(0, |v| v.len() as u32)
    }

    /// Gets the in-degree (number of incoming edges) of a node.
    #[allow(clippy::cast_possible_truncation)]
    pub fn in_degree(&self, node_id: u64) -> u32 {
        let incoming = self.incoming.read();
        // Safe: graph degree unlikely to exceed u32::MAX (4 billion edges to one node)
        incoming.get(&node_id).map_or(0, |v| v.len() as u32)
    }

    /// Gets all nodes with a specific label.
    pub fn get_nodes_by_label(&self, label: String) -> Vec<MobileGraphNode> {
        let nodes = self.nodes.read();
        nodes
            .values()
            .filter(|n| n.label == label)
            .cloned()
            .collect()
    }

    /// Gets all edges with a specific label.
    pub fn get_edges_by_label(&self, label: String) -> Vec<MobileGraphEdge> {
        let edges = self.edges.read();
        edges
            .values()
            .filter(|e| e.label == label)
            .cloned()
            .collect()
    }
}

/// Internal helpers (not exposed via UniFFI).
impl MobileGraphStore {
    /// Resolves edge IDs from an adjacency index to full edge objects.
    ///
    /// # Lock Order
    ///
    /// Acquires `edges` read-lock first, then the `index` read-lock, matching
    /// the write-side lock order (edges -> outgoing -> incoming).
    fn get_edges_from_index(
        &self,
        node_id: u64,
        index: &RwLock<HashMap<u64, Vec<u64>>>,
    ) -> Vec<MobileGraphEdge> {
        let edges = self.edges.read();
        let idx = index.read();
        idx.get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| edges.get(id).cloned()).collect())
            .unwrap_or_default()
    }

    /// Enqueues unvisited outgoing neighbors of `node_id` for further traversal.
    ///
    /// Each enqueued entry carries the edge-ID path taken to reach the neighbor
    /// (`path` so far plus the traversed edge), mirroring core's
    /// `TraversalResult::path`. No-op when `depth` has already reached
    /// `max_depth`.
    fn enqueue_neighbors(
        &self,
        node_id: u64,
        depth: u32,
        max_depth: u32,
        path: &[u64],
        visited: &mut std::collections::HashSet<u64>,
        queue: &mut std::collections::VecDeque<(u64, u32, Vec<u64>)>,
    ) {
        if depth >= max_depth {
            return;
        }
        for edge in self.get_outgoing(node_id) {
            if visited.insert(edge.target) {
                let mut next_path = path.to_vec();
                next_path.push(edge.id);
                queue.push_back((edge.target, depth + 1, next_path));
            }
        }
    }
}

impl Default for MobileGraphStore {
    fn default() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            edges: RwLock::new(HashMap::new()),
            outgoing: RwLock::new(HashMap::new()),
            incoming: RwLock::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a test node with the given ID and "Person" label.
    fn person_node(id: u64) -> MobileGraphNode {
        MobileGraphNode {
            id,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        }
    }

    /// Creates a test edge with the given ID, source, and target ("KNOWS" label).
    fn knows_edge(id: u64, source: u64, target: u64) -> MobileGraphEdge {
        MobileGraphEdge {
            id,
            source,
            target,
            label: "KNOWS".to_string(),
            properties_json: None,
        }
    }

    /// Creates a store with nodes [1..=count] and returns it.
    fn store_with_nodes(count: u64) -> Arc<MobileGraphStore> {
        let store = MobileGraphStore::new();
        for i in 1..=count {
            store.add_node(person_node(i));
        }
        store
    }

    #[test]
    fn test_mobile_graph_node_creation() {
        let node = MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: Some(r#"{"name": "John"}"#.to_string()),
            vector: None,
        };
        assert_eq!(node.id, 1);
        assert_eq!(node.label, "Person");
    }

    #[test]
    fn test_mobile_graph_edge_creation() {
        let edge = knows_edge(100, 1, 2);
        assert_eq!(edge.id, 100);
        assert_eq!(edge.source, 1);
        assert_eq!(edge.target, 2);
    }

    #[test]
    fn test_mobile_graph_store_add_nodes() {
        let store = store_with_nodes(1);
        assert_eq!(store.node_count(), 1);
    }

    #[test]
    fn test_mobile_graph_store_add_edges() {
        let store = store_with_nodes(2);
        let result = store.add_edge(knows_edge(100, 1, 2));
        assert!(result.is_ok());
        assert_eq!(store.edge_count(), 1);
    }

    #[test]
    fn test_mobile_graph_store_duplicate_edge_error() {
        let store = store_with_nodes(2);
        let _ = store.add_edge(knows_edge(100, 1, 2));
        let result = store.add_edge(knows_edge(100, 1, 2));
        assert!(result.is_err());
    }

    #[test]
    fn test_mobile_graph_store_get_outgoing() {
        let store = store_with_nodes(3);
        let _ = store.add_edge(knows_edge(100, 1, 2));
        let _ = store.add_edge(knows_edge(101, 1, 3));
        assert_eq!(store.get_outgoing(1).len(), 2);
    }

    #[test]
    fn test_mobile_graph_store_bfs_traverse() {
        let store = store_with_nodes(4);

        // Create chain: 1 -> 2 -> 3 -> 4
        let _ = store.add_edge(knows_edge(100, 1, 2));
        let _ = store.add_edge(knows_edge(101, 2, 3));
        let _ = store.add_edge(knows_edge(102, 3, 4));

        let results = store.bfs_traverse(1, 3, 100);

        // Should find nodes 2, 3, 4 at depths 1, 2, 3, each carrying the
        // edge-ID path mirroring core's TraversalResult::path.
        assert_eq!(results.len(), 3);
        assert!(results
            .iter()
            .any(|r| r.node_id == 2 && r.depth == 1 && r.path == vec![100]));
        assert!(results
            .iter()
            .any(|r| r.node_id == 3 && r.depth == 2 && r.path == vec![100, 101]));
        assert!(results
            .iter()
            .any(|r| r.node_id == 4 && r.depth == 3 && r.path == vec![100, 101, 102]));
    }

    #[test]
    fn test_traversal_result_from_core() {
        let core = velesdb_core::TraversalResult::new(7, vec![10, 20], 2);
        let mobile: TraversalResult = core.into();
        assert_eq!(mobile.node_id, 7);
        assert_eq!(mobile.path, vec![10, 20]);
        assert_eq!(mobile.depth, 2);
    }

    #[test]
    fn test_graph_node_from_core() {
        let mut props = std::collections::HashMap::new();
        props.insert("name".to_string(), serde_json::json!("Alice"));
        let core = velesdb_core::GraphNode::new(1, "Person")
            .with_properties(props)
            .with_vector(vec![0.1, 0.2]);
        let mobile: MobileGraphNode = core.into();
        assert_eq!(mobile.id, 1);
        assert_eq!(mobile.label, "Person");
        assert_eq!(mobile.vector, Some(vec![0.1, 0.2]));
        assert!(mobile.properties_json.is_some());
    }

    #[test]
    fn test_graph_edge_from_core() -> Result<(), velesdb_core::Error> {
        let core = velesdb_core::GraphEdge::new(100, 1, 2, "KNOWS")?;
        let mobile: MobileGraphEdge = core.into();
        assert_eq!(mobile.id, 100);
        assert_eq!(mobile.source, 1);
        assert_eq!(mobile.target, 2);
        assert_eq!(mobile.label, "KNOWS");
        assert_eq!(mobile.properties_json, None);
        Ok(())
    }

    #[test]
    fn test_mobile_graph_store_remove_node() {
        let store = store_with_nodes(2);
        let _ = store.add_edge(knows_edge(100, 1, 2));

        assert_eq!(store.node_count(), 2);
        assert_eq!(store.edge_count(), 1);

        store.remove_node(1);

        assert_eq!(store.node_count(), 1);
        assert_eq!(store.edge_count(), 0); // Edge should be removed too
    }

    #[test]
    fn test_mobile_graph_store_remove_edge() {
        let store = store_with_nodes(2);
        let _ = store.add_edge(knows_edge(100, 1, 2));

        assert_eq!(store.edge_count(), 1);

        store.remove_edge(100);

        assert_eq!(store.edge_count(), 0);
        assert!(store.get_outgoing(1).is_empty());
        assert!(store.get_incoming(2).is_empty());
    }

    #[test]
    fn test_mobile_graph_store_clear() {
        let store = store_with_nodes(2);
        let _ = store.add_edge(knows_edge(100, 1, 2));

        store.clear();

        assert_eq!(store.node_count(), 0);
        assert_eq!(store.edge_count(), 0);
    }
}
