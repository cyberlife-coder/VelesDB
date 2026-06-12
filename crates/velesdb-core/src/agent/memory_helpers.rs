//! Shared helpers for agent memory subsystems (EPIC-010).
//!
//! Extracts common patterns used by `SemanticMemory`, `EpisodicMemory`, and
//! `ProceduralMemory` to avoid code duplication across the three modules.
//!
//! These helpers are ready for adoption by memory submodules.
//! Currently tested directly; callers will migrate in a follow-up.

use crate::collection::Collection;
use crate::{Database, DistanceMetric, Point};
use parking_lot::RwLock;
use std::collections::HashSet;

use super::error::AgentMemoryError;

/// Reserved payload key carrying the durable expiry timestamp (epoch seconds).
///
/// Written by the `*_with_ttl` store paths so a TTL survives a process
/// restart: the in-memory [`MemoryTtl`](super::ttl::MemoryTtl) map is just a
/// cache rebuilt from this field at subsystem construction. Payloads without
/// this key (or with a non-u64 value) have no TTL.
///
/// The key is namespaced (`_veles_` prefix, like the `_semantic_memory`
/// system collections) so a user metadata field named `expires_at` — a common
/// business field — is never interpreted as a TTL. User-facing metadata paths
/// (`SemanticMemory::store_with_metadata` / `update_metadata`) strip this
/// reserved key, mirroring how the `content` parameter owns `content`.
pub(super) const EXPIRES_AT_KEY: &str = "_veles_expires_at";

/// Looks up a `Collection` by name, returning an `AgentMemoryError` if absent.
pub(super) fn get_collection(db: &Database, name: &str) -> Result<Collection, AgentMemoryError> {
    db.get_vector_collection(name)
        .map(|vc| vc.inner)
        .or_else(|| db.get_graph_collection(name).map(|gc| gc.inner))
        .or_else(|| db.get_metadata_collection(name).map(|mc| mc.inner))
        .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))
}

/// Validates that `actual` matches the `expected` embedding dimension.
pub(super) fn validate_dimension(expected: usize, actual: usize) -> Result<(), AgentMemoryError> {
    if actual != expected {
        return Err(AgentMemoryError::DimensionMismatch { expected, actual });
    }
    Ok(())
}

/// Opens an existing collection or creates a new one with cosine distance.
///
/// If the collection already exists, verifies that dimensions match and returns
/// the existing dimension. If it does not exist, creates it with `dimension`.
pub(super) fn open_or_create_collection(
    db: &Database,
    collection_name: &str,
    dimension: usize,
) -> Result<usize, AgentMemoryError> {
    let actual_dimension = if let Some(collection) = db.get_vector_collection(collection_name) {
        let existing_dim = collection.config().dimension;
        if existing_dim != dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: existing_dim,
                actual: dimension,
            });
        }
        existing_dim
    } else {
        db.create_collection(collection_name, dimension, DistanceMetric::Cosine)?;
        dimension
    };
    Ok(actual_dimension)
}

/// Loads all IDs from an existing collection into a `HashSet`.
pub(super) fn load_stored_ids(db: &Database, collection_name: &str) -> HashSet<u64> {
    db.get_vector_collection(collection_name)
        .map(|c| c.all_ids().into_iter().collect())
        .unwrap_or_default()
}

/// Removes all existing points from a collection.
pub(super) fn clear_collection(collection: &Collection) -> Result<(), AgentMemoryError> {
    let existing_ids = collection.all_ids();
    if !existing_ids.is_empty() {
        collection
            .delete(&existing_ids)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
    }
    Ok(())
}

/// Clears and rebuilds `stored_ids` from a set of deserialized points.
pub(super) fn rebuild_stored_ids(stored_ids: &RwLock<HashSet<u64>>, points: &[Point]) {
    let mut ids = stored_ids.write();
    ids.clear();
    for point in points {
        ids.insert(point.id);
    }
}

