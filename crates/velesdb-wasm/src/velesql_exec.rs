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
//! | `FUSION` (rrf, weighted, maximum)           | full             |
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
    let rows = dispatch(db, &parsed, &params)?;
    Ok(QueryResult::from_parts(kind, rows))
}

/// Dispatches the parsed query to the appropriate sub-module.
fn dispatch(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if query.is_match_query() {
        return velesql_graph::execute_match(db, query, params);
    }
    if let Some(dml) = &query.dml {
        return dispatch_dml(db, dml, params);
    }
    if let Some(ddl) = &query.ddl {
        return velesql_ddl::execute(db, ddl);
    }
    if let Some(intro) = &query.introspection {
        return velesql_introspection::execute(db, intro);
    }
    if let Some(admin) = &query.admin {
        velesql_admin::execute(db, admin)?;
        return Ok(Vec::new());
    }
    if let Some(compound) = &query.compound {
        return velesql_setops::execute(db, query, compound, params);
    }
    // Default path: a SELECT (no explicit DML/DDL/introspection/admin).
    velesql_select::execute(db, query, params)
}

/// Dispatches DML statements (INSERT / UPDATE / DELETE / graph mutations).
fn dispatch_dml(
    db: &mut DatabaseInner,
    dml: &DmlStatement,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    match dml {
        DmlStatement::Insert(s) | DmlStatement::Upsert(s) => {
            let n = velesql_insert::execute(db, s, params)?;
            Ok(synthesize_count_rows(n))
        }
        DmlStatement::Update(s) => {
            let n = velesql_update::execute(db, s, params)?;
            Ok(synthesize_count_rows(n))
        }
        DmlStatement::Delete(s) => {
            let n = velesql_delete::execute(db, s, params)?;
            Ok(synthesize_count_rows(n))
        }
        DmlStatement::InsertEdge(s) => {
            velesql_graph::insert_edge(db, s, params)?;
            Ok(synthesize_count_rows(1))
        }
        DmlStatement::DeleteEdge(s) => {
            let n = velesql_graph::delete_edge(db, s)?;
            Ok(synthesize_count_rows(n))
        }
        DmlStatement::SelectEdges(s) => velesql_graph::select_edges(db, s, params),
        DmlStatement::InsertNode(s) => {
            velesql_graph::insert_node(db, s)?;
            Ok(synthesize_count_rows(1))
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

/// Synthesizes count rows for INSERT / UPDATE / DELETE statement results.
fn synthesize_count_rows(count: u32) -> Vec<QueryResultRow> {
    (0..count)
        .map(|_| {
            QueryResultRow::build(0, 0.0, None).unwrap_or_else(|_| {
                QueryResultRow::synthetic(serde_json::json!({})).unwrap_or_else(|_| {
                    QueryResultRow::synthetic(serde_json::Value::Null)
                        .expect("JSON null serializes")
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseInner;

    fn new_db_with_metadata() -> DatabaseInner {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("docs").expect("test: create");
        db
    }

    #[test]
    fn test_execute_create_collection() {
        let mut db = DatabaseInner::new();
        let r = execute(
            &mut db,
            "CREATE COLLECTION vecs (dimension = 4, metric = 'cosine')",
            None,
        )
        .expect("test: ddl");
        assert_eq!(r.kind(), "ddl");
        assert!(db.contains("vecs"));
    }

    #[test]
    fn test_execute_invalid_sql_returns_error() {
        let mut db = DatabaseInner::new();
        let err = execute(&mut db, "NOT VALID AT ALL", None);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("parse error"));
    }

    #[test]
    fn test_execute_rejects_train() {
        let mut db = new_db_with_metadata();
        let err = execute(&mut db, "TRAIN QUANTIZER ON docs WITH (type = 'sq8')", None);
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("TRAIN"));
    }

    #[test]
    fn test_execute_accepts_union() {
        let mut db = new_db_with_metadata();
        // Empty union should return 0 rows rather than erroring.
        let r = execute(&mut db, "SELECT * FROM docs UNION SELECT * FROM docs", None)
            .expect("test: union");
        assert_eq!(r.row_count(), 0);
    }
}
