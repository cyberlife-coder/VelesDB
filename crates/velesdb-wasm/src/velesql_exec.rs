//! Top-level VelesQL executor for WASM (S4-13).
//!
//! Parses a VelesQL statement, resolves its JSON parameters, and dispatches
//! to the correct sub-module (DDL, DML, SELECT, introspection, admin, graph).
//! Returns a unified [`QueryResult`] whose shape mirrors the mobile surface
//! so JavaScript and Swift/Kotlin clients see a single, consistent API.
//!
//! # Supported statement matrix (updated for pre-seed demo parity)
//!
//! | Statement                                   | Supported        |
//! |---------------------------------------------|:----------------:|
//! | `SELECT * FROM coll [WHERE …] [LIMIT]`      | full             |
//! | `SELECT … WHERE vector NEAR $v LIMIT k`     | full             |
//! | `SELECT DISTINCT col FROM coll`             | full             |
//! | `SELECT COUNT/SUM/AVG/MIN/MAX`              | full             |
//! | `GROUP BY ... HAVING ...`                   | full             |
//! | `ORDER BY ... ASC/DESC` (any column, multi) | full             |
//! | `UNION [ALL] / INTERSECT / EXCEPT`          | full             |
//! | `JOIN` (INNER, LEFT) + `ON ...=...`         | full             |
//! | `WHERE similarity(v, $q) > t`               | full             |
//! | `FUSION` over `NEAR_FUSED` (rrf/max/avg)    | full             |
//! | `FUSION` over single-vector `NEAR` (rrf)    | metadata = hard filter; clause is a no-op |
//! | `FUSION` weighted / rsf over single `NEAR`  | **rejected** (no second scored branch) |
//! | `INSERT / UPSERT INTO coll (…) VALUES`      | full             |
//! | `UPDATE coll SET … WHERE …`                 | full             |
//! | `DELETE FROM coll WHERE …`                  | full             |
//! | `CREATE COLLECTION / DROP / TRUNCATE`       | full             |
//! | `CREATE INDEX / DROP INDEX`                 | **accepted no-op** (brute-force; explains expected behaviour) |
//! | `ANALYZE coll`                              | **accepted no-op** (returns synthetic stats) |
//! | `EXPLAIN <query>`                           | full (synth plan) |
//! | `SHOW COLLECTIONS`                          | full             |
//! | `DESCRIBE COLLECTION <name>`                | full             |
//! | `FLUSH [FULL] [coll]`                       | full (no-op)     |
//! | `INSERT / DELETE / SELECT EDGES`            | full (in-memory graph) |
//! | `INSERT NODE`                               | full             |
//! | `MATCH (a:L)-[:R]->(b:L2)` 1- & 2-hop       | full             |
//! | `TRAIN QUANTIZER`                           | rejected (requires persistence feature) |
//! | multi-hop MATCH beyond 2 hops               | rejected with clear message              |
//!
//! Rejected statements surface a descriptive error mentioning the unsupported
//! feature so JavaScript callers can react without parsing the SQL themselves.

use velesdb_core::velesql::{DmlStatement, Parser, Query};

use crate::database::DatabaseInner;
use crate::velesql_result::{classify_query, QueryResult, QueryResultRow};
use crate::velesql_value::{parse_params, Params};
use crate::{
    velesql_admin, velesql_ddl, velesql_delete, velesql_graph, velesql_insert,
    velesql_introspection, velesql_select, velesql_setops, velesql_update,
};

/// Executes a VelesQL query against the database.
///
/// Returns a [`QueryResult`] on success. Errors surface as `String` so the
/// FFI boundary can map them to `JsValue` without re-parsing.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    sql: &str,
    params_json: Option<&str>,
) -> Result<QueryResult, String> {
    let parsed = Parser::parse(sql).map_err(|e| {
        format!(
            "VelesQL parse error at position {}: {} (near '{}')",
            e.position, e.message, e.fragment
        )
    })?;
    let params = parse_params(params_json)?;

    reject_unsupported_top_level(&parsed)?;

    let kind = classify_query(&parsed);
    match dispatch(db, &parsed, &params)? {
        DispatchOutcome::Rows(rows) => Ok(QueryResult::from_parts(kind, rows)),
        // Finding F13: DML paths carry a count without placeholder rows,
        // saving O(n) allocations per mutation.
        DispatchOutcome::Mutation(count) => Ok(QueryResult::from_mutation(kind, count)),
    }
}

