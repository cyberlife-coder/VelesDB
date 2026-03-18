//! Graph collection creation and retrieval operations.

use crate::collection::GraphCollection;
use crate::{CollectionType, DistanceMetric, Result};

use super::Database;

#[allow(deprecated)]
impl Database {
    /// Creates a new graph collection.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    #[allow(clippy::needless_pass_by_value)] // Public API — changing to &ref would be breaking.
    pub fn create_graph_collection(
        &self,
        name: &str,
        schema: crate::collection::GraphSchema,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll =
            GraphCollection::create(path, name, None, DistanceMetric::Cosine, schema.clone())?;
        self.register_graph_collection(name, &coll, None, DistanceMetric::Cosine, &schema);
        Ok(())
    }

    /// Creates a new graph collection with node embeddings.
    ///
    /// Unlike [`create_graph_collection`](Self::create_graph_collection), this
    /// variant configures a vector dimension and distance metric so that nodes
    /// can store embeddings and support similarity search.
    ///
    /// # Errors
    ///
    /// Returns an error if a collection with the same name already exists.
    #[allow(clippy::needless_pass_by_value)] // Public API — changing to &ref would be breaking.
    pub fn create_graph_collection_with_embeddings(
        &self,
        name: &str,
        schema: crate::collection::GraphSchema,
        dimension: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = GraphCollection::create(path, name, Some(dimension), metric, schema.clone())?;
        self.register_graph_collection(name, &coll, Some(dimension), metric, &schema);
        Ok(())
    }

    /// Internal helper for `create_collection_typed` with `Graph` variant.
    pub(super) fn create_graph_collection_from_type(
        &self,
        name: &str,
        dimension: Option<usize>,
        metric: DistanceMetric,
        schema: &crate::collection::GraphSchema,
    ) -> Result<()> {
        self.ensure_collection_name_available(name)?;
        let path = self.data_dir.join(name);
        let coll = GraphCollection::create(path, name, dimension, metric, schema.clone())?;
        self.register_graph_collection(name, &coll, dimension, metric, schema);
        Ok(())
    }

    /// Registers a graph collection in both legacy and typed registries,
    /// notifies the observer, and bumps the schema version.
    fn register_graph_collection(
        &self,
        name: &str,
        coll: &GraphCollection,
        dimension: Option<usize>,
        metric: DistanceMetric,
        schema: &crate::collection::GraphSchema,
    ) {
        self.collections
            .write()
            .insert(name.to_string(), coll.inner.clone());
        self.graph_colls
            .write()
            .insert(name.to_string(), coll.clone());

        if let Some(ref obs) = self.observer {
            let kind = CollectionType::Graph {
                dimension,
                metric,
                schema: schema.clone(),
            };
            obs.on_collection_created(name, &kind);
        }

        self.schema_version
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns a `GraphCollection` by name.
    ///
    /// Checks the typed registry first.  Falls back to opening from disk if the
    /// collection was not registered in-memory (e.g. after a restart or when
    /// the collection was auto-created by a graph handler).  The instance is
    /// cached into the registry so subsequent calls are free.
    ///
    /// Returns `None` if the collection does not exist on disk.
    #[must_use]
    pub fn get_graph_collection(&self, name: &str) -> Option<GraphCollection> {
        if let Some(c) = self.graph_colls.read().get(name).cloned() {
            return Some(c);
        }
        self.open_graph_collection_from_disk(name)
    }

    /// Disk fallback for `get_graph_collection`.
    fn open_graph_collection_from_disk(&self, name: &str) -> Option<GraphCollection> {
        let cfg = self.read_collection_config(name)?;
        cfg.graph_schema.as_ref()?;
        let coll = GraphCollection::open(self.data_dir.join(name)).ok()?;
        self.graph_colls
            .write()
            .insert(name.to_string(), coll.clone());
        Some(coll)
    }
}
