//! Resolution of a `SPARSE_NEAR` operand to the sparse vector it names.

use crate::error::{Error, Result};
use crate::sparse_index::SparseVector;

use super::SparseVectorExpr;

impl SparseVectorExpr {
    /// The sparse vector this expression names: the literal itself, or the
    /// bind parameter it names read from `params`. Every engine resolves a
    /// `SPARSE_NEAR` operand through this one function — core's query
    /// pipeline and the WASM executor alike.
    ///
    /// A parameter accepts two JSON formats:
    /// - Structured: `{"indices": [1,2,3], "values": [0.5, 0.3, 0.1]}`
    /// - Shorthand:  `{"12": 0.8, "45": 0.3}`
    ///
    /// # Errors
    ///
    /// Returns [`Error::QueryParameter`] if the bind parameter is missing or
    /// has the wrong shape, or [`Error::InvalidVector`] for a weight that is
    /// not finite.
    pub fn resolve(
        &self,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<SparseVector> {
        let vector = self.parse(params)?;
        // A JSON weight past f32 (`1e300`) narrows to infinity: the same
        // finite check every typed sparse search runs.
        crate::validation::validate_sparse_vector_is_finite(&vector)?;
        Ok(vector)
    }

    /// Reads the sparse vector this expression names, unchecked.
    fn parse(
        &self,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<SparseVector> {
        match self {
            Self::Literal(sv) => Ok(sv.clone()),
            Self::Parameter(name) => {
                let val = params
                    .get(name)
                    .ok_or_else(|| Error::unbound_parameter(name))?;
                let obj = val.as_object().ok_or_else(|| {
                    Error::invalid_parameter(
                        name,
                        "expected a sparse vector: {\"indices\": [...], \"values\": [...]} \
                         or an {index: weight} map",
                    )
                })?;

                // Try structured format first: {"indices": [...], "values": [...]}
                if let Some(sv) = try_parse_structured_sparse(obj, name)? {
                    return Ok(sv);
                }

                // Shorthand: {"12": 0.8, "45": 0.3}
                parse_shorthand_sparse(obj, name)
            }
        }
    }
}

/// Tries to parse a structured sparse vector format with `indices` and `values` arrays.
fn try_parse_structured_sparse(
    obj: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Option<SparseVector>> {
    let (Some(indices_val), Some(values_val)) = (obj.get("indices"), obj.get("values")) else {
        return Ok(None);
    };
    let indices: Vec<u32> = serde_json::from_value(indices_val.clone())
        .map_err(|e| Error::invalid_parameter(name, format!("indices: {e}")))?;
    let values: Vec<f32> = serde_json::from_value(values_val.clone())
        .map_err(|e| Error::invalid_parameter(name, format!("values: {e}")))?;
    if indices.len() != values.len() {
        return Err(Error::invalid_parameter(
            name,
            "indices and values must have equal length",
        ));
    }
    Ok(Some(SparseVector::new(
        indices.into_iter().zip(values).collect(),
    )))
}

/// Parses shorthand sparse vector format: `{"12": 0.8, "45": 0.3}`.
fn parse_shorthand_sparse(
    obj: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<SparseVector> {
    let mut pairs = Vec::with_capacity(obj.len());
    for (k, v) in obj {
        let idx: u32 = k.parse().map_err(|_| {
            Error::invalid_parameter(name, format!("key '{k}' is not a valid u32 index"))
        })?;
        #[allow(clippy::cast_possible_truncation)]
        let weight = v.as_f64().ok_or_else(|| {
            Error::invalid_parameter(name, format!("value for key '{k}' is not a number"))
        })? as f32;
        pairs.push((idx, weight));
    }
    Ok(SparseVector::new(pairs))
}
