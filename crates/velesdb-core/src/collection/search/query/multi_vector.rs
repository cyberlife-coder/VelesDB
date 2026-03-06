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
use crate::error::{Error, Result};
use crate::storage::{PayloadStorage, VectorStorage};

impl Collection {
    /// Retrieves the vector for a given point ID from the named field.
    ///
    /// For `field == "vector"`, reads from the primary HNSW vector store.
    /// For any other field name, reads a JSON array from the point payload.
    ///
    /// # Errors
    ///
    /// Returns `Error::Config` if the field is not `"vector"` and the payload
    /// either has no such field or the value is not a JSON array of numbers.
    pub(crate) fn get_vector_for_field(
        &self,
        point_id: u64,
        field: &str,
    ) -> Result<Option<Vec<f32>>> {
        if field == "vector" {
            // Primary vector store
            let storage = self.vector_storage.read();
            return Ok(storage.retrieve(point_id)?);
        }

        // Named vector from payload JSON field
        let payload_storage = self.payload_storage.read();
        let Some(payload) = payload_storage.retrieve(point_id)? else {
            return Ok(None);
        };

        // Absent field → Ok(None): the point simply has no vector for this field.
        // This is normal for sparse multi-vector schemas and must not be treated as an error.
        // Reason: returning Err here caused all callers to silently skip ALL points when the
        // field name had a typo, making wrong queries return empty results with no error.
        let Some(value) = payload.get(field) else {
            return Ok(None);
        };

        let Some(array) = value.as_array() else {
            // Field exists but is not an array — this IS a data error.
            return Err(Error::Config(format!(
                "similarity() field '{}' is not a numeric array in payload.",
                field
            )));
        };

        #[allow(clippy::cast_possible_truncation)]
        // Reason: JSON vector components are f64 embeddings; f32 truncation is intentional
        // since the entire vector store operates on f32 precision.
        let vec: Option<Vec<f32>> = array.iter().map(|v| v.as_f64().map(|f| f as f32)).collect();

        match vec {
            Some(v) => Ok(Some(v)),
            None => Err(Error::Config(format!(
                "similarity() field '{}' contains non-numeric values.",
                field
            ))),
        }
    }
}
