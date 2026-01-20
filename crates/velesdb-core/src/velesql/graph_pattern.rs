//! Graph pattern AST types for MATCH clause (Graph Pattern Matching).
//!
//! This module contains AST types for Cypher-like graph queries in VelesQL.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::ast::{Condition, Value};

/// A MATCH clause for graph pattern matching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchClause {
    /// Graph patterns to match.
    pub patterns: Vec<GraphPattern>,
    /// Optional WHERE clause.
    pub where_clause: Option<Condition>,
    /// RETURN clause.
    pub return_clause: ReturnClause,
}

/// A graph pattern (path or named path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphPattern {
    /// Optional path name (for `p = (a)-[*]->(b)`).
    pub name: Option<String>,
    /// Nodes in the pattern.
    pub nodes: Vec<NodePattern>,
    /// Relationships between nodes.
    pub relationships: Vec<RelationshipPattern>,
}

/// A node pattern in a graph query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodePattern {
    /// Optional alias (e.g., `n` in `(n:Person)`).
    pub alias: Option<String>,
    /// Node labels (e.g., `["Person", "Author"]`).
    pub labels: Vec<String>,
    /// Node properties for filtering.
    pub properties: HashMap<String, Value>,
}

impl NodePattern {
    /// Creates a new empty node pattern.
    #[must_use]
    pub fn new() -> Self {
        Self {
            alias: None,
            labels: Vec::new(),
            properties: HashMap::new(),
        }
    }

    /// Sets the alias.
    #[must_use]
    pub fn with_alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    /// Adds a label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.labels.push(label.into());
        self
    }
}

impl Default for NodePattern {
    fn default() -> Self {
        Self::new()
    }
}

/// A relationship pattern in a graph query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipPattern {
    /// Optional alias (e.g., `r` in `-[r:WROTE]->`).
    pub alias: Option<String>,
    /// Relationship types (e.g., `["WROTE", "CREATED"]` for `[:WROTE|CREATED]`).
    pub types: Vec<String>,
    /// Direction of the relationship.
    pub direction: Direction,
    /// Variable length range (e.g., `(1, 3)` for `*1..3`).
    pub range: Option<(u32, u32)>,
    /// Relationship properties for filtering.
    pub properties: HashMap<String, Value>,
}

impl RelationshipPattern {
    /// Creates a new relationship pattern with direction.
    #[must_use]
    pub fn new(direction: Direction) -> Self {
        Self {
            alias: None,
            types: Vec::new(),
            direction,
            range: None,
            properties: HashMap::new(),
        }
    }
}

/// Direction of a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Outgoing: `-->`
    Outgoing,
    /// Incoming: `<--`
    Incoming,
    /// Both/undirected: `--`
    Both,
}

/// RETURN clause for specifying output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnClause {
    /// Items to return.
    pub items: Vec<ReturnItem>,
    /// Optional ORDER BY.
    pub order_by: Option<Vec<OrderByItem>>,
    /// Optional LIMIT.
    pub limit: Option<u64>,
}

/// A single item in the RETURN clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnItem {
    /// Expression to return (e.g., `n.name`).
    pub expression: String,
    /// Optional alias (e.g., `AS name`).
    pub alias: Option<String>,
}

/// ORDER BY item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderByItem {
    /// Expression to order by.
    pub expression: String,
    /// Sort order.
    pub descending: bool,
}
