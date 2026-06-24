//! Query execution logic for the REPL.
//!
//! Extracted from `repl.rs` to keep module size under 500 NLOC.

use anyhow::Result;
use instant::Instant;
use std::collections::HashMap;
use velesdb_core::Database;

use crate::repl::{QueryKind, QueryResult};
use crate::session::SessionSettings;

/// Execute a `VelesQL` query and return results.
///
/// Delegates to [`Database::execute_query`] for SELECT/DML/TRAIN queries.
/// MATCH queries are routed through [`route_match_query`], which also goes
/// through [`Database::execute_query`] so cross-collection `@collection`
/// annotations are enriched (the active collection set via `.use` is injected
/// as the `_collection` param when the query has no explicit `FROM`).
pub fn execute_query(
    db: &Database,
    query: &str,
    active_collection: Option<&str>,
    session: Option<&SessionSettings>,
) -> Result<QueryResult> {
    let start = Instant::now();

    // Parse the query
    let mut parsed = velesdb_core::velesql::Parser::parse(query)
        .map_err(|e| anyhow::anyhow!("Parse error: {}", e.message))?;

    // $parameter vectors can't be supplied from the REPL (use the REST API).
    // Returning Err (not Ok-empty) makes the REPL print a red error and scripts
    // exit non-zero instead of silently treating 0 rows as success.
    if has_param_vector(&parsed) {
        return Err(param_vector_unsupported_error());
    }

    // Apply REPL session settings (mode/ef_search into the AST WITH-options,
    // max_results as a LIMIT cap). Inline overrides always win.
    if let Some(session) = session {
        apply_session_settings(&mut parsed, session);
    }

    // Aggregation (GROUP BY / COUNT / SUM / ...) routes through the dedicated
    // aggregate engine; the standard SELECT projection path returns empty rows
    // for aggregate columns, so without this the REPL would print raw rows.
    if !parsed.is_match_query() && parsed.select.is_aggregation_query() {
        return run_aggregation_query(db, &parsed, active_collection, start);
    }

    run_row_query(db, &parsed, active_collection, start)
}

/// Applies REPL session settings to a parsed query before execution.
///
/// `mode` and an explicit `ef_search` are injected into the SELECT `WITH`
/// options (only when not already present — an inline `WITH(...)` always wins),
/// and `max_results` caps the effective `LIMIT`. Match/aggregate queries are not
/// touched (those clauses only steer the vector-search pipeline).
pub fn apply_session_settings(
    parsed: &mut velesdb_core::velesql::Query,
    session: &SessionSettings,
) {
    if parsed.is_match_query() {
        return;
    }
    inject_session_with_options(&mut parsed.select, session);
    cap_limit_to_max_results(&mut parsed.select, session.max_results());
}

/// Injects the session `mode`/`ef_search` into the SELECT `WITH` options,
/// preserving any inline override (inline wins).
fn inject_session_with_options(
    select: &mut velesdb_core::velesql::SelectStatement,
    session: &SessionSettings,
) {
    use velesdb_core::velesql::{WithClause, WithValue};

    let with = select.with_clause.get_or_insert_with(WithClause::new);
    if with.get_mode().is_none() {
        with.options.push(velesdb_core::velesql::WithOption {
            key: "mode".to_string(),
            value: WithValue::String(session.mode_str()),
        });
    }
    if let Some(ef) = session.ef_search() {
        if with.get_ef_search().is_none() {
            with.options.push(velesdb_core::velesql::WithOption {
                key: "ef_search".to_string(),
                value: WithValue::Integer(i64::try_from(ef).unwrap_or(i64::MAX)),
            });
        }
    }
}

/// Caps the effective `LIMIT` at the session `max_results`: a missing `LIMIT`
/// becomes `max_results`, and a larger explicit `LIMIT` is reduced to it.
fn cap_limit_to_max_results(
    select: &mut velesdb_core::velesql::SelectStatement,
    max_results: usize,
) {
    let cap = max_results as u64;
    select.limit = Some(select.limit.map_or(cap, |l| l.min(cap)));
}

