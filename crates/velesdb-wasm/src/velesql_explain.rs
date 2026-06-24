//! `EXPLAIN` support for the WASM executor (S4-13).
//!
//! WASM has no cost-based optimizer, but consumers still benefit from a
//! human-readable plan. This module walks the parsed AST and produces one
//! row per logical pipeline step, returning them as synthetic rows so the
//! JavaScript caller can render them in an "explain plan" table.
//!
//! # Vocabulary parity with core (backlog #23)
//!
//! The core `QueryPlan::to_plan_steps()` / `PlanStep::rest_operation()` plan
//! emitter — the single source of truth used by the REST `/query/explain`
//! endpoint — lives behind the `persistence` feature, which WASM never
//! enables (it pulls in `tokio`/`memmap2`/`rayon`). It is therefore
//! unreachable from this crate. Rather than re-export divergent labels, the
//! step rows below emit the SAME wire keys as the REST `ExplainStep`
//! (`step`, `operation`, `description`, `estimated_rows`, `estimation_method`)
//! and the SAME `operation` vocabulary as
//! [`PlanStepKind`](velesdb_core::velesql) /`rest_operation` —
//! `VectorSearch`, `FullScan`, `Filter`, `{Type}Join`, `GroupBy`, `Aggregate`,
//! `Sort`, `Limit`, `Offset`, `MatchTraversal`. WASM-only concerns with no core
//! plan node (FUSION, DISTINCT) are folded into a step's description instead of
//! inventing an out-of-taxonomy `operation`.

use velesdb_core::velesql::{Query, SelectStatement};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;

/// Builds an EXPLAIN plan as a sequence of synthetic rows (one per step).
pub(crate) fn explain(db: &DatabaseInner, query: &Query) -> Result<Vec<QueryResultRow>, String> {
    let steps = plan_steps(db, query);
    steps
        .into_iter()
        .enumerate()
        .map(|(i, step)| QueryResultRow::synthetic(step.to_json(i + 1)))
        .collect()
}

/// One EXPLAIN row, mirroring the REST `ExplainStep` wire shape.
struct PlanStep {
    /// Core `rest_operation()` taxonomy string (e.g. `FullScan`, `Filter`).
    operation: String,
    description: String,
    estimated_rows: Option<usize>,
    estimation_method: Option<&'static str>,
}

