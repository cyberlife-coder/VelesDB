//! Storage backend abstraction for [`crate::service::MemoryService`].
//!
//! The wedge orchestration (remember/recall/relate/forget/why/fusion) is
//! written once, generic over [`MemoryStore`], so it runs unchanged over any
//! backend: the native, file-backed [`NativeStore`] (the default — nothing
//! changes for existing callers), or an in-memory backend such as the one
//! `velesdb-wasm` provides for the browser (no filesystem, no `persistence`
//! feature).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use velesdb_core::agent::AgentMemory;
use velesdb_core::{Database, SearchResult};

use crate::error::MemoryError;
use crate::model::{ColumnFilter, MemoryEdge, Recollection};
use crate::service::Metadata;

/// The storage primitives [`crate::service::MemoryService`] needs: write,
/// vector search, graph edges, and by-id lookup. A backend that implements
/// this trait can run the full wedge (`remember`/`recall`/`recall_fused`/
/// `relate`/`forget`/`why`/`remember_extracted`) with no orchestration code
/// duplicated.
pub trait MemoryStore {
    /// Store a fact with no metadata or expiry.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError>;

    /// Store a fact tagged with `metadata`, no expiry.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError>;

    /// Store a fact that expires after `ttl_seconds`, no metadata.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError>;

    /// Merge `metadata` into an already-stored fact's payload, preserving any
    /// durable TTL. Used to combine metadata with an expiry (store both in
    /// two calls rather than needing every metadata×TTL combination as a
    /// separate primitive).
    ///
    /// # Errors
    /// Returns [`MemoryError`] if `id` is unknown or persistence fails.
    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError>;

    /// A fact's content and embedding, or `None` if unknown/expired.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError>;

    /// A fact's raw stored payload — reserved system keys (`_veles_*`)
    /// included, so the service layer can check the hub flag before
    /// stripping them for the caller — or `None` when the fact is
    /// unknown/expired.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError>;

    /// Batched [`Self::get_metadata`]: one storage round trip for every id
    /// in `ids`, results in the same order and length (an unknown or expired
    /// id maps to `None`). Same raw-payload semantics as the single-id form.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError>;

    /// Delete a fact.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if deletion fails.
    fn delete(&self, id: u64) -> Result<(), MemoryError>;

