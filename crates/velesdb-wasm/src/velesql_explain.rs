//! `EXPLAIN` support for the WASM executor (S4-13).
//!
//! WASM has no cost-based optimizer, but consumers still benefit from a
//! human-readable plan. This module walks the parsed AST and produces one
//! row per logical pipeline step, returning them as synthetic rows so the
//! JavaScript caller can render them in an "explain plan" table.

use velesdb_core::velesql::{Query, SelectStatement};

use crate::database::DatabaseInner;
use crate::velesql_result::QueryResultRow;

/// Builds an EXPLAIN plan as a sequence of synthetic rows (one per step).
pub(crate) fn explain(db: &DatabaseInner, query: &Query) -> Result<Vec<QueryResultRow>, String> {
    let steps = plan_steps(db, query);
    steps
        .into_iter()
        .enumerate()
        .map(|(i, step)| {
            let payload = serde_json::json!({
                "step": i + 1,
                "node": step.node,
                "detail": step.detail,
            });
            QueryResultRow::synthetic(payload)
        })
        .collect()
}

struct PlanStep {
    node: String,
    detail: String,
}

fn plan_steps(db: &DatabaseInner, query: &Query) -> Vec<PlanStep> {
    if let Some(match_clause) = &query.match_clause {
        return explain_match(match_clause);
    }
    if let Some(dml) = &query.dml {
        return explain_dml(dml);
    }
    if query.ddl.is_some() {
        return vec![PlanStep {
            node: "DDL".to_string(),
            detail: "Schema change — no scan involved.".to_string(),
        }];
    }
    explain_select(db, &query.select, query)
}

fn explain_select(db: &DatabaseInner, stmt: &SelectStatement, query: &Query) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    steps.push(PlanStep {
        node: "Scan".to_string(),
        detail: format!(
            "Scan {} ({} rows)",
            stmt.from,
            row_count_hint(db, &stmt.from)
        ),
    });
    append_join_steps(&mut steps, db, stmt);
    append_filter_step(&mut steps, stmt);
    append_group_steps(&mut steps, stmt);
    append_post_scan_steps(&mut steps, stmt);
    append_compound_steps(&mut steps, query);
    steps
}

fn append_join_steps(steps: &mut Vec<PlanStep>, db: &DatabaseInner, stmt: &SelectStatement) {
    for join in &stmt.joins {
        let right_count = row_count_hint(db, &join.table);
        steps.push(PlanStep {
            node: "NestedLoopJoin".to_string(),
            detail: format!(
                "{:?} JOIN {} ({right_count} rows) ON equality",
                join.join_type, join.table
            ),
        });
    }
}

fn append_filter_step(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if let Some(cond) = &stmt.where_clause {
        steps.push(PlanStep {
            node: "Filter".to_string(),
            detail: format!("WHERE {cond:?}"),
        });
    }
}

fn append_group_steps(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if stmt.group_by.is_some() {
        steps.push(PlanStep {
            node: "GroupBy".to_string(),
            detail: "Hash-group rows by GROUP BY columns.".to_string(),
        });
    }
    if stmt.having.is_some() {
        steps.push(PlanStep {
            node: "HavingFilter".to_string(),
            detail: "Filter groups on HAVING predicate.".to_string(),
        });
    }
}

fn append_post_scan_steps(steps: &mut Vec<PlanStep>, stmt: &SelectStatement) {
    if stmt.fusion_clause.is_some() {
        steps.push(PlanStep {
            node: "FusionCombine".to_string(),
            detail: "Fuse ranked branches (RRF / weighted).".to_string(),
        });
    }
    if matches!(stmt.distinct, velesdb_core::velesql::DistinctMode::All) {
        steps.push(PlanStep {
            node: "Distinct".to_string(),
            detail: "Deduplicate rows by projected columns.".to_string(),
        });
    }
    if stmt.order_by.is_some() {
        steps.push(PlanStep {
            node: "Sort".to_string(),
            detail: "Sort by ORDER BY keys.".to_string(),
        });
    }
    if stmt.limit.is_some() || stmt.offset.is_some() {
        steps.push(PlanStep {
            node: "LimitOffset".to_string(),
            detail: format!(
                "LIMIT={} OFFSET={}",
                stmt.limit.unwrap_or(u64::MAX),
                stmt.offset.unwrap_or(0),
            ),
        });
    }
}

