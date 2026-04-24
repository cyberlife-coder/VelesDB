//! Window function types for VelesQL (Issue #386).
//!
//! Phase 1: `ROW_NUMBER`, `RANK`, `DENSE_RANK` with `PARTITION BY` + `ORDER BY`.

use serde::{Deserialize, Serialize};

/// Window function type (Phase 1: ranking functions).
///
/// Marked `#[non_exhaustive]` so future phases (`LAG`, `LEAD`,
/// `FIRST_VALUE`, `NTILE`, aggregate windows, …) can be added
/// without a semver break. Downstream crates must include a
/// wildcard (`_ =>`) arm when matching on this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WindowFunctionType {
    /// `ROW_NUMBER()` — sequential numbering 1..N within each partition.
    RowNumber,
    /// `RANK()` — ranking with gaps on ties (e.g., 1, 2, 2, 4).
    Rank,
    /// `DENSE_RANK()` — ranking without gaps on ties (e.g., 1, 2, 2, 3).
    DenseRank,
}

impl WindowFunctionType {
    /// Returns the default column alias for this function type.
    #[must_use]
    pub fn default_alias(&self) -> &'static str {
        match self {
            Self::RowNumber => "row_number",
            Self::Rank => "rank",
            Self::DenseRank => "dense_rank",
        }
    }
}

/// `ORDER BY` item inside a window `OVER` clause.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowOrderBy {
    /// Column to sort by within the partition.
    pub column: String,
    /// Sort direction (`true` = DESC).
    pub descending: bool,
}

/// The `OVER` clause defining the window specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverClause {
    /// `PARTITION BY` columns (empty = entire result set is one partition).
    #[serde(default)]
    pub partition_by: Vec<String>,
    /// `ORDER BY` within each partition.
    #[serde(default)]
    pub order_by: Vec<WindowOrderBy>,
}

/// A window function expression in the SELECT list.
///
/// Example: `ROW_NUMBER() OVER (PARTITION BY source ORDER BY score DESC) AS rn`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowFunction {
    /// Type of window function.
    pub function_type: WindowFunctionType,
    /// `OVER` clause specification.
    pub over_clause: OverClause,
    /// Optional alias (`AS` clause). Defaults to function name.
    pub alias: Option<String>,
}
