//! Graph migration phase -- migrates FK relations as graph edges.

use tracing::{debug, info, warn};

use crate::config::{MigrationConfig, RelationConfig};
use crate::connectors::SourceConnector;
use crate::error::{Error, Result};
use crate::pipeline_points::stable_point_id;

/// Statistics from graph migration.
#[derive(Debug, Default, Clone)]
pub struct GraphMigrationStats {
    /// Total relations processed.
    pub relations_processed: usize,
    /// Total edges successfully created.
    pub edges_created: u64,
    /// Total edges that failed to create.
    pub edges_failed: u64,
}

/// Graph migration phase: migrates FK relations as edges in a `GraphCollection`.
pub struct GraphMigrationPhase<'a> {
    config: &'a MigrationConfig,
    connector: Box<dyn SourceConnector>,
}

impl<'a> GraphMigrationPhase<'a> {
    /// Creates a new graph migration phase.
    pub fn new(config: &'a MigrationConfig, connector: Box<dyn SourceConnector>) -> Self {
        Self { config, connector }
    }

    /// Connects the underlying source connector.
    ///
    /// # Errors
    ///
    /// Returns an error if the source connection fails.
    pub async fn connect(&mut self) -> Result<()> {
        self.connector.connect().await
    }

    /// Runs the graph migration.
    ///
    /// Iterates over all configured relations, extracts FK columns from the
    /// source, and inserts edges into the `VelesDB` `GraphCollection`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database or graph collection cannot be
    /// opened or created.
    pub async fn run(
        &self,
        db: &velesdb_core::Database,
    ) -> Result<GraphMigrationStats> {
        let graph_name = self
            .config
            .destination
            .graph_collection
            .as_deref()
            .ok_or_else(|| Error::Config("graph_collection not configured".to_string()))?;

        ensure_graph_collection_exists(db, graph_name)?;

        let gc = db
            .get_graph_collection(graph_name)
            .ok_or_else(|| {
                Error::DestinationConnection("Graph collection not found".to_string())
            })?;

        let relations = &self.config.relations;
        if relations.is_empty() {
            info!("No relations configured, skipping graph phase");
            return Ok(GraphMigrationStats::default());
        }

        // TODO(US-GRAPH-01): single-pass extraction — currently opens a second
        // connector and scans the entire source again for graph edges. This doubles
        // I/O for large sources. Future improvement: extract edges inline during
        // the main vector pipeline pass.
        let mut stats = GraphMigrationStats::default();
        let batch_size = self.config.options.batch_size;

        for relation in relations {
            info!(
                "Migrating relation: {} -> {}.{} [{}]",
                relation.from_column,
                relation.to_table,
                relation.to_column,
                relation.edge_label
            );

            let edges = self
                .extract_edges_for_relation(relation, batch_size)
                .await?;

            let total = edges.len() as u64;
            let inserted = gc.add_edges_batch(edges) as u64;
            stats.edges_created += inserted;
            stats.edges_failed += total.saturating_sub(inserted);
            stats.relations_processed += 1;
        }

        gc.flush_full()
            .map_err(|e| Error::DestinationConnection(format!("Graph flush failed: {e}")))?;

        info!(
            "Graph migration complete: {} edges created, {} failed",
            stats.edges_created, stats.edges_failed
        );

        Ok(stats)
    }

    async fn extract_edges_for_relation(
        &self,
        relation: &RelationConfig,
        batch_size: usize,
    ) -> Result<Vec<velesdb_core::GraphEdge>> {
        let mut edges = Vec::new();
        let mut offset = None;

        loop {
            let batch = self.connector.extract_batch(offset.clone(), batch_size).await?;

            for point in &batch.points {
                if let Some(edge) = build_edge(point, relation) {
                    edges.push(edge);
                } else {
                    debug!(
                        "Skipping point '{}': unsupported type or missing column '{}'",
                        point.id, relation.from_column
                    );
                }
            }

            if !batch.has_more {
                break;
            }
            offset = batch.next_offset;
        }

        Ok(edges)
    }
}

fn ensure_graph_collection_exists(
    db: &velesdb_core::Database,
    name: &str,
) -> Result<()> {
    if db.get_graph_collection(name).is_some() {
        return Ok(());
    }
    db.create_graph_collection(name, velesdb_core::GraphSchema::schemaless())
        .map_err(|e| {
            Error::DestinationConnection(format!("Cannot create graph collection: {e}"))
        })
}

