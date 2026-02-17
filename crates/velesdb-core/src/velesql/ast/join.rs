//! JOIN clause types for `VelesQL`.
//!
//! This module defines join types and conditions for cross-store queries.

use serde::{Deserialize, Serialize};

/// JOIN clause for cross-store queries (EPIC-031 US-004).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinClause {
    /// Type of join (INNER, LEFT, RIGHT, FULL).
    pub join_type: JoinType,
    /// Table/store name to join.
    pub table: String,
    /// Optional alias for the joined table.
    pub alias: Option<String>,
    /// Join condition (ON clause).
    pub condition: Option<JoinCondition>,
    /// USING clause columns.
    pub using_columns: Option<Vec<String>>,
}

/// Type of SQL JOIN operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JoinType {
    /// INNER JOIN.
    #[default]
    Inner,
    /// LEFT JOIN.
    Left,
    /// RIGHT JOIN.
    Right,
    /// FULL JOIN.
    Full,
}

/// Join condition specifying how to link tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinCondition {
    /// Left side of the join.
    pub left: ColumnRef,
    /// Right side of the join.
    pub right: ColumnRef,
}

/// Column reference with optional table/alias prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnRef {
    /// Optional table or alias prefix.
    pub table: Option<String>,
    /// Column or property name.
    pub column: String,
}