fn append_compound_steps(steps: &mut Vec<PlanStep>, query: &Query) {
    if let Some(compound) = &query.compound {
        for (op, _) in &compound.operations {
            steps.push(PlanStep {
                node: format!("{op:?}"),
                detail: "Combine with right-hand SELECT.".to_string(),
            });
        }
    }
}

fn explain_dml(dml: &velesdb_core::velesql::DmlStatement) -> Vec<PlanStep> {
    use velesdb_core::velesql::DmlStatement;
    let (label, detail) = match dml {
        DmlStatement::Insert(s) => (
            "Insert",
            format!("INSERT INTO {} {} row(s)", s.table, s.rows.len()),
        ),
        DmlStatement::Upsert(s) => (
            "Upsert",
            format!("UPSERT INTO {} {} row(s)", s.table, s.rows.len()),
        ),
        DmlStatement::Update(s) => ("Update", format!("UPDATE {}", s.table)),
        DmlStatement::Delete(s) => ("Delete", format!("DELETE FROM {}", s.table)),
        DmlStatement::InsertEdge(s) => ("InsertEdge", format!("INSERT EDGE INTO {}", s.collection)),
        DmlStatement::DeleteEdge(s) => ("DeleteEdge", format!("DELETE EDGE FROM {}", s.collection)),
        DmlStatement::SelectEdges(s) => {
            ("SelectEdges", format!("SELECT EDGES FROM {}", s.collection))
        }
        DmlStatement::InsertNode(s) => ("InsertNode", format!("INSERT NODE INTO {}", s.collection)),
        _ => ("Dml", "Unsupported DML variant".to_string()),
    };
    vec![PlanStep {
        node: label.to_string(),
        detail,
    }]
}

fn explain_match(clause: &velesdb_core::velesql::MatchClause) -> Vec<PlanStep> {
    vec![
        PlanStep {
            node: "GraphPatternMatch".to_string(),
            detail: format!(
                "Match {} node pattern(s), {} relationship(s)",
                clause.patterns.iter().map(|p| p.nodes.len()).sum::<usize>(),
                clause
                    .patterns
                    .iter()
                    .map(|p| p.relationships.len())
                    .sum::<usize>()
            ),
        },
        PlanStep {
            node: "Return".to_string(),
            detail: format!("Return {} item(s)", clause.return_clause.items.len()),
        },
    ]
}

fn row_count_hint(db: &DatabaseInner, name: &str) -> usize {
    db.get_shared_store(name)
        .map(|s| s.borrow().ids.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use velesdb_core::velesql::Parser;

    #[test]
    fn test_explain_plain_select() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("t").expect("test: create");
        let q = Parser::parse("SELECT * FROM t WHERE x = 1 LIMIT 5").expect("test: parse");
        let rows = explain(&db, &q).expect("test: explain");
        assert!(rows.len() >= 3);
        assert!(rows[0].data_json_ref().contains("Scan"));
    }

    #[test]
    fn test_explain_group_by_has_groupby_step() {
        let mut db = DatabaseInner::new();
        db.create_metadata_collection("t").expect("test: create");
        let q = Parser::parse("SELECT cat, COUNT(*) FROM t GROUP BY cat").expect("test: parse");
        let rows = explain(&db, &q).expect("test: explain");
        let joined = rows
            .iter()
            .map(crate::velesql_result::QueryResultRow::data_json_ref)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("GroupBy"));
    }

    #[test]
    fn test_explain_ddl_has_ddl_node() {
        let db = DatabaseInner::new();
        let q = Parser::parse("CREATE COLLECTION v (dimension = 4, metric = 'cosine')")
            .expect("test: parse");
        let rows = explain(&db, &q).expect("test: explain");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].data_json_ref().contains("DDL"));
    }

    #[test]
    fn test_explain_dml_insert_node() {
        let db = DatabaseInner::new();
        let q = Parser::parse("INSERT NODE INTO kg (id = 1, payload = '{}')").expect("test: parse");
        let rows = explain(&db, &q).expect("test: explain");
        assert!(rows[0].data_json_ref().contains("InsertNode"));
    }

    #[test]
    fn test_explain_match_has_pattern_step() {
        let db = DatabaseInner::new();
        let q = Parser::parse("MATCH (a:Person) RETURN a LIMIT 5").expect("test: parse");
        let rows = explain(&db, &q).expect("test: explain");
        assert!(rows[0].data_json_ref().contains("GraphPatternMatch"));
    }
}
