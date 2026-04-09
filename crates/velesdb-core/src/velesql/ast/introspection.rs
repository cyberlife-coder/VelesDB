//! Introspection statement types for VelesQL.
//!
//! This module defines SHOW, DESCRIBE, and EXPLAIN statement AST nodes.

use serde::{Deserialize, Serialize};

/// Introspection statements for database/collection metadata queries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum IntrospectionStatement {
    /// `SHOW COLLECTIONS` -- lists all collection names and types.
    ShowCollections,
    /// `DESCRIBE [COLLECTION] <name>` -- returns collection metadata.
    DescribeCollection(DescribeCollectionStatement),
    /// `EXPLAIN <query>` -- returns the query execution plan without executing.
    Explain(Box<super::Query>),
}

/// `DESCRIBE COLLECTION <name>` statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescribeCollectionStatement {
    /// Collection name to describe.
    pub name: String,
}
