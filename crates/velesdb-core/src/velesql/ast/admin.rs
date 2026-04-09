//! Admin statement types for VelesQL.
//!
//! This module defines FLUSH and other administrative statement AST nodes.

use serde::{Deserialize, Serialize};

/// Admin statements for database maintenance operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AdminStatement {
    /// `FLUSH [FULL] [collection]` -- persist collection data to disk.
    Flush(FlushStatement),
}

/// `FLUSH [FULL] [collection]` statement.
///
/// - `full = false`: WAL-only fast flush (default).
/// - `full = true`: Full flush including index serialization.
/// - `collection = None`: Flush all collections.
/// - `collection = Some(name)`: Flush a specific collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlushStatement {
    /// Whether to perform a full flush (includes index serialization).
    pub full: bool,
    /// Optional collection name; `None` means flush all.
    pub collection: Option<String>,
}