/// Returns `true` when a SELECT or MATCH `WHERE` references a `$parameter`
/// vector, which the REPL cannot supply (the user must use the REST API).
fn has_param_vector(parsed: &velesdb_core::velesql::Query) -> bool {
    parsed
        .select
        .where_clause
        .as_ref()
        .is_some_and(contains_param_vector)
        || parsed
            .match_clause
            .as_ref()
            .and_then(|m| m.where_clause.as_ref())
            .is_some_and(contains_param_vector)
}

/// The error returned when a query needs a `$parameter` vector the REPL cannot
/// supply. Surfaced as an `Err` so the REPL prints it red and scripts exit
/// non-zero rather than seeing an empty (silently-successful) result.
fn param_vector_unsupported_error() -> anyhow::Error {
    anyhow::anyhow!(
        "Vector search with $parameter requires the REST API. \
         Use literal vectors or metadata-only queries."
    )
}

/// Runs a row-returning query (MATCH, plain SELECT, or DML/DDL) and builds the
/// display rows. Plain `SELECT`s are projected through the core projection
/// engine (column selection, `AS` aliases, window functions) for parity with the
/// REST `/query` API; MATCH and DML/DDL keep the raw id+score+payload rendering.
fn run_row_query(
    db: &Database,
    parsed: &velesdb_core::velesql::Query,
    active_collection: Option<&str>,
    start: Instant,
) -> Result<QueryResult> {
    let kind = query_kind(parsed);
    let rows = if parsed.is_match_query() {
        let results = route_match_query(db, parsed, active_collection)?;
        results.into_iter().map(result_to_row).collect()
    } else {
        let results = db
            .execute_query(parsed, &HashMap::new())
            .map_err(|e| anyhow::anyhow!("Query error: {e}"))?;
        if matches!(kind, QueryKind::Select) {
            project_select_rows(&results, &parsed.select.columns)
        } else {
            results.into_iter().map(result_to_row).collect()
        }
    };
    Ok(QueryResult {
        rows,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        kind,
    })
}

/// Projects plain `SELECT` results through the core projection engine, matching
/// the REST API (only the requested columns, with `AS` aliases and window
/// functions) instead of always emitting id+score+full payload.
fn project_select_rows(
    results: &[velesdb_core::SearchResult],
    columns: &velesdb_core::velesql::SelectColumns,
) -> Vec<HashMap<String, serde_json::Value>> {
    velesdb_core::collection::search::query::projection::project_results(results, columns)
        .into_iter()
        .map(json_object_to_row)
        .collect()
}

/// Determine the [`QueryKind`] of a parsed query for display purposes.
fn query_kind(parsed: &velesdb_core::velesql::Query) -> QueryKind {
    if parsed.is_ddl_query() {
        QueryKind::Ddl
    } else if parsed.is_introspection_query() {
        QueryKind::Introspection
    } else if parsed.is_admin_query() {
        QueryKind::Admin
    } else if parsed.is_dml_query() {
        QueryKind::Dml
    } else if parsed.is_train() {
        QueryKind::Train
    } else {
        QueryKind::Select
    }
}

/// Route a MATCH query through [`Database::execute_query`] so cross-collection
/// `@collection` annotations are enriched.
///
/// `Database::execute_query` resolves the target collection from the query's
/// `FROM` clause or the `_collection` param, then merges payloads from any
/// `@collection`-annotated node patterns. The REPL selects the target via
/// `.use <collection>`, so when the query has no explicit `FROM` we inject the
/// active collection as `_collection`. Resolution covers graph, vector, and
/// metadata collections alike.
fn route_match_query(
    db: &Database,
    parsed: &velesdb_core::velesql::Query,
    active_collection: Option<&str>,
) -> Result<Vec<velesdb_core::SearchResult>> {
    let params = params_with_active_collection(
        parsed,
        active_collection,
        "MATCH queries require an active collection. Use: .use <collection_name>",
    )?;
    db.execute_query(parsed, &params)
        .map_err(|e| anyhow::anyhow!("Query error: {e}"))
}

