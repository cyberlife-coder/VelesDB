//! Secondary index types for metadata payload fields.

use parking_lot::RwLock;
use serde_json::Number;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Orderable JSON primitive value used as a key in secondary indexes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JsonValue {
    /// String JSON value.
    String(String),
    /// Numeric JSON value (normalized to f64 bits).
    Number(F64Key),
    /// Boolean JSON value.
    Bool(bool),
}

/// Wrapper type that provides total ordering for f64 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct F64Key(u64);

impl From<f64> for F64Key {
    fn from(value: f64) -> Self {
        Self(value.to_bits())
    }
}

impl PartialOrd for F64Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F64Key {
    fn cmp(&self, other: &Self) -> Ordering {
        f64::from_bits(self.0).total_cmp(&f64::from_bits(other.0))
    }
}

impl PartialOrd for JsonValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsonValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::Bool(_), _) | (Self::Number(_), Self::String(_)) => Ordering::Less,
            (Self::Number(_), Self::Bool(_)) | (Self::String(_), _) => Ordering::Greater,
        }
    }
}

impl JsonValue {
    /// Converts JSON payload primitive to an orderable key.
    #[must_use]
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(s) => Some(Self::String(s.clone())),
            serde_json::Value::Number(n) => Self::number_from_json(n),
            serde_json::Value::Bool(b) => Some(Self::Bool(*b)),
            _ => None,
        }
    }

    /// Converts VelesQL AST value into an index key.
    #[must_use]
    pub fn from_ast_value(value: &crate::velesql::Value) -> Option<Self> {
        match value {
            crate::velesql::Value::String(s) => Some(Self::String(s.clone())),
            crate::velesql::Value::Integer(i) => i
                .to_string()
                .parse::<f64>()
                .ok()
                .map(|v| Self::Number(F64Key::from(v))),
            crate::velesql::Value::Float(f) => Some(Self::Number(F64Key::from(*f))),
            crate::velesql::Value::Boolean(b) => Some(Self::Bool(*b)),
            _ => None,
        }
    }

    fn number_from_json(number: &Number) -> Option<Self> {
        number.as_f64().map(|v| Self::Number(F64Key::from(v)))
    }
}

/// Secondary index implementation.
#[derive(Debug)]
pub enum SecondaryIndex {
    /// B-tree index mapping JSON primitive values to point IDs.
    BTree(RwLock<BTreeMap<JsonValue, Vec<u64>>>),
}
