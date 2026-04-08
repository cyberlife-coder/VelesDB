//! Introspection executor for `VelesQL` v3.4.
//!
//! Handles SHOW COLLECTIONS, DESCRIBE COLLECTION, and EXPLAIN by
//! delegating to existing [`Database`] APIs.

use crate::velesql::IntrospectionStatement;
use crate::{Error, Result, SearchResult};

use super::Database;

impl Database {
    /// Dispatches an introspection statement to the appropriate executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the target collection does not exist (DESCRIBE)
    /// or if the inner query is invalid (EXPLAIN).
    pub(super) fn execute_introspection(
        &self,
        stmt: &IntrospectionStatement,
    ) -> Result<Vec<SearchResult>> {
        match stmt {
            IntrospectionStatement::ShowCollections => self.execute_show_collections(),
            IntrospectionStatement::DescribeCollection(desc) => {
                self.execute_describe_collection(&desc.name)
            }
            IntrospectionStatement::Explain(inner_query) => {
                self.execute_explain_introspection(inner_query)
            }
        }
    }

    /// Executes `SHOW COLLECTIONS` -- lists all collections with their types.
    ///
    /// Returns one `SearchResult` per collection, with payload containing
    /// `name` and `type` fields.
    ///
    /// Returns `Result` for `execute_introspection()` uniformity even though
    /// this particular variant never fails.
    #[allow(clippy::unnecessary_wraps)]
    fn execute_show_collections(&self) -> Result<Vec<SearchResult>> {
        let names = self.list_collections();
        let results = names
            .into_iter()
            .enumerate()
            .map(|(idx, name)| {
                let coll_type = self.resolve_collection_type(&name).unwrap_or("unknown");
                build_show_result(idx, &name, coll_type)
            })
            .collect();
        Ok(results)
    }

    /// Determines the type string for a named collection.
    ///
    /// Returns `None` if the collection does not exist in any registry.
    /// Checks registries in order: graph → metadata → vector.
    fn resolve_collection_type(&self, name: &str) -> Option<&'static str> {
        if self.get_graph_collection(name).is_some() {
            return Some("graph");
        }
        if self.get_metadata_collection(name).is_some() {
            return Some("metadata");
        }
        if self.get_vector_collection(name).is_some() {
            return Some("vector");
        }
        None
    }

    /// Executes `DESCRIBE COLLECTION <name>` -- returns collection metadata.
    ///
    /// # Errors
    ///
    /// Returns `CollectionNotFound` if the collection does not exist.
    fn execute_describe_collection(&self, name: &str) -> Result<Vec<SearchResult>> {
        let coll_type = self
            .resolve_collection_type(name)
            .ok_or_else(|| Error::CollectionNotFound(name.to_string()))?;

        let payload = build_describe_payload(self, name, coll_type);
        let result = SearchResult::new(crate::Point::metadata_only(0, payload), 0.0);
        Ok(vec![result])
    }

    /// Executes `EXPLAIN <query>` -- returns the query plan as a result.
    ///
    /// # Errors
    ///
    /// Returns an error if the inner query is invalid.
    fn execute_explain_introspection(
        &self,
        inner_query: &crate::velesql::Query,
    ) -> Result<Vec<SearchResult>> {
        let plan = self.explain_query(inner_query)?;
        let plan_json = serde_json::to_value(&plan)
            .unwrap_or_else(|_| serde_json::json!({"error": "failed to serialize plan"}));

        let mut payload = serde_json::Map::new();
        payload.insert("plan".to_string(), plan_json);
        payload.insert(
            "tree".to_string(),
            serde_json::Value::String(plan.to_tree()),
        );

        let result = SearchResult::new(
            crate::Point::metadata_only(0, serde_json::Value::Object(payload)),
            0.0,
        );
        Ok(vec![result])
    }
}

/// Builds a `SearchResult` for a single collection in SHOW COLLECTIONS output.
fn build_show_result(idx: usize, name: &str, coll_type: &str) -> SearchResult {
    let mut payload = serde_json::Map::new();
    payload.insert("name".to_string(), serde_json::json!(name));
    payload.insert("type".to_string(), serde_json::json!(coll_type));

    #[allow(clippy::cast_possible_truncation)]
    // SAFETY: collection count will never exceed u64::MAX in practice.
    let id = idx as u64;

    SearchResult::new(
        crate::Point::metadata_only(id, serde_json::Value::Object(payload)),
        0.0,
    )
}

/// Builds the describe payload for a collection, including type-specific metadata.
fn build_describe_payload(db: &Database, name: &str, coll_type: &str) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert("name".to_string(), serde_json::json!(name));
    payload.insert("type".to_string(), serde_json::json!(coll_type));

    add_type_specific_metadata(db, name, coll_type, &mut payload);

    serde_json::Value::Object(payload)
}

/// Adds type-specific fields (dimension, metric, `point_count`) to the payload.
fn add_type_specific_metadata(
    db: &Database,
    name: &str,
    coll_type: &str,
    payload: &mut serde_json::Map<String, serde_json::Value>,
) {
    match coll_type {
        "vector" => add_vector_metadata(db, name, payload),
        "graph" => add_graph_metadata(db, name, payload),
        "metadata" => add_metadata_metadata(db, name, payload),
        _ => {}
    }
}

/// Adds vector collection metadata (dimension, metric, `point_count`).
fn add_vector_metadata(
    db: &Database,
    name: &str,
    payload: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let Some(vc) = db.get_vector_collection(name) {
        payload.insert("dimension".to_string(), serde_json::json!(vc.dimension()));
        payload.insert(
            "metric".to_string(),
            serde_json::json!(format!("{:?}", vc.metric())),
        );
        payload.insert("point_count".to_string(), serde_json::json!(vc.len()));
    }
}

/// Adds graph collection metadata (`point_count`).
fn add_graph_metadata(
    db: &Database,
    name: &str,
    payload: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let Some(gc) = db.get_graph_collection(name) {
        payload.insert("point_count".to_string(), serde_json::json!(gc.len()));
    }
}

/// Adds metadata collection metadata (`point_count`).
fn add_metadata_metadata(
    db: &Database,
    name: &str,
    payload: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let Some(mc) = db.get_metadata_collection(name) {
        payload.insert("point_count".to_string(), serde_json::json!(mc.len()));
    }
}