    /// Vector search for up to `k` ids, narrowed to facts whose metadata
    /// exactly matches every key in `filter`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the query fails.
    fn query_filtered(
        &self,
        embedding: &[f32],
        k: usize,
        filter: &Metadata,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError>;

    /// Vector search for up to `k` ids, dropping facts whose metadata matches
    /// every key in `exclude`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the query fails.
    fn query_excluding(
        &self,
        embedding: &[f32],
        k: usize,
        exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError>;

    /// Vector search fused with structured `ColumnStore` predicates (ranges
    /// and comparisons, not just equality) — the engine behind
    /// [`crate::service::MemoryService::recall_where`].
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidFilter`] if a filter field is not a
    /// plain identifier or a filter value is non-scalar, or [`MemoryError`]
    /// if the query fails.
    fn query_columnar(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError>;

    /// Create a typed edge `from -> to`. Returns the edge id.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if either endpoint is missing or persistence fails.
    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError>;

    /// The outgoing edges of `id`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError>;

    /// The total number of live (non-expired) tracked facts, including
    /// internal entity hubs — used as a corpus-size proxy for idf weighting.
    fn count(&self) -> usize;
}

/// The default [`MemoryStore`]: the native, file-backed engine
/// (`velesdb-core`'s `Database`/`AgentMemory`, requiring the `persistence`
/// feature). Existing callers of `MemoryService::open` see no change — this
/// is exactly what they already ran.
pub struct NativeStore {
    memory: AgentMemory,
}

impl NativeStore {
    /// Open (or create) a native store at `path`, sized for `dimension`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the store cannot be opened.
    pub fn open<P: AsRef<Path>>(path: P, dimension: usize) -> Result<Self, MemoryError> {
        let db = Arc::new(Database::open(path)?);
        let memory = AgentMemory::with_dimension(db, dimension)?;
        Ok(Self { memory })
    }
}

impl MemoryStore for NativeStore {
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError> {
        self.memory
            .semantic()
            .store(id, content, embedding)
            .map_err(MemoryError::from)
    }

    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError> {
        self.memory
            .semantic()
            .store_with_metadata(id, content, embedding, metadata)
            .map_err(MemoryError::from)
    }

    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.memory
            .semantic()
            .store_with_ttl(id, content, embedding, ttl_seconds)
            .map_err(MemoryError::from)
    }

    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
        self.memory
            .semantic()
            .update_metadata(id, metadata)
            .map_err(MemoryError::from)
    }

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        self.memory.semantic().get(id).map_err(MemoryError::from)
    }

    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
        self.memory
            .semantic()
            .get_metadata(id)
            .map_err(MemoryError::from)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.memory
            .semantic()
            .get_metadata_batch(ids)
            .map_err(MemoryError::from)
    }

    fn delete(&self, id: u64) -> Result<(), MemoryError> {
        self.memory.semantic().delete(id).map_err(MemoryError::from)
    }

    fn query_filtered(
        &self,
        embedding: &[f32],
        k: usize,
        filter: &Metadata,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.memory
            .semantic()
            .query_filtered(embedding, k, filter, offset)
            .map_err(MemoryError::from)
    }

    fn query_excluding(
        &self,
        embedding: &[f32],
        k: usize,
        exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.memory
            .semantic()
            .query_excluding(embedding, k, exclude)
            .map_err(MemoryError::from)
    }

    fn query_columnar(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError> {
        let (sql, params) = self.build_fused_query(embedding, k, filters)?;
        // Field names are validated by `build_fused_query`; ensure each one is
        // indexed so the planner uses a bitmap prefilter instead of an O(n)
        // post-filter scan. Idempotent and incrementally maintained thereafter.
        for filter in filters {
            self.memory
                .semantic()
                .ensure_index(&filter.field)
                .map_err(MemoryError::from)?;
        }
        let results = self
            .memory
            .query_semantic(&sql, &params)
            .map_err(MemoryError::from)?;
        Ok(results.iter().map(to_recollection).collect())
    }

    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        self.memory
            .semantic()
            .relate(from, to, relation, None)
            .map_err(MemoryError::from)
    }

    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        Ok(self
            .memory
            .semantic()
            .relations(id)?
            .into_iter()
            .map(|edge| MemoryEdge {
                from: edge.source(),
                to: edge.target(),
                relation: edge.label().to_owned(),
            })
            .collect())
    }

    fn count(&self) -> usize {
        self.memory.semantic().count()
    }
}

impl NativeStore {
    /// Build the `VelesQL` for [`Self::query_columnar`]: a `NEAR` predicate
    /// plus one bound parameter per filter, against the semantic collection.
    /// Filter *values* are bound as query parameters (never interpolated);
    /// filter *field names* are validated to be plain identifiers.
    fn build_fused_query(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<(String, HashMap<String, Value>), MemoryError> {
        use std::fmt::Write as _;
        let mut params: HashMap<String, Value> = HashMap::new();
        params.insert("q".to_string(), json!(embedding));
        let mut predicate = String::from("vector NEAR $q");
        for (index, filter) in filters.iter().enumerate() {
            validate_field(&filter.field)?;
            validate_scalar(&filter.value)?;
            let key = format!("p{index}");
            let _ = write!(
                predicate,
                " AND {} {} ${key}",
                filter.field,
                filter.op.as_sql()
            );
            params.insert(key, filter.value.clone());
        }
        let sql = format!(
            "SELECT * FROM {} WHERE {predicate} LIMIT {k}",
            self.memory.semantic().collection_name()
        );
        Ok((sql, params))
    }
}

/// True for metadata keys the memory layer reserves: the engine's `content`
/// payload, and any `_veles_`-namespaced system key (durable TTL, entity
/// hubs). Mirrors [`crate::service`]'s own copy — kept private to each module
/// since both need it and neither should import a "utility" from the other.
fn is_reserved_key(key: &str) -> bool {
    key == "content" || key.starts_with("_veles_")
}

/// Map a core search result to a [`Recollection`], lifting the fact text out
/// of the reserved `content` payload key and surfacing any remaining
/// caller-supplied metadata (reserved system keys excluded).
fn to_recollection(result: &SearchResult) -> Recollection {
    let payload = result.point.payload.as_ref().and_then(Value::as_object);
    let content = payload
        .and_then(|payload| payload.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let metadata = payload.and_then(|payload| {
        let metadata: Map<String, Value> = payload
            .iter()
            .filter(|(key, _)| !is_reserved_key(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        (!metadata.is_empty()).then_some(metadata)
    });
    Recollection {
        id: result.point.id,
        score: result.score,
        content,
        metadata,
    }
}

/// Accept only plain, non-reserved identifier field names, so a filter field
/// can be safely placed into the query text (its value is always a bound
/// parameter). Rejects the reserved system columns the docs promise are off
/// limits: `content` (the fact payload) and any `_veles_`-prefixed engine key
/// (e.g. durable TTL).
fn validate_field(field: &str) -> Result<(), MemoryError> {
    let plain = !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let reserved = field == "content" || field.starts_with("_veles_");
    if plain && !reserved {
        Ok(())
    } else {
        Err(MemoryError::InvalidFilter(field.to_owned()))
    }
}

/// Reject non-scalar filter values. Only strings, numbers, and booleans can be
/// compared against a `ColumnStore` column; binding an array/object/null would
/// fail deep in the query engine and surface as an opaque internal error instead
/// of a clear client-input error.
fn validate_scalar(value: &Value) -> Result<(), MemoryError> {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
        _ => Err(MemoryError::InvalidFilter(format!(
            "value must be a string, number, or boolean, got {value}"
        ))),
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
