//! UPDATE dispatch for the WASM VelesQL executor (S4-13).
//!
//! Scope: payload-only `SET column = value` updates with a standard WHERE
//! clause (including IN / BETWEEN / LIKE / AND / OR). The `vector` column
//! is NOT writable via UPDATE — reassigning the embedding of an existing
//! point requires UPSERT instead, matching the Mobile executor semantics.

use velesdb_core::velesql::{UpdateAssignment, UpdateStatement};

use crate::database::DatabaseInner;
use crate::velesql_helpers::collect_matching_indices;
use crate::velesql_value::{resolve_value, Params};

/// Executes an UPDATE statement. Returns the number of rows whose payload
/// was mutated.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &UpdateStatement,
    params: &Params,
) -> Result<u32, String> {
    validate_assignments(&stmt.assignments)?;

    let store = db.get_shared_store(&stmt.table)?;
    let indices = collect_matching_indices(&store, stmt.where_clause.as_ref(), params)?;
    apply_updates(&store, &indices, &stmt.assignments, params)?;

    let n = u32::try_from(indices.len()).unwrap_or(u32::MAX);
    Ok(n)
}

/// Rejects assignments that target reserved columns.
fn validate_assignments(assignments: &[UpdateAssignment]) -> Result<(), String> {
    if assignments.is_empty() {
        return Err("UPDATE must include at least one SET assignment".to_string());
    }
    for a in assignments {
        if a.column == "id" {
            return Err("UPDATE cannot modify the 'id' column".to_string());
        }
        if a.column == "vector" {
            return Err("UPDATE cannot modify the 'vector' column (use UPSERT)".to_string());
        }
    }
    Ok(())
}

/// Applies the `SET` assignments to the rows at the given indices.
fn apply_updates(
    store: &std::rc::Rc<std::cell::RefCell<crate::vector_store::VectorStore>>,
    indices: &[usize],
    assignments: &[UpdateAssignment],
    params: &Params,
) -> Result<(), String> {
    // Resolve assignment values ONCE; params + literals are stable across rows.
    let resolved: Vec<(String, serde_json::Value)> = assignments
        .iter()
        .map(|a| resolve_value(&a.value, params).map(|v| (a.column.clone(), v)))
        .collect::<Result<_, _>>()?;

    let mut borrowed = store.borrow_mut();
    for &idx in indices {
        let Some(slot) = borrowed.payloads.get_mut(idx) else {
            continue;
        };
        let map = ensure_object(slot);
        for (col, val) in &resolved {
            map.insert(col.clone(), val.clone());
        }
    }
    Ok(())
}

/// Ensures the payload at `slot` is a JSON object we can mutate.
///
/// Replaces `None` / non-object payloads with a fresh empty object so the
/// caller can insert assignment values into it.
fn ensure_object(
    slot: &mut Option<serde_json::Value>,
) -> &mut serde_json::Map<String, serde_json::Value> {
    if !matches!(slot, Some(serde_json::Value::Object(_))) {
        *slot = Some(serde_json::Value::Object(serde_json::Map::new()));
    }
    match slot {
        Some(serde_json::Value::Object(m)) => m,
        // Unreachable by construction above, but kept defensively.
        _ => unreachable!("ensure_object guarantees an Object variant"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::{DmlStatement, Parser};

    fn parse_update(sql: &str) -> UpdateStatement {
        let q = Parser::parse(sql).expect("test: parse");
        match q.dml.expect("test: has dml") {
            DmlStatement::Update(s) => s,
            other => panic!("expected UPDATE, got {other:?}"),
        }
    }

    fn seed_metadata_docs(db: &mut DatabaseInner) {
        db.create_metadata_collection("docs").expect("test: create");
        let store = db.get_shared_store("docs").expect("test: store");
        let mut borrowed = store.borrow_mut();
        borrowed.ids.push(1);
        borrowed
            .payloads
            .push(Some(serde_json::json!({"title": "first", "cat": "tech"})));
        borrowed.ids.push(2);
        borrowed
            .payloads
            .push(Some(serde_json::json!({"title": "second", "cat": "food"})));
    }

    #[test]
    fn test_update_sets_field_on_match() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_update("UPDATE docs SET title = 'renamed' WHERE id = 1");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
        assert_eq!(n, 1);

        let store = db.get_shared_store("docs").expect("test: store");
        let borrowed = store.borrow();
        let p = borrowed.payloads[0].as_ref().expect("test: payload");
        assert_eq!(p["title"], "renamed");
    }

    #[test]
    fn test_update_without_where_affects_all_rows() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_update("UPDATE docs SET cat = 'x'");
        let n =
            execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update all");
        assert_eq!(n, 2);
    }

    #[test]
    fn test_update_on_id_column_is_rejected() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_update("UPDATE docs SET id = 99 WHERE id = 1");
        let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("'id'"));
    }

    #[test]
    fn test_update_on_vector_column_is_rejected() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let stmt = parse_update("UPDATE vecs SET vector = 'x' WHERE id = 1");
        let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
        assert!(err.is_err());
        assert!(err.expect_err("test: err").contains("'vector'"));
    }

    #[test]
    fn test_update_missing_collection_errors() {
        let db = DatabaseInner::new();
        let stmt = parse_update("UPDATE ghost SET x = 1 WHERE id = 1");
        let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
        assert!(err.is_err());
    }

    #[test]
    fn test_update_no_rows_match_returns_zero() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_update("UPDATE docs SET title = 'z' WHERE id = 999");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: update");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_update_with_param() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_update("UPDATE docs SET cat = $new WHERE id = 1");
        let params = parse_params(Some(r#"{"new": "gaming"}"#)).expect("test: p");
        let n = execute(&db, &stmt, &params).expect("test: update");
        assert_eq!(n, 1);

        let store = db.get_shared_store("docs").expect("test: store");
        let borrowed = store.borrow();
        let p = borrowed.payloads[0].as_ref().expect("test: payload");
        assert_eq!(p["cat"], "gaming");
    }
}
