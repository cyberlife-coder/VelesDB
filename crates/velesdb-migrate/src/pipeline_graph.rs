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

    /// Closes the underlying source connector, releasing any held resources.
    ///
    /// # Errors
    ///
    /// Returns an error if the connector close fails.
    pub async fn close(&mut self) -> Result<()> {
        self.connector.close().await
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
    pub async fn run(&self, db: &velesdb_core::Database) -> Result<GraphMigrationStats> {
        let graph_name = self
            .config
            .destination
            .graph_collection
            .as_deref()
            .ok_or_else(|| Error::Config("graph_collection not configured".to_string()))?;

        ensure_graph_collection_exists(db, graph_name)?;

        let gc = db.get_graph_collection(graph_name).ok_or_else(|| {
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
                relation.from_column, relation.to_table, relation.to_column, relation.edge_label
            );

            let (created, failed) = self
                .migrate_relation_edges(relation, batch_size, &gc)
                .await?;
            stats.edges_created += created;
            stats.edges_failed += failed;
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

    /// Streams a relation's edges into the graph collection batch by batch.
    ///
    /// Each source batch is converted to edges and inserted immediately, so
    /// only one batch worth of edges is held in memory at a time. The previous
    /// implementation accumulated every edge of a relation into a single `Vec`
    /// before one bulk insert, which could exhaust memory on large sources.
    ///
    /// Returns `(edges_created, edges_failed)`.
    async fn migrate_relation_edges(
        &self,
        relation: &RelationConfig,
        batch_size: usize,
        gc: &velesdb_core::GraphCollection,
    ) -> Result<(u64, u64)> {
        let mut created = 0u64;
        let mut failed = 0u64;
        let mut offset = None;
        // add_edges_batch now requires every edge endpoint to have a stored
        // node payload (#1442). This graph collection is a fresh, edge-only
        // index — the real point data lives in the vector destination
        // collection — so every endpoint needs a minimal stub payload
        // seeded here. Tracks ids already stubbed across the whole
        // relation so a node referenced by many edges (e.g. a popular FK
        // target) is not re-written on every recurrence.
        let mut seeded_nodes: std::collections::HashSet<u64> = std::collections::HashSet::new();

        loop {
            let batch = self
                .connector
                .extract_batch(offset.clone(), batch_size)
                .await?;

            // Guard against connectors that return has_more=true with an empty batch
            // (e.g. cursor-based connectors on transient gaps), which would otherwise
            // cause an infinite loop restarting from offset=None.
            if batch.points.is_empty() {
                break;
            }

            let mut edges = Vec::with_capacity(batch.points.len());
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

            if !edges.is_empty() {
                seed_edge_endpoints(&edges, &mut seeded_nodes, |id| {
                    gc.upsert_node_payload(id, &serde_json::json!({}))
                });

                let total = edges.len() as u64;
                match gc.add_edges_batch(edges.clone()) {
                    Ok(inserted) => {
                        created += inserted as u64;
                        failed += total.saturating_sub(inserted as u64);
                    }
                    Err(e) => {
                        // add_edges_batch validates the whole batch before any
                        // write, so one genuinely bad edge (e.g. node-seeding
                        // above failed for one id) fails the entire batch.
                        // Fall back to per-edge inserts so that degrades to a
                        // counted failure instead of aborting the migration.
                        warn!("Edge batch insert failed ({e}); retrying edges individually");
                        for edge in edges {
                            match gc.add_edge(edge) {
                                Ok(()) => created += 1,
                                Err(e) => {
                                    failed += 1;
                                    debug!("Edge insert failed: {e}");
                                }
                            }
                        }
                    }
                }
            }

            if !batch.has_more {
                break;
            }
            offset = batch.next_offset;
        }

        Ok((created, failed))
    }
}

fn ensure_graph_collection_exists(db: &velesdb_core::Database, name: &str) -> Result<()> {
    if db.get_graph_collection(name).is_some() {
        return Ok(());
    }
    db.create_graph_collection(name, velesdb_core::GraphSchema::schemaless())
        .map_err(|e| Error::DestinationConnection(format!("Cannot create graph collection: {e}")))
}

fn build_edge(
    point: &crate::connectors::ExtractedPoint,
    relation: &RelationConfig,
) -> Option<velesdb_core::GraphEdge> {
    let from_node_id = stable_point_id(&point.id);

    let fk_value = point.payload.get(&relation.from_column)?;
    let fk_str = value_to_id_str(fk_value)?;
    let to_node_id = stable_point_id(&fk_str);

    // Delegate to core's canonical edge-id derivation (FNV-1a over the raw
    // LE bytes of source, target, label) so the same logical edge gets the
    // same id as every other VelesDB engine.
    let id = velesdb_core::hash_edge_id(from_node_id, to_node_id, &relation.edge_label);

    let edge = velesdb_core::GraphEdge::new(id, from_node_id, to_node_id, &relation.edge_label)
        .inspect_err(|e| warn!("Failed to create edge {}: {}", id, e))
        .ok()?;

    Some(attach_weight(edge, point, relation))
}

fn attach_weight(
    edge: velesdb_core::GraphEdge,
    point: &crate::connectors::ExtractedPoint,
    relation: &RelationConfig,
) -> velesdb_core::GraphEdge {
    let Some(weight_col) = &relation.weight_column else {
        return edge;
    };
    let Some(weight) = point.payload.get(weight_col).and_then(|v| v.as_f64()) else {
        return edge;
    };
    edge.with_properties(std::collections::HashMap::from([(
        "weight".to_string(),
        serde_json::json!(weight),
    )]))
}

/// Seeds a stub node payload for each not-yet-seeded edge endpoint.
///
/// Extracted from `migrate_relation_edges` so the seed/retry bookkeeping is
/// unit-testable with an injectable `upsert` closure (real usage passes
/// `GraphCollection::upsert_node_payload`; tests can simulate a failure for a
/// specific id). `seeded_nodes` is shared across the whole relation so a node
/// referenced by many edges (e.g. a popular FK target) is not re-written on
/// every recurrence.
fn seed_edge_endpoints<F, E>(
    edges: &[velesdb_core::GraphEdge],
    seeded_nodes: &mut std::collections::HashSet<u64>,
    mut upsert: F,
) where
    F: FnMut(u64) -> std::result::Result<(), E>,
    E: std::fmt::Display,
{
    for edge in edges {
        for id in [edge.source(), edge.target()] {
            if seeded_nodes.insert(id) {
                if let Err(e) = upsert(id) {
                    warn!("Failed to seed graph node {id}: {e}");
                }
            }
        }
    }
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

    fn make_weighted_relation(from: &str, label: &str, weight_col: &str) -> RelationConfig {
        RelationConfig {
            weight_column: Some(weight_col.to_string()),
            ..make_relation(from, label)
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
        assert_eq!(
            e1.id(),
            e2.id(),
            "Edge IDs must be deterministic for the same input"
        );
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
        assert_ne!(
            e1.id(),
            e2.id(),
            "Different edge labels must produce different IDs"
        );
    }

    #[test]
    fn test_build_edge_attaches_numeric_weight_property() {
        // GIVEN: a relation with a weight_column whose value is numeric
        let point = make_point(
            "doc-1",
            serde_json::json!({"author_id": "auth-42", "score": 0.75}),
        );
        let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

        // WHEN: build_edge runs the weighted branch of attach_weight
        let edge = build_edge(&point, &relation).expect("test: edge should be Some");

        // THEN: the weight is attached as an edge property
        let weight = edge
            .property("weight")
            .expect("test: weight property should be present");
        assert_eq!(weight, &serde_json::json!(0.75));
    }

    #[test]
    fn test_build_edge_attaches_integer_weight_as_f64() {
        // GIVEN: a relation whose weight column holds a JSON integer
        let point = make_point("doc-9", serde_json::json!({"ref_id": "tgt-9", "rank": 3}));
        let relation = make_weighted_relation("ref_id", "LINKS_TO", "rank");

        // WHEN: build_edge runs (integer is coerced via as_f64)
        let edge = build_edge(&point, &relation).expect("test: edge should be Some");

        // THEN: the integer weight is stored as a float
        let weight = edge
            .property("weight")
            .expect("test: weight property should be present");
        assert_eq!(
            weight.as_f64().expect("test: weight should be numeric"),
            3.0
        );
    }

    #[test]
    fn test_build_edge_weight_column_missing_value_skips_property() {
        // GIVEN: a weight_column configured but absent from the point payload
        let point = make_point("doc-2", serde_json::json!({"author_id": "auth-7"}));
        let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

        // WHEN: build_edge runs the second early-return branch of attach_weight
        let edge = build_edge(&point, &relation).expect("test: edge should be Some");

        // THEN: no weight property is attached and properties stay empty
        assert!(edge.property("weight").is_none());
        assert!(edge.properties().is_empty());
    }

    #[test]
    fn test_build_edge_weight_non_numeric_skips_property() {
        // GIVEN: a weight_column present but holding a non-numeric (string) value
        let point = make_point(
            "doc-3",
            serde_json::json!({"author_id": "auth-8", "score": "high"}),
        );
        let relation = make_weighted_relation("author_id", "AUTHORED_BY", "score");

        // WHEN: build_edge runs (as_f64 returns None for a string)
        let edge = build_edge(&point, &relation).expect("test: edge should be Some");

        // THEN: the non-numeric value is ignored, no weight property attached
        assert!(edge.property("weight").is_none());
        assert!(edge.properties().is_empty());
    }

    #[test]
    fn seed_retries_failed_node_on_next_occurrence() {
        // Regression guard: a node whose stub-seed upsert fails must NOT be
        // marked as seeded, so a later edge referencing the same node id
        // retries the upsert instead of silently skipping it forever (which
        // would leave that node's edges permanently rejected by #1442's
        // add_edges_batch validation, since the endpoint was never actually
        // stored).
        let attempts = std::cell::RefCell::new(Vec::new());
        let mut seeded = std::collections::HashSet::new();

        let edge1 = velesdb_core::GraphEdge::new(1, 100, 200, "REL").expect("valid edge");
        let edge2 = velesdb_core::GraphEdge::new(2, 100, 300, "REL").expect("valid edge");

        // First occurrence of node 100: its upsert fails.
        seed_edge_endpoints(std::slice::from_ref(&edge1), &mut seeded, |id| {
            attempts.borrow_mut().push(id);
            if id == 100 {
                Err("simulated storage failure")
            } else {
                Ok(())
            }
        });
        assert!(
            !seeded.contains(&100),
            "a failed upsert must not mark the node as seeded"
        );
        assert!(seeded.contains(&200), "the succeeding endpoint is seeded");

        // Second occurrence of node 100 (via edge2): must retry since it was
        // never marked seeded.
        seed_edge_endpoints(std::slice::from_ref(&edge2), &mut seeded, |id| {
            attempts.borrow_mut().push(id);
            Ok::<(), &str>(())
        });
        assert!(
            seeded.contains(&100),
            "node 100 must be seeded after a successful retry"
        );
        assert_eq!(
            *attempts.borrow(),
            vec![100, 200, 100, 300],
            "node 100's upsert must be attempted again on its next occurrence"
        );
    }
}
