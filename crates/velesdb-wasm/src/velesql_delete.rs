//! DELETE dispatch for the WASM VelesQL executor (S4-13).
//!
//! Evaluates the mandatory WHERE clause against every row of the target
//! collection and removes the matching entries. Works on both metadata-only
//! and vector collections (the store's `swap_remove` path handles both).

use velesdb_core::velesql::DeleteStatement;

use crate::database::DatabaseInner;
use crate::velesql_helpers::collect_matching_indices;
use crate::velesql_value::Params;

/// Executes a DELETE statement. Returns the number of rows removed.
pub(crate) fn execute(
    db: &DatabaseInner,
    stmt: &DeleteStatement,
    params: &Params,
) -> Result<u32, String> {
    let store = db.get_shared_store(&stmt.table)?;
    let to_remove = collect_matching_indices(&store, Some(&stmt.where_clause), params)?;
    remove_indices_desc(&store, &to_remove);

    Ok(u32::try_from(to_remove.len()).unwrap_or(u32::MAX))
}

/// Removes rows from the store in descending order to keep indices stable.
fn remove_indices_desc(
    store: &std::rc::Rc<std::cell::RefCell<crate::vector_store::VectorStore>>,
    indices: &[usize],
) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut borrowed = store.borrow_mut();
    for &idx in &sorted {
        crate::store_insert::remove_at_index(&mut borrowed, idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::{DmlStatement, Parser};

    fn parse_delete(sql: &str) -> DeleteStatement {
        let q = Parser::parse(sql).expect("test: parse");
        match q.dml.expect("test: has dml") {
            DmlStatement::Delete(s) => s,
            other => panic!("expected DELETE, got {other:?}"),
        }
    }

    fn seed_metadata_docs(db: &mut DatabaseInner) {
        db.create_metadata_collection("docs").expect("test: create");
        let store = db.get_shared_store("docs").expect("test: store");
        let mut borrowed = store.borrow_mut();
        for (id, cat) in [(1u64, "tech"), (2, "food"), (3, "tech")] {
            borrowed.ids.push(id);
            borrowed
                .payloads
                .push(Some(serde_json::json!({"cat": cat})));
        }
    }

    #[test]
    fn test_delete_by_id() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_delete("DELETE FROM docs WHERE id = 2");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
        assert_eq!(n, 1);

        let store = db.get_shared_store("docs").expect("test: store");
        let borrowed = store.borrow();
        assert!(!borrowed.ids.contains(&2));
        assert_eq!(borrowed.ids.len(), 2);
    }

    #[test]
    fn test_delete_by_payload_field() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_delete("DELETE FROM docs WHERE cat = 'tech'");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
        assert_eq!(n, 2);

        let store = db.get_shared_store("docs").expect("test: store");
        assert_eq!(store.borrow().ids.len(), 1);
    }

    #[test]
    fn test_delete_no_match_returns_zero() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_delete("DELETE FROM docs WHERE id = 999");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_delete_missing_collection_errors() {
        let db = DatabaseInner::new();
        let stmt = parse_delete("DELETE FROM ghost WHERE id = 1");
        let err = execute(&db, &stmt, &parse_params(None).expect("test: p"));
        assert!(err.is_err());
    }

    #[test]
    fn test_delete_preserves_other_rows_indices() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let stmt = parse_delete("DELETE FROM docs WHERE id IN (1, 3)");
        let n = execute(&db, &stmt, &parse_params(None).expect("test: p")).expect("test: delete");
        assert_eq!(n, 2);

        let store = db.get_shared_store("docs").expect("test: store");
        let borrowed = store.borrow();
        assert_eq!(borrowed.ids, vec![2]);
    }
}