/// Outcome of dispatching a parsed query to its sub-module.
///
/// `Rows` is returned for row-producing statements (SELECT, SHOW,
/// DESCRIBE, MATCH, SELECT EDGES, DDL-noop rows, ANALYZE synthetic rows).
/// `Mutation` is returned for row-affecting statements so the executor
/// can build a placeholder-free [`QueryResult`] (Finding F13).
enum DispatchOutcome {
    Rows(Vec<QueryResultRow>),
    Mutation(u32),
}

/// Dispatches the parsed query to the appropriate sub-module.
fn dispatch(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<DispatchOutcome, String> {
    if query.is_match_query() {
        return velesql_graph::execute_match(db, query, params).map(DispatchOutcome::Rows);
    }
    if let Some(dml) = &query.dml {
        return dispatch_dml(db, dml, params);
    }
    if let Some(ddl) = &query.ddl {
        return velesql_ddl::execute(db, ddl).map(DispatchOutcome::Rows);
    }
    if let Some(intro) = &query.introspection {
        return velesql_introspection::execute(db, intro).map(DispatchOutcome::Rows);
    }
    if let Some(admin) = &query.admin {
        velesql_admin::execute(db, admin)?;
        return Ok(DispatchOutcome::Rows(Vec::new()));
    }
    if let Some(compound) = &query.compound {
        return velesql_setops::execute(db, query, compound, params).map(DispatchOutcome::Rows);
    }
    // Default path: a SELECT (no explicit DML/DDL/introspection/admin).
    velesql_select::execute(db, query, params).map(DispatchOutcome::Rows)
}

/// Dispatches DML statements (INSERT / UPDATE / DELETE / graph mutations).
///
/// All variants except `SelectEdges` yield a mutation count; `SelectEdges`
/// is a row-producing DML statement and propagates its rows unchanged.
fn dispatch_dml(
    db: &mut DatabaseInner,
    dml: &DmlStatement,
    params: &Params,
) -> Result<DispatchOutcome, String> {
    match dml {
        DmlStatement::Insert(s) | DmlStatement::Upsert(s) => {
            let n = velesql_insert::execute(db, s, params)?;
            Ok(DispatchOutcome::Mutation(n))
        }
        DmlStatement::Update(s) => {
            let n = velesql_update::execute(db, s, params)?;
            Ok(DispatchOutcome::Mutation(n))
        }
        DmlStatement::Delete(s) => {
            let n = velesql_delete::execute(db, s, params)?;
            Ok(DispatchOutcome::Mutation(n))
        }
        DmlStatement::InsertEdge(s) => {
            velesql_graph::insert_edge(db, s, params)?;
            Ok(DispatchOutcome::Mutation(1))
        }
        DmlStatement::DeleteEdge(s) => {
            let n = velesql_graph::delete_edge(db, s)?;
            Ok(DispatchOutcome::Mutation(n))
        }
        DmlStatement::SelectEdges(s) => {
            velesql_graph::select_edges(db, s, params).map(DispatchOutcome::Rows)
        }
        DmlStatement::InsertNode(s) => {
            velesql_graph::insert_node(db, s)?;
            Ok(DispatchOutcome::Mutation(1))
        }
        _ => Err(format!("Unsupported DML variant in WASM: {dml:?}")),
    }
}

/// Rejects top-level features that the executor cannot handle yet.
fn reject_unsupported_top_level(query: &Query) -> Result<(), String> {
    if query.is_train() {
        return Err(
            "TRAIN QUANTIZER requires the persistence feature; not available in WASM".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "velesql_exec_unit_tests.rs"]
mod tests;