/// Snapshot payload: memory points plus the relation edges between them.
///
/// Older snapshots were a bare `Vec<Point>` JSON array; `deserialize` keeps
/// reading those (no edges). New snapshots serialize this envelope so
/// relations survive a serialize → restore round-trip.
#[derive(serde::Serialize, serde::Deserialize)]
struct MemorySnapshot {
    points: Vec<Point>,
    #[serde(default)]
    edges: Vec<crate::collection::graph::GraphEdge>,
}

/// Serializes points (and the relation edges connecting them) from a
/// collection using the given ID set.
pub(super) fn serialize_points(
    collection: &Collection,
    ids: &[u64],
) -> Result<Vec<u8>, AgentMemoryError> {
    // get_raw: snapshots must include expired-but-not-yet-swept points so a
    // restore + auto_expire can still reclaim them (no storage leak).
    let points: Vec<_> = collection.get_raw(ids).into_iter().flatten().collect();
    let id_set: HashSet<u64> = points.iter().map(|p| p.id).collect();
    // Only edges whose BOTH endpoints are part of the snapshot are
    // meaningful after a restore.
    let edges: Vec<_> = collection
        .get_all_edges()
        .into_iter()
        .filter(|e| id_set.contains(&e.source()) && id_set.contains(&e.target()))
        .collect();
    serde_json::to_vec(&MemorySnapshot { points, edges })
        .map_err(|e| AgentMemoryError::IoError(e.to_string()))
}

/// Deserializes a snapshot and replaces the collection contents — points AND
/// relation edges (the clear's delete-cascade removes the previous edges).
///
/// Returns the deserialized points so callers can rebuild their own indexes.
pub(super) fn deserialize_into_collection(
    data: &[u8],
    collection: &Collection,
) -> Result<Option<Vec<Point>>, AgentMemoryError> {
    if data.is_empty() {
        return Ok(None);
    }

    let snapshot: MemorySnapshot = serde_json::from_slice(data)
        .or_else(|_| {
            // Backward compatibility: pre-graph snapshots are a bare array.
            serde_json::from_slice::<Vec<Point>>(data).map(|points| MemorySnapshot {
                points,
                edges: Vec::new(),
            })
        })
        .map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

    clear_collection(collection)?;
    upsert_points(collection, snapshot.points.clone())?;
    if !snapshot.edges.is_empty() {
        collection
            .add_edges_batch(snapshot.edges)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
    }

    Ok(Some(snapshot.points))
}