impl PlanStep {
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
        if let Some(method) = self.estimation_method {
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
}

fn plan_steps(db: &DatabaseInner, query: &Query) -> Vec<PlanStep> {
    if let Some(match_clause) = &query.match_clause {
        return explain_match(match_clause);
    }
    if let Some(dml) = &query.dml {
        return explain_dml(dml);
    }
    if query.ddl.is_some() {
        return vec![PlanStep::plain(
            "FullScan",
            "Schema change (DDL) — no scan involved.",
        )];
    }
    explain_select(db, &query.select, query)
}

fn explain_select(db: &DatabaseInner, stmt: &SelectStatement, query: &Query) -> Vec<PlanStep> {
    let mut steps = vec![scan_step(db, stmt)];
    append_join_steps(&mut steps, db, stmt);
    append_filter_step(&mut steps, stmt);
    append_group_steps(&mut steps, stmt);
    append_post_scan_steps(&mut steps, stmt);
    append_compound_steps(&mut steps, query);
    steps
}

/// Builds the leading scan step: `VectorSearch` when the WHERE carries a NEAR
/// clause (mirroring core's vector-search node), otherwise `FullScan`.
fn scan_step(db: &DatabaseInner, stmt: &SelectStatement) -> PlanStep {
    let rows = row_count_hint(db, &stmt.from);
    let has_vector = stmt
        .where_clause
        .as_ref()
        .is_some_and(velesdb_core::velesql::Condition::has_vector_search);
    let (operation, description) = if has_vector {
        (
            "VectorSearch",
            "ANN search using NEAR clause (brute-force in WASM)".to_string(),
        )
    } else {
        ("FullScan", format!("Scan collection '{}'", stmt.from))
    };
    PlanStep {
        operation: operation.to_string(),
        description,
        estimated_rows: Some(rows),
        estimation_method: Some("row count"),
    }
}

fn append_join_steps(steps: &mut Vec<PlanStep>, db: &DatabaseInner, stmt: &SelectStatement) {
    for join in &stmt.joins {
        let right_count = row_count_hint(db, &join.table);
        // `{Type}Join` reproduces core `rest_operation()` for a Join node.
        steps.push(PlanStep::plain(
            format!("{:?}Join", join.join_type),
            format!(
                "Join with '{}' ({right_count} rows) ON equality",
                join.table
            ),
        ));
    }
}

fn append_filter_step(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if stmt.where_clause.is_some() {
        steps.push(PlanStep::plain("Filter", "Apply WHERE clause predicates"));
    }
}

fn append_group_steps(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if stmt.group_by.is_some() {
        steps.push(PlanStep::plain(
            "GroupBy",
            "Group rows by specified columns",
        ));
    }
    if stmt.is_aggregation_query() {
        steps.push(PlanStep::plain(
            "Aggregate",
            "Compute aggregate functions (COUNT, SUM, etc.)",
        ));
    }
    if stmt.having.is_some() {
        // HAVING has no dedicated core plan node; it is a predicate filter over
        // grouped rows, so it surfaces as a `Filter` within the taxonomy.
        steps.push(PlanStep::plain(
            "Filter",
            "Filter groups on HAVING predicate",
        ));
    }
}

fn append_post_scan_steps(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if stmt.order_by.is_some() {
        steps.push(PlanStep::plain("Sort", sort_description(stmt)));
    }
    append_pagination_steps(steps, stmt);
}

/// Sort description, folding the WASM-only DISTINCT/FUSION notes in (neither has
/// a dedicated core plan node, so they are not standalone operations).
fn sort_description(stmt: &SelectStatement) -> String {
    let mut desc = "Sort results by ORDER BY clause".to_string();
    if stmt.fusion_clause.is_some() {
        desc.push_str(" (after FUSION combine)");
    }
    if matches!(stmt.distinct, velesdb_core::velesql::DistinctMode::All) {
        desc.push_str(" (DISTINCT applied)");
    }
    desc
}

/// Emits `Limit` (folding any OFFSET into its description) or a standalone
/// `Offset`, mirroring core's pagination folding.
fn append_pagination_steps(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    match (stmt.limit, stmt.offset) {
        (Some(limit), offset) => {
            let off = offset.unwrap_or(0);
            let mut step = PlanStep::plain("Limit", format!("Apply LIMIT {limit} OFFSET {off}"));
            step.estimated_rows = usize::try_from(limit).ok();
            steps.push(step);
        }
        (None, Some(offset)) => {
            steps.push(PlanStep::plain(
                "Offset",
                format!("Skip {offset} rows (OFFSET)"),
            ));
        }
        (None, None) => {}
    }
}

fn append_compound_steps(steps: &mut Vec<PlanStep>, query: &Query) {
    if let Some(compound) = &query.compound {
        for (op, _) in &compound.operations {
            // Set operations have no dedicated core plan node; surface them as a
            // Filter-class combine to stay within the taxonomy.
            steps.push(PlanStep::plain(
                "Filter",
                format!("Combine with right-hand SELECT ({op:?})"),
            ));
        }
    }
}

fn explain_dml(dml: &velesdb_core::velesql::DmlStatement) -> Vec<PlanStep> {
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
    vec![PlanStep::plain("FullScan", description)]
}

fn explain_match(clause: &velesdb_core::velesql::MatchClause) -> Vec<PlanStep> {
    let node_count: usize = clause.patterns.iter().map(|p| p.nodes.len()).sum();
    let rel_count: usize = clause.patterns.iter().map(|p| p.relationships.len()).sum();
    vec![PlanStep::plain(
        "MatchTraversal",
        format!("Graph traversal: {node_count} node pattern(s), {rel_count} relationship(s), RETURN {} item(s)",
            clause.return_clause.items.len()),
    )]
}

fn row_count_hint(db: &DatabaseInner, name: &str) -> usize {
    db.get_shared_store(name)
        .map(|s| s.borrow().ids.len())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "velesql_explain_tests.rs"]
mod tests;
