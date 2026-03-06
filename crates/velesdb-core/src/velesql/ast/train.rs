//! TRAIN QUANTIZER statement AST node.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::WithValue;

/// A TRAIN QUANTIZER statement.
///
/// Represents: `TRAIN QUANTIZER ON <collection> WITH (m=8, k=256, ...)`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainStatement {
    /// Target collection name.
    pub collection: String,
    /// Training parameters (m, k, type, oversampling, sample, force, etc.).
    pub params: HashMap<String, WithValue>,
}
