//! ORDER BY resolution for grouped aggregation rows.
//!
//! A grouped row carries one field per GROUP BY column and one per SELECT
//! aggregate. An ORDER BY item either names one of those fields — a GROUP BY
//! column (`ORDER BY category`), an aggregate's alias or default name
//! (`ORDER BY n`, `ORDER BY count`) — or repeats an aggregate expression
//! (`ORDER BY COUNT(*)`). The parser gives the repeated expression no alias,
//! so it carries no name of its own: deriving one from it looked rows up under
//! `count` while `COUNT(*) AS n` had stored them under `n`, every comparison
//! came back equal, and LIMIT kept whichever groups came first.
//!
//! [`SelectStatement::grouped_sort_keys`] is the one place an item is matched
//! to the column it means. Core's `execute_aggregate` and the WASM aggregate
//! pipeline both sort through it, each reading an aggregate under the name it
//! stored it with (the two name an unaliased aggregate differently — see
//! `documented_divergences` D001 in `conformance/velesql_executor_cases.json`).
//! An item that names no column of the rows is refused with `V015` instead of
//! comparing every row as equal.

use super::ast::{AggregateArg, AggregateFunction, OrderByExpr, SelectColumns, SelectStatement};
use super::validation::{ValidationError, ValidationErrorKind};

/// The column of a grouped aggregation row one ORDER BY item sorts on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GroupedSortKey<'a> {
    /// A GROUP BY column, by name.
    GroupColumn(&'a str),
    /// A SELECT aggregate, stored under whatever name the executor gives it.
    Aggregate(&'a AggregateFunction),
}

impl SelectStatement {
    /// Resolves the ORDER BY of a GROUP BY aggregation to the row columns it
    /// sorts on, in order, each with its direction (`true` is descending).
    ///
    /// An item resolves to a SELECT aggregate when it repeats it
    /// (`ORDER BY COUNT(*)`, whatever its alias) or names its alias or default
    /// name (`ORDER BY n`, `ORDER BY count`), and to a GROUP BY column when it
    /// names one. A statement without ORDER BY resolves to no keys.
    ///
    /// # Errors
    ///
    /// [`ValidationErrorKind::OrderByNotInResult`] (`V015`) for an item that
    /// names no column of the grouped rows: an aggregate the SELECT does not
    /// list, a column outside GROUP BY, `similarity()` or an arithmetic
    /// expression.
    pub fn grouped_sort_keys(&self) -> Result<Vec<(GroupedSortKey<'_>, bool)>, ValidationError> {
        self.order_by
            .iter()
            .flatten()
            .map(|item| Ok((self.grouped_sort_key(&item.expr)?, item.descending)))
            .collect()
    }

    /// The SELECT aggregate an `ORDER BY <aggregate>` item repeats: the same
    /// function over the same argument, whatever alias either carries.
    ///
    /// # Errors
    ///
    /// `V015` when the SELECT lists no such aggregate.
    pub(crate) fn order_by_aggregate_target(
        &self,
        wanted: &AggregateFunction,
    ) -> Result<&AggregateFunction, ValidationError> {
        self.aggregations()
            .iter()
            .find(|agg| {
                agg.function_type == wanted.function_type && agg.argument == wanted.argument
            })
            .ok_or_else(|| {
                not_in_result(
                    describe_aggregate(wanted),
                    "add the aggregate to the SELECT list to sort groups by it",
                )
            })
    }

    fn grouped_sort_key(&self, expr: &OrderByExpr) -> Result<GroupedSortKey<'_>, ValidationError> {
        match expr {
            OrderByExpr::Aggregate(wanted) => self
                .order_by_aggregate_target(wanted)
                .map(GroupedSortKey::Aggregate),
            OrderByExpr::Field(name) => self.grouped_column_named(name).ok_or_else(|| {
                not_in_result(
                    name.clone(),
                    "order groups by a GROUP BY column, or by a SELECT aggregate, \
                     its alias or its default name",
                )
            }),
            OrderByExpr::Similarity(_) | OrderByExpr::SimilarityBare => Err(not_in_result(
                "similarity()",
                "grouped rows carry no score: order groups by a GROUP BY column or a \
                 SELECT aggregate",
            )),
            OrderByExpr::Arithmetic(arithmetic) => Err(not_in_result(
                arithmetic.to_string(),
                "grouped rows carry no computed column: order groups by a GROUP BY column \
                 or a SELECT aggregate",
            )),
        }
    }

    /// The row column `name` refers to. An aggregate takes precedence over a
    /// GROUP BY column of the same name, as both executors write aggregates
    /// into the row after the group keys.
    fn grouped_column_named(&self, name: &str) -> Option<GroupedSortKey<'_>> {
        let aggregate = self
            .aggregations()
            .iter()
            .find(|agg| agg.result_key() == name)
            .map(GroupedSortKey::Aggregate);
        aggregate.or_else(|| {
            self.group_by
                .iter()
                .flat_map(|group_by| &group_by.columns)
                .find(|column| *column == name)
                .map(|column| GroupedSortKey::GroupColumn(column))
        })
    }

    fn aggregations(&self) -> &[AggregateFunction] {
        match &self.columns {
            SelectColumns::Aggregations(aggregations)
            | SelectColumns::Mixed { aggregations, .. } => aggregations,
            _ => &[],
        }
    }
}

/// Validates the ORDER BY of a GROUP BY aggregation against its grouped rows
/// (`V015`). A vector GROUP BY (`NEAR` in WHERE) is not one: it returns
/// search results whose ORDER BY may also use `similarity()`, and resolves its
/// aggregate items when it groups (`vector_group_by::order_by_on_group_fields`).
pub(super) fn validate_grouped_order_by(stmt: &SelectStatement) -> Result<(), ValidationError> {
    if stmt.group_by.is_some() && stmt.is_aggregation_query() {
        stmt.grouped_sort_keys()?;
    }
    Ok(())
}

/// A `V015` error naming the unresolved ORDER BY item, which the error's
/// display shows through its suggestion only.
fn not_in_result(item: impl Into<String>, remedy: &str) -> ValidationError {
    let item = item.into();
    let suggestion = format!("ORDER BY {item}: {remedy}");
    ValidationError::new(
        ValidationErrorKind::OrderByNotInResult,
        None,
        item,
        suggestion,
    )
}

/// `SUM(price)`-style rendering of an aggregate for an error message.
fn describe_aggregate(agg: &AggregateFunction) -> String {
    let function = format!("{:?}", agg.function_type).to_uppercase();
    let argument = match &agg.argument {
        AggregateArg::Wildcard => "*",
        AggregateArg::Score => "score",
        AggregateArg::Column(column) => column,
    };
    format!("{function}({argument})")
}
