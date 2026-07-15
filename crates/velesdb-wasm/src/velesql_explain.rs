//! `EXPLAIN` support for the WASM executor (S4-13).
//!
//! # Delegation to the core plan emitter (audit 61b5e8 follow-up R6)
//!
//! Since P1.4 the core `velesql::explain` module compiles without the
//! `persistence` feature, so SELECT/MATCH plans delegate to the single source
//! of truth used by the REST `/query/explain` endpoint —
//! [`QueryPlan::to_plan_steps()`] / `PlanStep::rest_operation()` — instead of
//! re-implementing the step walk from the raw AST. The step rows keep the REST
//! `ExplainStep` wire keys (`step`, `operation`, `description`,
//! `estimated_rows`, `estimation_method`).
//!
//! Only concerns with no core plan node stay local:
//! - **DML/DDL** rows — core plans cover reads only;
//! - **compound** (`UNION`/…) combine rows, folded into `Filter`-class steps;
//! - the leading scan step's **`estimated_rows`** — the WASM store knows its
//!   real row count, where the core plan without collection stats carries none;
//! - the **NEAR description** — WASM searches brute-force, not via an HNSW
//!   index, so the core wording would overstate the execution.

use velesdb_core::velesql::{PlanStep, Query, QueryPlan};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;

/// Builds an EXPLAIN plan as a sequence of synthetic rows (one per step).
pub(crate) fn explain(db: &DatabaseInner, query: &Query) -> Result<Vec<QueryResultRow>, String> {
    step_rows(db, query)
        .into_iter()
        .enumerate()
        .map(|(i, row)| QueryResultRow::synthetic(row.to_json(i + 1)))
        .collect()
}

/// One EXPLAIN row, mirroring the REST `ExplainStep` wire shape.
struct StepRow {
    /// Core `rest_operation()` taxonomy string (e.g. `FullScan`, `Filter`).
    operation: String,
    description: String,
    estimated_rows: Option<u64>,
    estimation_method: Option<String>,
}

impl StepRow {
    /// Serializes the row with the REST `ExplainStep` keys, omitting the
    /// optional estimate fields when absent (matching core's `skip_if_none`).
    fn to_json(&self, step: usize) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("step".to_string(), serde_json::json!(step));
        map.insert("operation".to_string(), serde_json::json!(self.operation));
        map.insert(
            "description".to_string(),
            serde_json::json!(self.description),
        );
        if let Some(rows) = self.estimated_rows {
            map.insert("estimated_rows".to_string(), serde_json::json!(rows));
        }
        if let Some(method) = &self.estimation_method {
            map.insert("estimation_method".to_string(), serde_json::json!(method));
        }
        serde_json::Value::Object(map)
    }

    /// Builds a step with no row estimate (most non-scan operations).
    fn plain(operation: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            description: description.into(),
            estimated_rows: None,
            estimation_method: None,
        }
    }

    /// Maps a core plan step onto the wire row, rewording the NEAR description
    /// (WASM has no HNSW index — the scan is brute-force).
    fn from_core(step: &PlanStep) -> Self {
        let operation = step.rest_operation();
        let description = if operation == "VectorSearch" {
            "ANN search using NEAR clause (brute-force in WASM)".to_string()
        } else {
            step.description.clone()
        };
        Self {
            operation,
            description,
            estimated_rows: step.estimated_rows,
            estimation_method: step.estimation_method.clone(),
        }
    }
}

fn step_rows(db: &DatabaseInner, query: &Query) -> Vec<StepRow> {
    if let Some(dml) = &query.dml {
        return explain_dml(dml);
    }
    if query.ddl.is_some() {
        return vec![StepRow::plain(
            "FullScan",
            "Schema change (DDL) — no scan involved.",
        )];
    }
    let plan = QueryPlan::from_query(query);
    let mut rows: Vec<StepRow> = plan
        .to_plan_steps()
        .iter()
        .map(StepRow::from_core)
        .collect();
    if query.match_clause.is_none() {
        enrich_leading_scan(&mut rows, db, &query.select.from);
    }
    append_compound_rows(&mut rows, query);
    rows
}

/// Injects the WASM store's real row count into the leading scan step. The
/// core plan (built without collection stats) carries no scan estimate.
fn enrich_leading_scan(rows: &mut [StepRow], db: &DatabaseInner, collection: &str) {
    if let Some(scan) = rows
        .first_mut()
        .filter(|r| matches!(r.operation.as_str(), "FullScan" | "VectorSearch"))
    {
        if scan.estimated_rows.is_none() {
            scan.estimated_rows = Some(row_count_hint(db, collection));
            scan.estimation_method = Some("row count".to_string());
        }
    }
}

fn append_compound_rows(rows: &mut Vec<StepRow>, query: &Query) {
    if let Some(compound) = &query.compound {
        for (op, _) in &compound.operations {
            // Set operations have no dedicated core plan node; surface them as a
            // Filter-class combine to stay within the taxonomy.
            rows.push(StepRow::plain(
                "Filter",
                format!("Combine with right-hand SELECT ({op:?})"),
            ));
        }
    }
}

fn explain_dml(dml: &velesdb_core::velesql::DmlStatement) -> Vec<StepRow> {
    use velesdb_core::velesql::DmlStatement;
    // DML has no read-plan vocabulary in core; surface a single FullScan-class
    // row whose description names the mutation.
    let description = match dml {
        DmlStatement::Insert(s) => format!("INSERT INTO {} {} row(s)", s.table, s.rows.len()),
        DmlStatement::Upsert(s) => format!("UPSERT INTO {} {} row(s)", s.table, s.rows.len()),
        DmlStatement::Update(s) => format!("UPDATE {}", s.table),
        DmlStatement::Delete(s) => format!("DELETE FROM {}", s.table),
        DmlStatement::InsertEdge(s) => format!("INSERT EDGE INTO {}", s.collection),
        DmlStatement::DeleteEdge(s) => format!("DELETE EDGE FROM {}", s.collection),
        DmlStatement::SelectEdges(s) => format!("SELECT EDGES FROM {}", s.collection),
        DmlStatement::InsertNode(s) => format!("INSERT NODE INTO {}", s.collection),
        _ => "Unsupported DML variant".to_string(),
    };
    vec![StepRow::plain("FullScan", description)]
}

fn row_count_hint(db: &DatabaseInner, name: &str) -> u64 {
    db.get_shared_store(name)
        .map(|s| s.borrow().ids.len() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "velesql_explain_tests.rs"]
mod tests;
