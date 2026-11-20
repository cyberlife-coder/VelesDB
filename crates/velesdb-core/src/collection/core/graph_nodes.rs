//! Node reads for graph collections: every stored node, whether it carries
//! an embedding, a payload, or both.
//!
//! A graph node is stored as a payload (`store_node_payload`), as a vector
//! (an embedding upserted without a payload), or as both. The node set is
//! therefore the union of the two stores, which
//! [`Collection::all_point_ids`] lists and [`Collection::len`] counts
//! (`RowUniverse::StoredNodes`, `size.rs`), and
//! [`Collection::get`] on a graph reads its rows through
//! [`Collection::get_stored_nodes`].

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::types::Collection;
use crate::point::Point;
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Retrieves stored nodes by id: a node with a vector, a payload or both
    /// is returned with what it stores — an empty `vector` when it has no
    /// embedding — and an id naming nothing reads `None`.
    ///
    /// TTL-expired nodes read `None`, as [`get`](Self::get) hides them.
    pub(crate) fn get_stored_nodes(&self, ids: &[u64]) -> Vec<Option<Point>> {
        let now_secs = now_unix_secs();
        // LOCK ORDER: vector_storage(2) before payload_storage(3).
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        ids.iter()
            .map(|&id| {
                let vector = vector_storage.retrieve(id).ok().flatten();
                let payload = payload_storage.retrieve(id).ok().flatten();
                if vector.is_none() && payload.is_none() {
                    return None;
                }
                if is_payload_expired(payload.as_ref(), now_secs) {
                    return None;
                }
                Some(Point {
                    id,
                    vector: vector.unwrap_or_default(),
                    payload,
                    sparse_vectors: None,
                })
            })
            .collect()
    }
}