/// Builds the params map for a query, injecting the active collection as the
/// `_collection` key when the query has no explicit `FROM` (the REPL selects the
/// target via `.use <collection>`). `requires_msg` is the error shown when no
/// active collection is set.
fn params_with_active_collection(
    parsed: &velesdb_core::velesql::Query,
    active_collection: Option<&str>,
    requires_msg: &str,
) -> Result<HashMap<String, serde_json::Value>> {
    let mut params = HashMap::new();
    if parsed.select.from.is_empty() {
        let col_name = active_collection.ok_or_else(|| anyhow::anyhow!("{requires_msg}"))?;
        params.insert(
            "_collection".to_string(),
            serde_json::Value::String(col_name.to_string()),
        );
    }
    Ok(params)
}

/// Routes a `GROUP BY` / scalar-aggregate SELECT through the aggregate engine and
/// renders the JSON result (object or array-of-groups) into display rows.
fn run_aggregation_query(
    db: &Database,
    parsed: &velesdb_core::velesql::Query,
    active_collection: Option<&str>,
    start: Instant,
) -> Result<QueryResult> {
    let params = params_with_active_collection(
        parsed,
        active_collection,
        "Aggregation queries require an active collection. Use: .use <collection_name>",
    )?;
    let value = db
        .execute_aggregate(parsed, &params)
        .map_err(|e| anyhow::anyhow!("Query error: {e}"))?;
    Ok(QueryResult {
        rows: aggregate_value_to_rows(value),
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        kind: QueryKind::Select,
    })
}

/// Converts the JSON from [`Database::execute_aggregate`] into display rows: a
/// top-level array yields one row per group; any other value is a single row.
fn aggregate_value_to_rows(value: serde_json::Value) -> Vec<HashMap<String, serde_json::Value>> {
    match value {
        serde_json::Value::Array(items) => items.into_iter().map(json_object_to_row).collect(),
        other => vec![json_object_to_row(other)],
    }
}

/// Flattens a JSON object into a display row; non-objects map to a `value` cell.
fn json_object_to_row(value: serde_json::Value) -> HashMap<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        other => HashMap::from([("value".to_string(), other)]),
    }
}

/// Convert a single [`velesdb_core::SearchResult`] into a display row.
fn result_to_row(r: velesdb_core::SearchResult) -> HashMap<String, serde_json::Value> {
    let mut row = HashMap::new();
    row.insert("id".to_string(), serde_json::json!(r.point.id));
    row.insert("score".to_string(), serde_json::json!(r.score));

    if let Some(serde_json::Value::Object(map)) = &r.point.payload {
        for (k, v) in map {
            row.insert(k.clone(), v.clone());
        }
    }
    row
}

pub(crate) fn contains_param_vector(condition: &velesdb_core::velesql::Condition) -> bool {
    use velesdb_core::velesql::{Condition, SparseVectorExpr, VectorExpr};
    match condition {
        Condition::VectorSearch(vs) => matches!(vs.vector, VectorExpr::Parameter(_)),
        Condition::VectorFusedSearch(vfs) => vfs
            .vectors
            .iter()
            .any(|v| matches!(v, VectorExpr::Parameter(_))),
        Condition::SparseVectorSearch(svs) => {
            matches!(svs.vector, SparseVectorExpr::Parameter(_))
        }
        Condition::Similarity(sim) => matches!(sim.vector, VectorExpr::Parameter(_)),
        Condition::And(left, right) | Condition::Or(left, right) => {
            contains_param_vector(left) || contains_param_vector(right)
        }
        Condition::Not(inner) | Condition::Group(inner) => contains_param_vector(inner),
        Condition::Comparison(_)
        | Condition::In(_)
        | Condition::Between(_)
        | Condition::Like(_)
        | Condition::IsNull(_)
        | Condition::Match(_)
        | Condition::GraphMatch(_)
        | Condition::Contains(_)
        | Condition::ContainsText(_)
        | Condition::GeoDistance(_)
        | Condition::GeoBbox(_) => false,
        _ => false,
    }
}

#[cfg(test)]
#[path = "repl_execute_tests.rs"]
mod repl_execute_tests;
