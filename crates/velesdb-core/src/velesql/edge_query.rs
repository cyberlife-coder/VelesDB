//! What a `SELECT EDGES … WHERE` asks for, read once for every engine that
//! answers it (core's DML executor, the WASM graph store).
//!
//! The grammar is deliberately small: an AND chain whose every conjunct is
//! `source = N`, `target = N` or `label = 'X'`, optionally parenthesised.
//! Anything else — `OR`, `!=`, another column, a bound parameter, a negative
//! id — is refused, never ignored: an ignored conjunct would return edges the
//! query excluded.

use crate::error::{Error, Result};
use crate::velesql::{CompareOp, Comparison, Condition, Value};

/// The edge constraints of a `SELECT EDGES` WHERE clause.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdgeQuery {
    /// `source = N`.
    pub source: Option<u64>,
    /// `target = N`.
    pub target: Option<u64>,
    /// `label = 'X'`.
    pub label: Option<String>,
    /// Two conjuncts pinned one column to different values: nothing matches.
    pub contradictory: bool,
}

impl EdgeQuery {
    /// Reads `condition` (absent: every edge).
    ///
    /// # Errors
    ///
    /// [`Error::Query`] for a condition outside the grammar above.
    pub fn from_where(condition: Option<&Condition>) -> Result<Self> {
        let mut query = Self::default();
        if let Some(condition) = condition {
            query.add(condition)?;
        }
        Ok(query)
    }

    fn add(&mut self, condition: &Condition) -> Result<()> {
        match condition {
            Condition::Comparison(cmp) => self.add_comparison(cmp),
            Condition::And(left, right) => {
                self.add(left)?;
                self.add(right)
            }
            Condition::Group(inner) => self.add(inner),
            _ => Err(Error::Query(
                "SELECT EDGES WHERE supports: source=N, target=N, label='X', and AND combinations"
                    .to_string(),
            )),
        }
    }

    fn add_comparison(&mut self, cmp: &Comparison) -> Result<()> {
        if cmp.operator != CompareOp::Eq {
            return Err(Error::Query(
                "SELECT EDGES WHERE only supports '=' operator".to_string(),
            ));
        }
        match cmp.column.as_str() {
            "source" => {
                let id = literal_id(&cmp.value)?;
                self.contradictory |= self.source.replace(id).is_some_and(|prev| prev != id);
            }
            "target" => {
                let id = literal_id(&cmp.value)?;
                self.contradictory |= self.target.replace(id).is_some_and(|prev| prev != id);
            }
            "label" => {
                let Value::String(label) = &cmp.value else {
                    return Err(Error::Query("Expected a string value".to_string()));
                };
                self.contradictory |= self
                    .label
                    .replace(label.clone())
                    .is_some_and(|prev| prev != *label);
            }
            col => {
                return Err(Error::Query(format!(
                    "SELECT EDGES WHERE does not support column '{col}'. Use: source, target, label"
                )))
            }
        }
        Ok(())
    }

    /// Whether an edge with these endpoints and label satisfies the query.
    #[must_use]
    pub fn matches(&self, source: u64, target: u64, label: &str) -> bool {
        !self.contradictory
            && self.source.is_none_or(|s| s == source)
            && self.target.is_none_or(|t| t == target)
            && self.label.as_deref().is_none_or(|l| l == label)
    }

    /// The properties of an `INSERT EDGE … WITH PROPERTIES (…)`: literal
    /// values only, on every engine.
    ///
    /// # Errors
    ///
    /// [`Error::Query`] for a bound parameter, a temporal expression or a
    /// subquery.
    pub fn literal_properties(
        properties: &[(String, Value)],
    ) -> Result<serde_json::Map<String, serde_json::Value>> {
        properties
            .iter()
            .map(|(key, value)| match value {
                Value::Parameter(_) | Value::Temporal(_) | Value::Subquery(_) => Err(Error::Query(
                    "Edge properties must be literal values".to_string(),
                )),
                _ => Ok((key.clone(), value.to_json())),
            })
            .collect()
    }

    /// An edge label as every engine stores it: trimmed, never empty.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidEdgeLabel`] for an empty or whitespace-only label.
    pub fn normalized_label(label: &str) -> Result<String> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidEdgeLabel(
                "Edge label cannot be empty or whitespace-only".to_string(),
            ));
        }
        Ok(trimmed.to_string())
    }

    /// The row a `SELECT EDGES` answers for one edge, on every engine.
    #[must_use]
    pub fn row(
        id: u64,
        source: u64,
        target: u64,
        label: &str,
        properties: &serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "edge_id": id,
            "source": source,
            "target": target,
            "label": label,
            "properties": properties,
        })
    }
}

/// A point or edge-endpoint id literal: a non-negative integer.
///
/// # Errors
///
/// [`Error::Query`] for a negative integer or a non-integer value.
pub(crate) fn literal_id(value: &Value) -> Result<u64> {
    match value {
        Value::Integer(v) => {
            u64::try_from(*v).map_err(|_| Error::Query(format!("ID must be non-negative, got {v}")))
        }
        Value::UnsignedInteger(v) => Ok(*v),
        _ => Err(Error::Query("ID values must be integers".to_string())),
    }
}

#[cfg(test)]
#[path = "edge_query_tests.rs"]
mod tests;
