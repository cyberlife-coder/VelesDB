//! Compound queries (`UNION`, `UNION ALL`, `INTERSECT`, `EXCEPT`) in the WASM
//! executor, with core's semantics: `velesdb_core::velesql::execute_compound`
//! decides the operands, the identity of a row (its projected values), the
//! order and the window the clauses after the last operand put on the
//! combined rows. Each operand runs through the SELECT pipeline
//! ([`crate::velesql_select::execute_operand`]); the combined rows are
//! projected by the first operand's column list, as core's are.

use velesdb_core::velesql::{Query, SelectOrderBy, SelectStatement};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Executes a compound query.
///
/// # Errors
///
/// The first error an operand returns, or an `ORDER BY` the window cannot
/// evaluate.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let higher_is_better = crate::velesql_select::metric_direction(db, &query.select)?;
    let ceiling =
        u64::try_from(crate::guardrails::MAX_INTERMEDIATE_CARDINALITY).unwrap_or(u64::MAX);
    let rows = velesdb_core::velesql::execute_compound(
        query,
        higher_is_better,
        ceiling,
        |operand| crate::velesql_select::execute_operand(db, operand, params),
        |rows, order_by| {
            let window = ordered_by(&query.select, order_by);
            crate::velesql_orderby::sort_rows(&window, rows, higher_is_better)
        },
    )?;
    crate::velesql_project::project(&query.select, &rows)
}

/// `stmt` ordered by `order_by`: the statement the window's ORDER BY is
/// evaluated against.
fn ordered_by(stmt: &SelectStatement, order_by: &[SelectOrderBy]) -> SelectStatement {
    SelectStatement {
        order_by: Some(order_by.to_vec()),
        ..stmt.clone()
    }
}

#[cfg(test)]
#[path = "velesql_setops_tests.rs"]
mod tests;