fn build_edge(
    point: &crate::connectors::ExtractedPoint,
    relation: &RelationConfig,
) -> Option<velesdb_core::GraphEdge> {
    let from_node_id = stable_point_id(&point.id);

    let fk_value = point.payload.get(&relation.from_column)?;
    let fk_str = value_to_id_str(fk_value)?;
    let to_node_id = stable_point_id(&fk_str);

    let key = format!("{from_node_id}-{to_node_id}-{}", relation.edge_label);
    let id = crate::pipeline::fnv1a64(key.as_bytes());

    let edge = match velesdb_core::GraphEdge::new(id, from_node_id, to_node_id, &relation.edge_label) {
        Ok(e) => e,
        Err(e) => {
            warn!("Failed to create edge {}: {}", id, e);
            return None;
        }
    };

    Some(attach_weight(edge, point, relation))
}

fn attach_weight(
    edge: velesdb_core::GraphEdge,
    point: &crate::connectors::ExtractedPoint,
    relation: &RelationConfig,
) -> velesdb_core::GraphEdge {
    let weight_col = match relation.weight_column {
        Some(ref col) => col,
        None => return edge,
    };

    let weight = match point.payload.get(weight_col).and_then(|v| v.as_f64()) {
        Some(w) => w,
        None => return edge,
    };

    let mut props = std::collections::HashMap::new();
    props.insert("weight".to_string(), serde_json::json!(weight));
    edge.with_properties(props)
}

fn value_to_id_str(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::ExtractedPoint;

    fn make_point(id: &str, payload: serde_json::Value) -> ExtractedPoint {
        let payload_map = payload
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        ExtractedPoint {
            id: id.to_string(),
            vector: vec![],
            payload: payload_map,
            sparse_vector: None,
        }
    }

    fn make_relation(from: &str, label: &str) -> RelationConfig {
        RelationConfig {
            from_column: from.to_string(),
            to_table: "target".to_string(),
            to_column: "id".to_string(),
            edge_label: label.to_string(),
            weight_column: None,
        }
    }

    #[test]
    fn test_build_edge_string_fk() {
        let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
        let relation = make_relation("author_id", "AUTHORED_BY");
        let edge = build_edge(&point, &relation);
        assert!(edge.is_some());
        let e = edge.expect("test: edge should be Some");
        assert_eq!(e.source(), stable_point_id("doc-1"));
        assert_eq!(e.target(), stable_point_id("auth-42"));
    }

    #[test]
    fn test_build_edge_numeric_fk() {
        let point = make_point("99", serde_json::json!({"category_id": 7}));
        let relation = make_relation("category_id", "BELONGS_TO");
        let edge = build_edge(&point, &relation);
        assert!(edge.is_some());
        assert_eq!(
            edge.expect("test: edge should be Some").source(),
            stable_point_id("99")
        );
    }

    #[test]
    fn test_build_edge_missing_fk_returns_none() {
        let point = make_point("1", serde_json::json!({}));
        let relation = make_relation("author_id", "AUTHORED_BY");
        assert!(build_edge(&point, &relation).is_none());
    }

    #[test]
    fn test_build_edge_string_fk_deterministic_id() {
        // GIVEN: same point and relation
        let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
        let relation = make_relation("author_id", "AUTHORED_BY");

        // WHEN: build_edge is called twice
        let e1 = build_edge(&point, &relation).expect("test: e1 should be Some");
        let e2 = build_edge(&point, &relation).expect("test: e2 should be Some");

        // THEN: both produce the same deterministic ID
        assert_eq!(e1.id(), e2.id(), "Edge IDs must be deterministic for the same input");
    }

    #[test]
    fn test_build_edge_different_labels_produce_different_ids() {
        // GIVEN: same point but different edge labels
        let point = make_point("doc-1", serde_json::json!({"author_id": "auth-42"}));
        let rel1 = make_relation("author_id", "AUTHORED_BY");
        let rel2 = make_relation("author_id", "EDITED_BY");

        // WHEN: build_edge is called with each relation
        let e1 = build_edge(&point, &rel1).expect("test: e1 should be Some");
        let e2 = build_edge(&point, &rel2).expect("test: e2 should be Some");

        // THEN: different labels produce different IDs
        assert_ne!(e1.id(), e2.id(), "Different edge labels must produce different IDs");
    }
}
