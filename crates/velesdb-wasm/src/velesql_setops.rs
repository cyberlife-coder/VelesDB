//! Compound queries (`UNION`, `UNION ALL`, `INTERSECT`, `EXCEPT`) in the WASM
//! executor, with core's semantics: `velesdb_core::velesql::execute_compound`
//! decides the operands, the identity of a row (its projected values), the
//! order and the window the clauses after the last operand put on the
//! combined rows. Each operand runs through the SELECT pipeline
//! ([`crate::velesql_select::execute_operand`]); the combined rows are
//! projected by the first operand's column list, as core's are.

use velesdb_core::velesql::{Query, SelectOrderBy, SelectStatement};

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_result::QueryResultRow;
use crate::velesql_value::Params;

/// Gives each row its own vector, as core's search results carry theirs: a
/// compound's rows come from several collections, whose ids are local, so the
/// window's `similarity(vector, $v)` reads the vector on the row.
pub(crate) fn carry_own_vectors(
    rows: &mut [velesdb_core::SearchResult],
    store: &crate::store_state::StoreState,
) {
    let dim = store.dimension;
    if dim == 0 {
        return;
    }
    let index = crate::velesql_select::build_id_to_idx(store);
    for row in rows {
        if let Some(&at) = index.get(&row.point.id) {
            row.point.vector = store.data[at * dim..(at + 1) * dim].to_vec();
        }
    }
}

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
) -> Result<Vec<QueryResultRow>, ExecError> {
    let higher_is_better = crate::velesql_select::metric_direction(db, &query.select)?;
    // The window ranks by the first operand's metric, as core's; each row
    // carries its own vector (`execute_operand`).
    let metric = db.get_shared_store(&query.select.from)?.borrow().metric;
    let ceiling =
        u64::try_from(crate::guardrails::MAX_INTERMEDIATE_CARDINALITY).unwrap_or(u64::MAX);
    let rows = velesdb_core::velesql::execute_compound(
        query,
        higher_is_better,
        ceiling,
        |operand| {
            let inputs =
                velesdb_core::velesql::compound_rank_inputs(query, &operand.select.columns);
            crate::velesql_select::execute_operand(db, operand, params, &inputs)
        },
        |rows, order_by| {
            let window = ordered_by(&query.select, order_by);
            let ctx = crate::velesql_orderby::OrderContext {
                higher_is_better,
                vectors: Some(crate::velesql_orderby::OrderVectors {
                    params,
                    metric,
                    store: None,
                }),
            };
            crate::velesql_orderby::sort_rows(&window, rows, &ctx)
        },
    )?;
    crate::velesql_project::project_compound(query, &rows)
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
