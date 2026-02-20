//! DML statement types for VelesQL.
//!
//! This module defines INSERT/UPDATE statement AST nodes.

use serde::{Deserialize, Serialize};

use super::{Condition, Value};

/// INSERT statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertStatement {
    /// Target collection/table name.
    pub table: String,
    /// Target columns.
    pub columns: Vec<String>,
    /// Values corresponding to `columns`.
    pub values: Vec<Value>,
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
    /// Target collection/table name.
    pub table: String,
    /// SET assignments.
    pub assignments: Vec<UpdateAssignment>,
    /// Optional WHERE clause.
    pub where_clause: Option<Condition>,
}

/// DML statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DmlStatement {
    /// INSERT statement.
    Insert(InsertStatement),
    /// UPDATE statement.
    Update(UpdateStatement),
}
