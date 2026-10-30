//! ORDER BY resolution for grouped aggregation rows.
//!
//! A grouped row carries one field per GROUP BY column and one per SELECT
//! aggregate. An ORDER BY item either names one of those fields or repeats an
//! aggregate expression (`ORDER BY COUNT(*)`). The parser gives the repeated
//! expression no alias, so it carries no name of its own: deriving one from it
//! looked rows up under `count` while `COUNT(*) AS n` had stored them under
//! `n`, every comparison came back equal, and LIMIT kept whichever groups came
//! first.
//!
//! [`SelectStatement::grouped_sort_keys`] is the one place an item is matched
//! to the column it means, and [`SelectStatement::order_by_aggregates_as_fields`]
//! the one place a vector GROUP BY maps its ORDER BY aggregates to fields.
//! Core and the WASM executor both go through them, each passing the names it
//! gives an unaliased aggregate (the two differ — see `documented_divergences`
//! D001 in `conformance/velesql_executor_cases.json`). An item that names no
//! column of the rows is refused with `V016` instead of comparing every row as
//! equal.

use std::borrow::Cow;

use super::ast::{
    AggregateArg, AggregateFunction, AggregateType, OrderByExpr, SelectColumns, SelectStatement,
};
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
    /// An item that repeats a SELECT aggregate (`ORDER BY COUNT(*)`, whatever
    /// its alias) resolves to it. A name resolves, in this order, to the SELECT
    /// aggregate it aliases, to a GROUP BY column, then to an unaliased
    /// aggregate whose `default_name` — the name the calling executor outputs
    /// it under — it is. A statement without ORDER BY resolves to no keys.
    ///
    /// # Errors
    ///
    /// [`ValidationErrorKind::OrderByNotInResult`] (`V016`) for an item that
    /// names no column of the grouped rows: an aggregate the SELECT does not
    /// list, a name none of the above, `similarity()` or an arithmetic
    /// expression.
    pub fn grouped_sort_keys(
        &self,
        default_name: impl Fn(&AggregateFunction) -> String,
    ) -> Result<Vec<(GroupedSortKey<'_>, bool)>, ValidationError> {
        self.order_by
            .iter()
            .flatten()
            .map(|item| {
                Ok((
                    self.grouped_sort_key(&item.expr, &default_name)?,
                    item.descending,
                ))
            })
            .collect()
    }

    /// For a vector GROUP BY, whose groups come back as search results: this
    /// statement with each `ORDER BY <aggregate>` replaced by the field the
    /// executor stores that SELECT aggregate in (`output_name`), so the search
    /// ORDER BY sorts on it. Every other item — `similarity()`, arithmetic, a
    /// field — is the search ORDER BY's to evaluate. Borrows the statement
    /// unchanged when no item is an aggregate.
    ///
    /// # Errors
    ///
    /// `V016` for an ORDER BY aggregate the SELECT does not list.
    pub fn order_by_aggregates_as_fields(
        &self,
        output_name: impl Fn(&AggregateFunction) -> String,
    ) -> Result<Cow<'_, Self>, ValidationError> {
        let has_aggregate_item = self
            .order_by
            .iter()
            .flatten()
            .any(|item| matches!(item.expr, OrderByExpr::Aggregate(_)));
        if !has_aggregate_item {
            return Ok(Cow::Borrowed(self));
        }
        let mut rewritten = self.clone();
        for item in rewritten.order_by.iter_mut().flatten() {
            if let OrderByExpr::Aggregate(wanted) = &item.expr {
                let field = output_name(self.order_by_aggregate_target(wanted)?);
                item.expr = OrderByExpr::Field(field);
            }
        }
        Ok(Cow::Owned(rewritten))
    }

    /// The SELECT aggregate an `ORDER BY <aggregate>` item repeats: the same
    /// function over the same argument, whatever alias either carries.
    fn order_by_aggregate_target(
        &self,
        wanted: &AggregateFunction,
    ) -> Result<&AggregateFunction, ValidationError> {
        aggregations(self)
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

    fn grouped_sort_key(
        &self,
        expr: &OrderByExpr,
        default_name: &impl Fn(&AggregateFunction) -> String,
    ) -> Result<GroupedSortKey<'_>, ValidationError> {
        match expr {
            OrderByExpr::Aggregate(wanted) => self
                .order_by_aggregate_target(wanted)
                .map(GroupedSortKey::Aggregate),
            OrderByExpr::Field(name) => {
                self.grouped_column_named(name, default_name)
                    .ok_or_else(|| {
                        not_in_result(
                            name.clone(),
                            "order groups by a GROUP BY column, or by a SELECT aggregate, \
                             its alias or its default name",
                        )
                    })
            }
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

    /// The row column `name` refers to, with SQL's precedence: an explicit
    /// SELECT alias, then a GROUP BY column, then the executor's default name
    /// for an unaliased aggregate.
    fn grouped_column_named(
        &self,
        name: &str,
        default_name: &impl Fn(&AggregateFunction) -> String,
    ) -> Option<GroupedSortKey<'_>> {
        let aggregations = aggregations(self);
        let aliased = aggregations
            .iter()
            .find(|agg| agg.alias.as_deref() == Some(name));
        let group_column = || {
            self.group_by
                .iter()
                .flat_map(|group_by| &group_by.columns)
                .find(|column| *column == name)
                .map(|column| GroupedSortKey::GroupColumn(column))
        };
        let default_named = || {
            aggregations
                .iter()
                .find(|agg| agg.alias.is_none() && default_name(agg) == name)
                .map(GroupedSortKey::Aggregate)
        };
        aliased
            .map(GroupedSortKey::Aggregate)
            .or_else(group_column)
            .or_else(default_named)
    }
}

