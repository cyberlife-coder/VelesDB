//! Graph bindings for VelesDB Mobile (UniFFI).
//!
//! Provides UniFFI bindings for graph operations on iOS and Android.

use std::collections::HashMap;
use std::sync::Arc;

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

/// Traversal result from BFS.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraversalResult {
    /// Target node ID.
    pub node_id: u64,
    /// Depth from source.
    pub depth: u32,
}

/// In-memory graph store for mobile knowledge graphs.
#[derive(uniffi::Object)]
pub struct MobileGraphStore {
    nodes: std::sync::RwLock<HashMap<u64, MobileGraphNode>>,
    edges: std::sync::RwLock<HashMap<u64, MobileGraphEdge>>,
    outgoing: std::sync::RwLock<HashMap<u64, Vec<u64>>>,
    incoming: std::sync::RwLock<HashMap<u64, Vec<u64>>>,
}

#[uniffi::export]
impl MobileGraphStore {
    /// Creates a new empty graph store.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            nodes: std::sync::RwLock::new(HashMap::new()),
            edges: std::sync::RwLock::new(HashMap::new()),
            outgoing: std::sync::RwLock::new(HashMap::new()),
            incoming: std::sync::RwLock::new(HashMap::new()),
        })
    }

    /// Adds a node to the graph.
    pub fn add_node(&self, node: MobileGraphNode) {
        let mut nodes = self.nodes.write().unwrap();
        nodes.insert(node.id, node);
    }

    /// Adds an edge to the graph.
    pub fn add_edge(&self, edge: MobileGraphEdge) -> Result<(), crate::VelesError> {
        let mut edges = self.edges.write().unwrap();
        if edges.contains_key(&edge.id) {
            return Err(crate::VelesError::Database {
                message: format!("Edge with ID {} already exists", edge.id),
            });
        }

        let source = edge.source;
        let target = edge.target;
        let id = edge.id;

        edges.insert(id, edge);
        drop(edges);

        let mut outgoing = self.outgoing.write().unwrap();
        outgoing.entry(source).or_default().push(id);
        drop(outgoing);

        let mut incoming = self.incoming.write().unwrap();
        incoming.entry(target).or_default().push(id);

        Ok(())
    }

    /// Gets a node by ID.
    pub fn get_node(&self, id: u64) -> Option<MobileGraphNode> {
        let nodes = self.nodes.read().unwrap();
        nodes.get(&id).cloned()
    }

    /// Gets an edge by ID.
    pub fn get_edge(&self, id: u64) -> Option<MobileGraphEdge> {
        let edges = self.edges.read().unwrap();
        edges.get(&id).cloned()
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> u64 {
        let nodes = self.nodes.read().unwrap();
        nodes.len() as u64
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> u64 {
        let edges = self.edges.read().unwrap();
        edges.len() as u64
    }

    /// Gets outgoing edges from a node.
    pub fn get_outgoing(&self, node_id: u64) -> Vec<MobileGraphEdge> {
        let outgoing = self.outgoing.read().unwrap();
        let edges = self.edges.read().unwrap();

        outgoing
            .get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| edges.get(id).cloned()).collect())
            .unwrap_or_default()
    }

    /// Gets incoming edges to a node.
    pub fn get_incoming(&self, node_id: u64) -> Vec<MobileGraphEdge> {
        let incoming = self.incoming.read().unwrap();
        let edges = self.edges.read().unwrap();

        incoming
            .get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| edges.get(id).cloned()).collect())
            .unwrap_or_default()
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
        use std::collections::{HashSet, VecDeque};

        let mut results: Vec<TraversalResult> = Vec::new();
        let mut visited: HashSet<u64> = HashSet::new();
        let mut queue: VecDeque<(u64, u32)> = VecDeque::new();

        queue.push_back((source_id, 0));
        visited.insert(source_id);

        while let Some((node_id, depth)) = queue.pop_front() {
            if results.len() >= limit as usize {
                break;
            }

            if depth > 0 {
                results.push(TraversalResult { node_id, depth });
            }

            if depth < max_depth {
                for edge in self.get_outgoing(node_id) {
                    if !visited.contains(&edge.target) {
                        visited.insert(edge.target);
                        queue.push_back((edge.target, depth + 1));
                    }
                }
            }
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
        let mut edges = self.edges.write().unwrap();
        let mut outgoing = self.outgoing.write().unwrap();
        let mut incoming = self.incoming.write().unwrap();
        let mut nodes = self.nodes.write().unwrap();

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
    pub fn remove_edge(&self, edge_id: u64) {
        let mut edges = self.edges.write().unwrap();
        if let Some(edge) = edges.remove(&edge_id) {
            drop(edges);

            let mut outgoing = self.outgoing.write().unwrap();
            if let Some(ids) = outgoing.get_mut(&edge.source) {
                ids.retain(|&id| id != edge_id);
            }
            drop(outgoing);

            let mut incoming = self.incoming.write().unwrap();
            if let Some(ids) = incoming.get_mut(&edge.target) {
                ids.retain(|&id| id != edge_id);
            }
        }
    }

    /// Clears all nodes and edges.
    ///
    /// # Lock Order
    ///
    /// Acquires locks in consistent order: edges → outgoing → incoming → nodes
    pub fn clear(&self) {
        // Consistent lock order: edges → outgoing → incoming → nodes
        let mut edges = self.edges.write().unwrap();
        let mut outgoing = self.outgoing.write().unwrap();
        let mut incoming = self.incoming.write().unwrap();
        let mut nodes = self.nodes.write().unwrap();

        edges.clear();
        outgoing.clear();
        incoming.clear();
        nodes.clear();
    }
}

