//! Multi-vector field support for similarity() queries (P1-A).
//!
//! Allows `similarity(field_name, $v)` to reference vectors stored either:
//! - In the primary HNSW vector store (field name `"vector"`)
//! - As a JSON array in the point payload under the given field name
//!
//! # Example
//!
//! ```ignore
//! // Primary vector field
//! SELECT * FROM docs WHERE similarity(vector, $v) > 0.8 LIMIT 10;
//!
//! // Named vector stored in payload
//! SELECT * FROM docs WHERE similarity(title_embedding, $v) > 0.8 LIMIT 10;
//! ```

use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Retrieves the vector for a given point ID from the named field.
    ///
    /// For `field == "vector"`, reads from the primary HNSW vector store.
    /// For any other field name, reads a JSON array from the point payload.
    ///
    /// # Errors
    ///
    /// Returns `Error::Query` for a non-null named field whose value is
    /// outside the JSON array-of-numbers representation.
    pub(crate) fn get_vector_for_field(
        &self,
        point_id: u64,
        field: &str,
    ) -> Result<Option<Vec<f32>>> {
        if field == crate::velesql::PRIMARY_VECTOR_FIELD {
            // Primary vector store
            let storage = self.storage.vector_storage.read();
            return Ok(storage.retrieve(point_id)?);
        }

        // Named vector from payload JSON field
        let payload_storage = self.storage.payload_storage.read();
        Self::vector_from_payload_field(&payload_storage, point_id, field)
    }

    /// Reads a named vector from a payload JSON field through an
    /// already-held payload guard ([`crate::velesql::payload_vector`]).
    fn vector_from_payload_field(
        payload_storage: &crate::storage::LogPayloadStorage,
        point_id: u64,
        field: &str,
    ) -> Result<Option<Vec<f32>>> {
        let Some(payload) = payload_storage.retrieve(point_id)? else {
            return Ok(None);
        };
        crate::velesql::payload_vector(&payload, field)
    }
}
