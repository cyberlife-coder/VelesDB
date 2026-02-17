//! Value types for `VelesQL` expressions.
//!
//! This module defines values, vectors, temporal expressions,
//! and subquery types used in `VelesQL` queries.

use serde::{Deserialize, Serialize};

/// Vector expression in a NEAR clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VectorExpr {
    /// Literal vector: [0.1, 0.2, ...]
    Literal(Vec<f32>),
    /// Parameter reference: `$param_name`
    Parameter(String),
}

/// A value in `VelesQL`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// Integer value.
    Integer(i64),
    /// Float value.
    Float(f64),
    /// String value.
    String(String),
    /// Boolean value.
    Boolean(bool),
    /// Null value.
    Null,
    /// Parameter reference.
    Parameter(String),
    /// Temporal function (EPIC-038).
    Temporal(TemporalExpr),
    /// Scalar subquery (EPIC-039).
    Subquery(Box<Subquery>),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Integer(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Boolean(v)
    }
}

/// Scalar subquery expression (EPIC-039).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subquery {
    /// The SELECT statement of the subquery.
    pub select: super::select::SelectStatement,
    /// Correlated columns (references to outer query).
    #[serde(default)]
    pub correlations: Vec<CorrelatedColumn>,
}

/// A correlated column reference in a subquery (EPIC-039).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelatedColumn {
    /// Outer query table/alias reference.
    pub outer_table: String,
    /// Column name in outer query.
    pub outer_column: String,
    /// Column in subquery that references it.
    pub inner_column: String,
}

/// Temporal expression for date/time operations (EPIC-038).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemporalExpr {
    /// Current timestamp: `NOW()`
    Now,
    /// Interval expression: `INTERVAL '7 days'`
    Interval(IntervalValue),
    /// Arithmetic: `NOW() - INTERVAL '7 days'`
    Subtract(Box<TemporalExpr>, Box<TemporalExpr>),
    /// Arithmetic: `NOW() + INTERVAL '1 hour'`
    Add(Box<TemporalExpr>, Box<TemporalExpr>),
}

impl TemporalExpr {
    /// Evaluates the temporal expression to epoch seconds.
    #[must_use]
    pub fn to_epoch_seconds(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};

        // SAFETY: Current Unix timestamps fit in i64 until year 292 billion.
        // Use saturating conversion for theoretical future-proofing.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        match self {
            Self::Now => now,
            Self::Interval(iv) => iv.to_seconds(),
            Self::Subtract(left, right) => left.to_epoch_seconds() - right.to_epoch_seconds(),
            Self::Add(left, right) => left.to_epoch_seconds() + right.to_epoch_seconds(),
        }
    }
}

/// Interval value with magnitude and unit (EPIC-038).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalValue {
    /// Numeric magnitude.
    pub magnitude: i64,
    /// Time unit.
    pub unit: IntervalUnit,
}

impl IntervalValue {
    /// Converts the interval to seconds.
    #[must_use]
    pub fn to_seconds(&self) -> i64 {
        match self.unit {
            IntervalUnit::Seconds => self.magnitude,
            IntervalUnit::Minutes => self.magnitude * 60,
            IntervalUnit::Hours => self.magnitude * 3600,
            IntervalUnit::Days => self.magnitude * 86400,
            IntervalUnit::Weeks => self.magnitude * 604_800,
            IntervalUnit::Months => self.magnitude * 2_592_000,
        }
    }
}

/// Time unit for INTERVAL expressions (EPIC-038).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntervalUnit {
    /// Seconds.
    Seconds,
    /// Minutes.
    Minutes,
    /// Hours.
    Hours,
    /// Days.
    Days,
    /// Weeks.
    Weeks,
    /// Months.
    Months,
}
