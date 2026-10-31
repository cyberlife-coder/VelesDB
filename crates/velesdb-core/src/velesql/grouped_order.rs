//! The output columns of an aggregate query, and ORDER BY over them.
//!
//! An aggregate query outputs one field per GROUP BY column and one per SELECT
//! aggregate, the latter under [`AggregateFunction::result_key`] — its alias or
//! default name — in every engine. Two rules hold over those fields, enforced
//! at validation by [`validate_grouped_rows`] so core and the WASM executor
//! refuse the same queries:
//!
//! - **Names are distinct (`V017`).** A field written twice keeps only the
//!   later value: `GROUP BY category, count` beside `COUNT(*)` returned the
//!   count and lost the `count` column.
//! - **An ORDER BY item names one of them (`V016`).** It either names a field
//!   or repeats an aggregate expression (`ORDER BY COUNT(*)`); the parser gives
//!   the repeated expression no alias, so deriving a name from it looked rows
//!   up under `count` while `COUNT(*) AS n` had stored them under `n`, every
//!   comparison came back equal, and LIMIT kept whichever groups came first.
//!   [`SelectStatement::grouped_sort_keys`] and, for a vector GROUP BY,
//!   [`SelectStatement::order_by_aggregates_as_fields`] are the one place an
//!   item is matched to its field.

use std::borrow::Cow;
use std::collections::HashSet;

use super::ast::{
    AggregateArg, AggregateFunction, AggregateType, OrderByExpr, SelectColumns, SelectStatement,
};
use super::validation::{ValidationError, ValidationErrorKind};

impl SelectStatement {
    /// Resolves the ORDER BY of a GROUP BY aggregation to the fields of its
    /// grouped rows it sorts on, in order, each with its direction (`true` is
    /// descending).
    ///
    /// An item that repeats a SELECT aggregate (`ORDER BY COUNT(*)`, whatever
    /// its alias) resolves to that aggregate's field; a name resolves to the
    /// field it is, an aggregate's or a GROUP BY column's — the two never
    /// share a name in a validated query (`V017`). A statement without ORDER
    /// BY resolves to no keys.
    ///
    /// # Errors
    ///
    /// [`ValidationErrorKind::OrderByNotInResult`] (`V016`) for an item that
    /// names no field of the grouped rows: an aggregate the SELECT does not
    /// list, a name that is neither, `similarity()` or an arithmetic
    /// expression.
    pub fn grouped_sort_keys(&self) -> Result<Vec<(String, bool)>, ValidationError> {
        self.order_by
            .iter()
            .flatten()
            .map(|item| Ok((self.grouped_sort_field(&item.expr)?, item.descending)))
            .collect()
    }

    /// For a vector GROUP BY, whose groups come back as search results: this
    /// statement with each `ORDER BY <aggregate>` replaced by the field that
    /// SELECT aggregate is output in, so the search ORDER BY sorts on it. Every
    /// other item — `similarity()`, arithmetic, a field — is the search ORDER
    /// BY's to evaluate. Borrows the statement unchanged when no item is an
    /// aggregate.
    ///
    /// # Errors
    ///
    /// `V016` for an ORDER BY aggregate the SELECT does not list.
    pub fn order_by_aggregates_as_fields(&self) -> Result<Cow<'_, Self>, ValidationError> {
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
                let field = self.order_by_aggregate_target(wanted)?.result_key();
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

    fn grouped_sort_field(&self, expr: &OrderByExpr) -> Result<String, ValidationError> {
        match expr {
            OrderByExpr::Aggregate(wanted) => self
                .order_by_aggregate_target(wanted)
                .map(AggregateFunction::result_key),
            OrderByExpr::Field(name) => {
                let is_aggregate = aggregations(self)
                    .iter()
                    .any(|agg| agg.result_key() == *name);
                if is_aggregate || group_columns(self).any(|column| column == name) {
                    Ok(name.clone())
                } else {
                    Err(not_in_result(
                        name.clone(),
                        "order groups by a GROUP BY column, or by a SELECT aggregate, \
                         its alias or its default name",
                    ))
                }
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

/// Validates the output columns of an aggregate query: distinct names
/// (`V017`), then an ORDER BY that names them (`V016`). A vector GROUP BY
/// returns search results, whose ORDER BY may also use `similarity()`,
/// arithmetic and payload fields; only its ORDER BY aggregates are checked
/// against the SELECT list.
pub(super) fn validate_grouped_rows(stmt: &SelectStatement) -> Result<(), ValidationError> {
    validate_output_names(stmt)?;
    if stmt.is_vector_group_by() {
        stmt.order_by_aggregates_as_fields()?;
    } else if stmt.group_by.is_some() {
        stmt.grouped_sort_keys()?;
    }
    Ok(())
}

/// Refuses (`V017`) an aggregate whose output field repeats another output
/// column's name: a GROUP BY column, a selected column (WASM outputs a
/// selected column outside GROUP BY too), or another aggregate.
fn validate_output_names(stmt: &SelectStatement) -> Result<(), ValidationError> {
    let aggregate_fields: Vec<String> = aggregations(stmt)
        .iter()
        .map(AggregateFunction::result_key)
        .collect();
    let selected_columns = match &stmt.columns {
        SelectColumns::Mixed { columns, .. } | SelectColumns::Columns(columns) => {
            columns.as_slice()
        }
        _ => &[],
    };
    let mut names: HashSet<&str> = group_columns(stmt)
        .map(String::as_str)
        .chain(selected_columns.iter().map(|column| column.name.as_str()))
        .collect();
    match aggregate_fields.iter().find(|field| !names.insert(field)) {
        Some(field) => Err(ValidationError::new(
            ValidationErrorKind::DuplicateOutputColumn,
            None,
            field.clone(),
            format!(
                "two output columns are named '{field}', so one would overwrite the other: \
                 give the aggregate a distinct alias (AS ...)"
            ),
        )),
        None => Ok(()),
    }
}

fn group_columns(stmt: &SelectStatement) -> impl Iterator<Item = &String> {
    stmt.group_by.iter().flat_map(|group_by| &group_by.columns)
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