/// Deletes points by ID from a collection.
pub(super) fn delete_from_collection(
    collection: &Collection,
    ids: &[u64],
) -> Result<(), AgentMemoryError> {
    collection
        .delete(ids)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Upserts points into a collection.
pub(super) fn upsert_points(
    collection: &Collection,
    points: Vec<Point>,
) -> Result<(), AgentMemoryError> {
    collection
        .upsert(points)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Searches a collection by vector similarity.
pub(super) fn search_collection(
    collection: &Collection,
    query: &[f32],
    k: usize,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    collection
        .search(query, k)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Deletes a point by ID, removes it from the `stored_ids` tracking set, and
/// clears its TTL entry.
///
/// This is the common delete pattern shared by `SemanticMemory` and
/// `ProceduralMemory`. `EpisodicMemory` has additional temporal-index cleanup.
pub(super) fn delete_tracked_point(
    db: &Database,
    collection_name: &str,
    id: u64,
    stored_ids: &RwLock<HashSet<u64>>,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    delete_from_collection(&collection, &[id])?;
    stored_ids.write().remove(&id);
    ttl.remove(kind, id);
    Ok(())
}

/// Serializes all tracked points from a collection using the `stored_ids` set.
///
/// Shared by `SemanticMemory` and `ProceduralMemory`.
/// `EpisodicMemory` uses temporal-index IDs instead.
pub(super) fn serialize_tracked_points(
    db: &Database,
    collection_name: &str,
    stored_ids: &RwLock<HashSet<u64>>,
) -> Result<Vec<u8>, AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    let all_ids: Vec<u64> = stored_ids.read().iter().copied().collect();
    serialize_points(&collection, &all_ids)
}

/// Replaces collection contents from serialized bytes and rebuilds the
/// `stored_ids` tracking set.
///
/// Shared by `SemanticMemory` and `ProceduralMemory`.
/// `EpisodicMemory` rebuilds its temporal index instead.
pub(super) fn deserialize_tracked_points(
    db: &Database,
    collection_name: &str,
    data: &[u8],
    stored_ids: &RwLock<HashSet<u64>>,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    if let Some(points) = deserialize_into_collection(data, &collection)? {
        rebuild_stored_ids(stored_ids, &points);
    }
    Ok(())
}

/// Validates the query embedding, searches the collection, and filters out
/// expired results.
///
/// This is the common search preamble shared by `SemanticMemory::query`,
/// `EpisodicMemory::recall_similar`, and `ProceduralMemory::recall`. Each
/// caller then maps the returned `SearchResult` items into its own return
/// type.
pub(super) fn search_filtered(
    db: &Database,
    collection_name: &str,
    dimension: usize,
    query_embedding: &[f32],
    k: usize,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    validate_dimension(dimension, query_embedding.len())?;
    let collection = get_collection(db, collection_name)?;
    // Over-fetch so that expired-but-not-yet-deleted points evicted by the
    // post-search TTL filter do not shrink the result set below `k`. Expired
    // ids occupy at most `expired_count` of the top-k slots, so fetching
    // `k + expired_count` and then filtering guarantees up to `k` live results.
    // Scoped to `kind` so another subsystem's expired entries do not inflate
    // the over-fetch.
    let fetch_k = k.saturating_add(ttl.expired_count(kind));
    let results = search_collection(&collection, query_embedding, fetch_k)?;
    Ok(results
        .into_iter()
        .filter(|r| !ttl.is_expired(kind, r.point.id))
        .take(k)
        .collect())
}

/// Validates a count-prefixed binary buffer and returns the entry count.
///
/// The expected format is `[count: u64 LE][entries: count * entry_size bytes]`.
/// Returns `None` if the buffer is too small, the count cannot be read, or the
/// total length does not match the declared count.
///
/// Used by `TemporalIndex::deserialize` (16-byte entries) and
/// `MemoryTtl::deserialize` (25-byte entries).
#[allow(clippy::cast_possible_truncation)] // count validated against buffer length
pub(super) fn validate_binary_header(data: &[u8], entry_size: usize) -> Option<usize> {
    if data.len() < 8 {
        return None;
    }
    let count = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;
    let payload = data.len() - 8;
    // Untrusted `count`: bound it against the actual payload first so the
    // `count * entry_size` below cannot wrap on a 32-bit target and produce a
    // total that spuriously equals `data.len()`.
    if entry_size == 0 || count > payload / entry_size {
        return None;
    }
    let total = 8usize.checked_add(count.checked_mul(entry_size)?)?;
    if data.len() != total {
        return None;
    }
    Some(count)
}

/// Initializes the common fields shared by `SemanticMemory` and `ProceduralMemory`.
///
/// Opens or creates the backing collection, resolves the actual dimension,
/// and loads the set of stored point IDs. Returns a tuple of
/// `(collection_name, actual_dimension, stored_ids)` ready for struct
/// construction.
pub(super) fn init_tracked_memory(
    db: &Database,
    collection_name: &str,
    dimension: usize,
) -> Result<(String, usize, RwLock<HashSet<u64>>), AgentMemoryError> {
    let name = collection_name.to_string();
    let actual_dimension = open_or_create_collection(db, &name, dimension)?;
    let stored_ids = RwLock::new(load_stored_ids(db, &name));
    Ok((name, actual_dimension, stored_ids))
}

/// Validates an optional embedding dimension and returns a concrete vector.
///
/// If `embedding` is `Some`, validates that its length matches `dimension`
/// and returns a clone. If `None`, returns a zero-vector of the given
/// dimension. This is the common setup shared by `EpisodicMemory::record`
/// and `ProceduralMemory::learn`.
pub(super) fn resolve_embedding(
    dimension: usize,
    embedding: Option<&[f32]>,
) -> Result<Vec<f32>, AgentMemoryError> {
    if let Some(emb) = embedding {
        validate_dimension(dimension, emb.len())?;
    }
    Ok(embedding.map_or_else(|| vec![0.0; dimension], <[f32]>::to_vec))
}

/// Inserts the durable [`EXPIRES_AT_KEY`] into a JSON object payload.
///
/// No-op when `expires_at` is `None` or the payload is not a JSON object
/// (the `*_with_ttl` callers always build object payloads).
pub(super) fn attach_expiry(payload: &mut serde_json::Value, expires_at: Option<u64>) {
    if let (Some(expiry), Some(obj)) = (expires_at, payload.as_object_mut()) {
        obj.insert(EXPIRES_AT_KEY.to_string(), serde_json::Value::from(expiry));
    }
}

/// Verifies that a memory id refers to a live (non-expired, existing) point
/// and returns it.
///
/// Expired-but-not-yet-swept entries are invisible on every read surface;
/// write surfaces (TTL refresh, relate) must reject them the same way.
pub(super) fn ensure_live(
    collection: &Collection,
    collection_name: &str,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
    id: u64,
) -> Result<Point, AgentMemoryError> {
    if ttl.is_expired(kind, id) {
        return Err(AgentMemoryError::NotFound(format!(
            "memory id {id} is expired in {collection_name}"
        )));
    }
    get_point_or_not_found(collection, collection_name, id)
}

/// Seeds a relation edge-id counter past every existing edge id.
///
/// One pass over the edge-id registry (no edge cloning).
pub(super) fn seed_edge_counter(collection: &Collection) -> std::sync::atomic::AtomicU64 {
    let next = collection
        .max_edge_id()
        .map_or(1, |max| max.saturating_add(1));
    std::sync::atomic::AtomicU64::new(next)
}

/// Adds a relation edge between two memory points, allocating a fresh edge
/// id from `next_edge_id` (skipping ids taken by direct graph writes; the
/// `EdgeExists` retry covers the residual allocation race).
pub(super) fn add_relation_edge(
    collection: &Collection,
    next_edge_id: &std::sync::atomic::AtomicU64,
    endpoints: (u64, u64),
    rel_type: &str,
    properties: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<u64, AgentMemoryError> {
    let (from_id, to_id) = endpoints;
    loop {
        let edge_id = next_edge_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if collection.edge_exists(edge_id) {
            continue; // taken by a direct graph write — no junk WAL entry
        }
        let mut edge = crate::collection::graph::GraphEdge::new(edge_id, from_id, to_id, rel_type)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
        if let Some(props) = properties {
            let map: std::collections::HashMap<String, serde_json::Value> =
                props.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            edge = edge.with_properties(map);
        }
        match collection.add_edge(edge) {
            Ok(()) => return Ok(edge_id),
            // Lost the residual allocation race — try the next id.
            Err(crate::error::Error::EdgeExists(_)) => {}
            Err(e) => return Err(AgentMemoryError::CollectionError(e.to_string())),
        }
    }
}

/// Compensates the `relate()` check-then-add window: when an endpoint was
/// deleted between `ensure_live` and the edge write, the cascade may have
/// already run — remove the freshly written edge instead of leaving it
/// dangling forever.
pub(super) fn verify_relation_endpoints(
    collection: &Collection,
    edge_id: u64,
    endpoints: (u64, u64),
) -> Result<(), AgentMemoryError> {
    let (from_id, to_id) = endpoints;
    let alive = collection.get(&[from_id, to_id]);
    if alive.iter().flatten().count() == 2 {
        return Ok(());
    }
    let _ = collection.remove_edge(edge_id);
    Err(AgentMemoryError::NotFound(format!(
        "a relation endpoint ({from_id} or {to_id}) was deleted concurrently"
    )))
}

/// Retrieves a point by id, surfacing `NotFound` when it does not exist.
///
/// The `collection.get(&[id]) … next()` idiom shared by the agent
/// read-modify-write flows (`set_ttl_durable`, metadata updates).
pub(super) fn get_point_or_not_found(
    collection: &Collection,
    collection_name: &str,
    id: u64,
) -> Result<Point, AgentMemoryError> {
    collection
        .get(&[id])
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| {
            AgentMemoryError::NotFound(format!("memory id {id} not found in {collection_name}"))
        })
}

/// Durably sets (or refreshes) the TTL of an existing memory point.
///
/// Unlike `MemoryTtl::set_ttl` (in-memory map only, lost on restart), this
/// persists the expiry as the reserved [`EXPIRES_AT_KEY`] payload field via
/// an upsert and then updates the in-memory map, so the TTL survives a
/// restart (it is rebuilt by [`rebuild_ttl_from_payloads`]).
///
/// A `ttl_seconds` of 0 expires the entry immediately; its storage is
/// reclaimed by the next `auto_expire` sweep. Expired entries are invisible
/// on every read surface and cannot be resurrected here: refreshing an
/// already-expired id returns `NotFound`, mirroring `update_metadata`.
///
/// # Concurrency
///
/// This is a read-modify-write without a cross-call lock (the engine has no
/// transactions): a concurrent writer to the same id follows last-writer-wins
/// for the whole point, like the other agent update flows.
///
/// # Errors
///
/// Returns `NotFound` when no live point with `id` exists, or
/// `CollectionError` when the collection cannot be resolved, the stored
/// payload is not a JSON object (the expiry field cannot be attached), or
/// the upsert fails.
pub(super) fn set_ttl_durable(
    db: &Database,
    collection_name: &str,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
    id: u64,
    ttl_seconds: u64,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    // Expired-but-not-yet-swept entries are invisible on every read surface;
    // refreshing their TTL would silently resurrect them.
    let point = ensure_live(&collection, collection_name, ttl, kind, id)?;
    let expires_at = super::ttl::MemoryTtl::now().saturating_add(ttl_seconds);
    let mut payload = point
        .payload
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    attach_expiry(&mut payload, Some(expires_at));
    if payload.get(EXPIRES_AT_KEY).is_none() {
        // attach_expiry is a no-op on non-object payloads — failing loudly
        // beats returning Ok for a TTL that would silently vanish on restart.
        return Err(AgentMemoryError::CollectionError(format!(
            "memory id {id} in {collection_name} has a non-object payload; cannot persist TTL"
        )));
    }
    upsert_points(
        &collection,
        vec![Point {
            id,
            vector: point.vector,
            payload: Some(payload),
            sparse_vectors: point.sparse_vectors,
        }],
    )?;
    ttl.set_expiry(kind, id, expires_at);
    Ok(())
}

/// Rebuilds the in-memory TTL map for `kind` from persisted point payloads.
///
/// Called eagerly at subsystem construction (same pattern and cost class as
/// the episodic temporal-index rebuild) so that after a restart, `is_expired`,
/// `expired_count`, and `auto_expire` all see the durable TTLs again. Points
/// without an [`EXPIRES_AT_KEY`] payload field have no TTL.
///
/// # Errors
///
/// Returns an error when the collection cannot be resolved.
pub(super) fn rebuild_ttl_from_payloads(
    db: &Database,
    collection_name: &str,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    let all_ids = collection.all_ids();
    // get_raw: the filtered `get` hides expired-but-not-yet-swept points, so
    // using it here would drop their TTLs from the rebuilt map after a
    // restart and `auto_expire` would never reclaim them (storage leak).
    for point in collection.get_raw(&all_ids).into_iter().flatten() {
        let expiry = point
            .payload
            .as_ref()
            .and_then(|p| p.get(EXPIRES_AT_KEY))
            .and_then(serde_json::Value::as_u64);
        if let Some(expires_at) = expiry {
            ttl.set_expiry(kind, point.id, expires_at);
        }
    }
    Ok(())
}

/// Executes a `VelesQL` query string against a named collection.
///
/// Resolves the collection from the database by name, delegates to
/// `Collection::execute_query_str`, then filters out TTL-expired points so the
/// `VelesQL` bridge has the same read semantics as the native query APIs
/// (expired-but-not-yet-deleted entries are invisible on every read surface).
///
/// # Errors
///
/// Returns `AgentMemoryError::CollectionError` if the collection is not found,
/// or `AgentMemoryError::DatabaseError` if the query fails to parse or execute.
pub(super) fn execute_velesql(
    db: &Database,
    collection_name: &str,
    sql: &str,
    params: &std::collections::HashMap<String, serde_json::Value>,
    ttl: &super::ttl::MemoryTtl,
    kind: super::ttl::MemoryKind,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    let results = collection
        .execute_query_str(sql, params)
        .map_err(AgentMemoryError::DatabaseError)?;
    Ok(results
        .into_iter()
        .filter(|r| !ttl.is_expired(kind, r.point.id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_dimension ---

    #[test]
    fn validate_dimension_matching_returns_ok() {
        assert!(validate_dimension(128, 128).is_ok());
    }

    #[test]
    fn validate_dimension_zero_matches_zero() {
        assert!(validate_dimension(0, 0).is_ok());
    }

    #[test]
    fn validate_dimension_mismatch_returns_error() {
        let err = validate_dimension(128, 64).unwrap_err();
        assert!(
            matches!(
                err,
                AgentMemoryError::DimensionMismatch {
                    expected: 128,
                    actual: 64
                }
            ),
            "Expected DimensionMismatch, got: {err:?}"
        );
    }

    #[test]
    fn validate_dimension_swapped_values_are_distinct() {
        // validate_dimension(64, 128) should give expected=64, actual=128
        let err = validate_dimension(64, 128).unwrap_err();
        assert!(matches!(
            err,
            AgentMemoryError::DimensionMismatch {
                expected: 64,
                actual: 128
            }
        ));
    }

    // --- rebuild_stored_ids ---

    #[test]
    fn rebuild_stored_ids_populates_from_points() {
        let stored_ids = RwLock::new(HashSet::new());
        let points = vec![
            Point::without_payload(10, vec![0.0; 4]),
            Point::without_payload(20, vec![0.0; 4]),
            Point::without_payload(30, vec![0.0; 4]),
        ];

        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert!(ids.contains(&30));
    }

    #[test]
    fn rebuild_stored_ids_clears_previous_ids() {
        let mut initial = HashSet::new();
        initial.insert(1);
        initial.insert(2);
        let stored_ids = RwLock::new(initial);

        let points = vec![Point::without_payload(99, vec![0.0; 4])];
        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&99));
        assert!(!ids.contains(&1));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn rebuild_stored_ids_empty_points_clears_all() {
        let mut initial = HashSet::new();
        initial.insert(5);
        let stored_ids = RwLock::new(initial);

        rebuild_stored_ids(&stored_ids, &[]);

        assert!(stored_ids.read().is_empty());
    }

    #[test]
    fn rebuild_stored_ids_deduplicates() {
        let stored_ids = RwLock::new(HashSet::new());
        let points = vec![
            Point::without_payload(1, vec![0.0; 4]),
            Point::without_payload(1, vec![1.0; 4]), // same ID
        ];

        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&1));
    }

    // --- open_or_create_collection (requires persistence + tempdir) ---

    #[cfg(feature = "persistence")]
    mod persistence_tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn open_or_create_creates_new_collection() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            let dim = open_or_create_collection(&db, "test_coll", 64).unwrap();
            assert_eq!(dim, 64);

            // Collection should now be retrievable.
            assert!(db.get_vector_collection("test_coll").is_some());
        }

        #[test]
        fn open_or_create_returns_existing_with_matching_dim() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            // First call creates.
            open_or_create_collection(&db, "my_coll", 128).unwrap();

            // Second call with same dim should succeed.
            let dim = open_or_create_collection(&db, "my_coll", 128).unwrap();
            assert_eq!(dim, 128);
        }

        #[test]
        fn open_or_create_errors_on_dimension_mismatch() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            open_or_create_collection(&db, "dim_coll", 64).unwrap();

            let err = open_or_create_collection(&db, "dim_coll", 128).unwrap_err();
            assert!(
                matches!(
                    err,
                    AgentMemoryError::DimensionMismatch {
                        expected: 64,
                        actual: 128
                    }
                ),
                "Expected DimensionMismatch, got: {err:?}"
            );
        }
    }
}