impl Default for MobileGraphStore {
    fn default() -> Self {
        Self {
            nodes: std::sync::RwLock::new(HashMap::new()),
            edges: std::sync::RwLock::new(HashMap::new()),
            outgoing: std::sync::RwLock::new(HashMap::new()),
            incoming: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let edge = MobileGraphEdge {
            id: 100,
            source: 1,
            target: 2,
            label: "KNOWS".to_string(),
            properties_json: None,
        };
        assert_eq!(edge.id, 100);
        assert_eq!(edge.source, 1);
        assert_eq!(edge.target, 2);
    }

    #[test]
    fn test_mobile_graph_store_add_nodes() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });

        assert_eq!(store.node_count(), 2);
    }

    #[test]
    fn test_mobile_graph_store_add_edges() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        assert_eq!(store.edge_count(), 1);
    }

    #[test]
    fn test_mobile_graph_store_get_neighbors() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 3,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();
        store
            .add_edge(MobileGraphEdge {
                id: 101,
                source: 1,
                target: 3,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        let neighbors = store.get_neighbors(1);
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&2));
        assert!(neighbors.contains(&3));
    }

    #[test]
    fn test_mobile_graph_store_bfs_traverse() {
        let store = MobileGraphStore::new();
        for i in 1..=4 {
            store.add_node(MobileGraphNode {
                id: i,
                label: "Node".to_string(),
                properties_json: None,
                vector: None,
            });
        }
        // 1 -> 2 -> 3 -> 4
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "NEXT".to_string(),
                properties_json: None,
            })
            .unwrap();
        store
            .add_edge(MobileGraphEdge {
                id: 101,
                source: 2,
                target: 3,
                label: "NEXT".to_string(),
                properties_json: None,
            })
            .unwrap();
        store
            .add_edge(MobileGraphEdge {
                id: 102,
                source: 3,
                target: 4,
                label: "NEXT".to_string(),
                properties_json: None,
            })
            .unwrap();

        let results = store.bfs_traverse(1, 3, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].node_id, 2);
        assert_eq!(results[0].depth, 1);
    }

    #[test]
    fn test_mobile_graph_store_get_incoming() {
        let store = MobileGraphStore::new();
        for i in 1..=3 {
            store.add_node(MobileGraphNode {
                id: i,
                label: "Person".to_string(),
                properties_json: None,
                vector: None,
            });
        }
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 3,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();
        store
            .add_edge(MobileGraphEdge {
                id: 101,
                source: 2,
                target: 3,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        let incoming = store.get_incoming(3);
        assert_eq!(incoming.len(), 2);
    }

    #[test]
    fn test_mobile_graph_store_remove_node() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        assert_eq!(store.node_count(), 2);
        assert_eq!(store.edge_count(), 1);

        store.remove_node(1);

        assert_eq!(store.node_count(), 1);
        assert_eq!(store.edge_count(), 0);
    }

    #[test]
    fn test_mobile_graph_store_clear() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        assert_eq!(store.node_count(), 2);
        assert_eq!(store.edge_count(), 1);

        store.clear();

        assert_eq!(store.node_count(), 0);
        assert_eq!(store.edge_count(), 0);
    }

    #[test]
    fn test_mobile_graph_store_get_node() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: Some(r#"{"name": "John"}"#.to_string()),
            vector: None,
        });

        let node = store.get_node(1);
        assert!(node.is_some());
        assert_eq!(node.unwrap().label, "Person");

        let missing = store.get_node(999);
        assert!(missing.is_none());
    }

    #[test]
    fn test_mobile_graph_store_get_edge() {
        let store = MobileGraphStore::new();
        store.add_node(MobileGraphNode {
            id: 1,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store.add_node(MobileGraphNode {
            id: 2,
            label: "Person".to_string(),
            properties_json: None,
            vector: None,
        });
        store
            .add_edge(MobileGraphEdge {
                id: 100,
                source: 1,
                target: 2,
                label: "KNOWS".to_string(),
                properties_json: None,
            })
            .unwrap();

        let edge = store.get_edge(100);
        assert!(edge.is_some());
        assert_eq!(edge.unwrap().label, "KNOWS");

        let missing = store.get_edge(999);
        assert!(missing.is_none());
    }

    /// Regression test for deadlock between add_edge() and remove_node()
    ///
    /// This test verifies that concurrent add_edge and remove_node operations
    /// do not deadlock due to inconsistent lock ordering.
    #[test]
    fn test_concurrent_add_edge_remove_node_no_deadlock() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(MobileGraphStore::new());

        // Pre-populate with nodes
        for i in 0u64..100 {
            store.add_node(MobileGraphNode {
                id: i,
                label: "Node".to_string(),
                properties_json: None,
                vector: None,
            });
        }

        let store_add = Arc::clone(&store);
        let store_remove = Arc::clone(&store);

        // Thread 1: continuously add edges
        let handle_add = thread::spawn(move || {
            for i in 0u64..50 {
                let _ = store_add.add_edge(MobileGraphEdge {
                    id: 1000 + i,
                    source: i % 100,
                    target: (i + 1) % 100,
                    label: "EDGE".to_string(),
                    properties_json: None,
                });
            }
        });

        // Thread 2: continuously remove nodes
        let handle_remove = thread::spawn(move || {
            for i in 50u64..100 {
                store_remove.remove_node(i);
            }
        });

        // If there's a deadlock, these joins will hang (test timeout)
        handle_add.join().expect("add thread should not panic");
        handle_remove
            .join()
            .expect("remove thread should not panic");

        // Verify store is in consistent state
        assert!(store.node_count() <= 100);
    }
}
