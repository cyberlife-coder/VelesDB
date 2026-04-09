//! DML statement types for VelesQL.
//!
//! This module defines INSERT/UPDATE/DELETE and graph mutation AST nodes.

use serde::{Deserialize, Serialize};

use super::{Condition, Value};

/// INSERT or UPSERT statement (supports multi-row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertStatement {
    /// Target collection name.
    pub table: String,
    /// Target columns.
    pub columns: Vec<String>,
    /// Rows of values; each row corresponds to `columns`.
    ///
    /// **Breaking change from v3.4**: type changed from `Vec<Value>` (single
    /// row) to `Vec<Vec<Value>>` (multi-row). No backward-compatible
    /// deserialization — the plan cache is in-memory (`Instant`-keyed) so
    /// no persistent data uses this format.
    pub rows: Vec<Vec<Value>>,
}

/// UPDATE assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateAssignment {
    /// Column name to update.
    pub column: String,
    /// Assigned value expression.
    pub value: Value,
}

/// UPDATE statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateStatement {
    /// Target collection name.
    pub table: String,
    /// SET assignments.
    pub assignments: Vec<UpdateAssignment>,
    /// Optional WHERE clause.
    pub where_clause: Option<Condition>,
}

/// INSERT EDGE statement (VelesQL v3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertEdgeStatement {
    /// Target graph collection name.
    pub collection: String,
    /// Optional explicit edge ID.
    pub edge_id: Option<u64>,
    /// Source node ID.
    pub source: u64,
    /// Target node ID.
    pub target: u64,
    /// Edge label/type.
    pub label: String,
    /// Optional edge properties.
    pub properties: Vec<(String, Value)>,
}

/// DELETE FROM statement (VelesQL v3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteStatement {
    /// Target collection name.
    pub table: String,
    /// WHERE clause (mandatory — prevents accidental full deletion).
    pub where_clause: Condition,
}

/// DELETE EDGE statement (VelesQL v3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteEdgeStatement {
    /// Target graph collection name.
    pub collection: String,
    /// Edge ID to delete.
    pub edge_id: u64,
}

/// SELECT EDGES statement (VelesQL v3.5 Phase 5).
///
/// Queries edges from a graph collection with optional filtering
/// by source node, target node, or edge label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectEdgesStatement {
    /// Target graph collection name.
    pub collection: String,
    /// Optional WHERE clause for filtering (source=N, target=N, label='X').
    pub where_clause: Option<Condition>,
    /// Optional LIMIT clause.
    pub limit: Option<u64>,
}

/// INSERT NODE statement (VelesQL v3.5 Phase 5).
///
/// Inserts or updates a node payload in a graph collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertNodeStatement {
    /// Target graph collection name.
    pub collection: String,
    /// Node ID.
    pub node_id: u64,
    /// JSON payload for the node.
    pub payload: serde_json::Value,
}

/// DML statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DmlStatement {
    /// INSERT statement.
    Insert(InsertStatement),
    /// UPSERT statement (VelesQL v3.5 Phase 4).
    Upsert(InsertStatement),
    /// UPDATE statement.
    Update(UpdateStatement),
    /// INSERT EDGE statement (VelesQL v3.3).
    InsertEdge(InsertEdgeStatement),
    /// DELETE FROM statement (VelesQL v3.3).
    Delete(DeleteStatement),
    /// DELETE EDGE statement (VelesQL v3.3).
    DeleteEdge(DeleteEdgeStatement),
    /// SELECT EDGES statement (VelesQL v3.5 Phase 5).
    SelectEdges(SelectEdgesStatement),
    /// INSERT NODE statement (VelesQL v3.5 Phase 5).
    InsertNode(InsertNodeStatement),
}
