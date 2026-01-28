//! WITH clause types for query-time configuration.
//!
//! This module defines WITH clause options for overriding
//! search parameters on a per-query basis.

use serde::{Deserialize, Serialize};

/// WITH clause for query-time configuration overrides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WithClause {
    /// Configuration options as key-value pairs.
    pub options: Vec<WithOption>,
}

impl WithClause {
    /// Creates a new empty WITH clause.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an option to the WITH clause.
    #[must_use]
    pub fn with_option(mut self, key: impl Into<String>, value: WithValue) -> Self {
        self.options.push(WithOption {
            key: key.into(),
            value,
        });
        self
    }

    /// Gets an option value by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&WithValue> {
        self.options
            .iter()
            .find(|opt| opt.key.eq_ignore_ascii_case(key))
            .map(|opt| &opt.value)
    }

    /// Gets the search mode if specified.
    #[must_use]
    pub fn get_mode(&self) -> Option<&str> {
        self.get("mode").and_then(|v| v.as_str())
    }

    /// Gets ef_search if specified.
    #[must_use]
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub fn get_ef_search(&self) -> Option<usize> {
        self.get("ef_search")
            .and_then(WithValue::as_integer)
            .map(|v| v as usize)
    }

    /// Gets timeout in milliseconds if specified.
    #[must_use]
    #[allow(clippy::cast_sign_loss)]
    pub fn get_timeout_ms(&self) -> Option<u64> {
        self.get("timeout_ms")
            .and_then(WithValue::as_integer)
            .map(|v| v as u64)
    }

    /// Gets rerank option if specified.
    #[must_use]
    pub fn get_rerank(&self) -> Option<bool> {
        self.get("rerank").and_then(WithValue::as_bool)
    }
}

/// A single option in a WITH clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WithOption {
    /// Option key.
    pub key: String,
    /// Option value.
    pub value: WithValue,
}

/// Value type for WITH clause options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WithValue {
    /// String value.
    String(String),
    /// Integer value.
    Integer(i64),
    /// Float value.
    Float(f64),
    /// Boolean value.
    Boolean(bool),
    /// Identifier (unquoted string).
    Identifier(String),
}

impl WithValue {
    /// Returns the value as a string if applicable.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Identifier(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the value as an integer.
    #[must_use]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns the value as a float.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            #[allow(clippy::cast_precision_loss)]
            Self::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Returns the value as a boolean.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}