/// Whether a vector GROUP BY scores each group by the average of its rows'
/// scores rather than its best row's: when `aggregations` asks for
/// `AVG(score)` and not `MAX(score)`. `ORDER BY similarity()` sorts groups on
/// that score.
#[must_use]
pub fn vector_group_score_is_average(aggregations: &[AggregateFunction]) -> bool {
    let over_score = |function_type: AggregateType| {
        aggregations
            .iter()
            .any(|agg| agg.function_type == function_type && is_score_argument(&agg.argument))
    };
    over_score(AggregateType::Avg) && !over_score(AggregateType::Max)
}

/// The `score` pseudo-column, spelled as the keyword or as a column name in
/// any letter case.
fn is_score_argument(argument: &AggregateArg) -> bool {
    match argument {
        AggregateArg::Score => true,
        AggregateArg::Column(column) => column.eq_ignore_ascii_case("score"),
        AggregateArg::Wildcard => false,
    }
}

/// Validates the ORDER BY of a GROUP BY against its grouped rows (`V016`),
/// with core's names for unaliased aggregates. A vector GROUP BY returns
/// search results, whose ORDER BY may also use `similarity()`, arithmetic and
/// payload fields; only its ORDER BY aggregates are checked against the
/// SELECT list.
pub(super) fn validate_grouped_order_by(stmt: &SelectStatement) -> Result<(), ValidationError> {
    if stmt.is_vector_group_by() {
        stmt.order_by_aggregates_as_fields(AggregateFunction::result_key)?;
    } else if stmt.group_by.is_some() {
        stmt.grouped_sort_keys(AggregateFunction::result_key)?;
    }
    Ok(())
}

fn aggregations(stmt: &SelectStatement) -> &[AggregateFunction] {
    match &stmt.columns {
        SelectColumns::Aggregations(aggregations) | SelectColumns::Mixed { aggregations, .. } => {
            aggregations
        }
        _ => &[],
    }
}

/// A `V016` error naming the unresolved ORDER BY item, which the error's
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
