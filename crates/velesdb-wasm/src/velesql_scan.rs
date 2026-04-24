//! Shared row-scan helpers for the WASM SELECT pipeline (S4-13).
//!
//! Extracts the WHERE-filtered row iteration that both the plain-scan path
//! (payload-only SELECT) and the aggregation pipeline need. Keeps
//! `velesql_select.rs` under the NLOC limit while giving the aggregator a
//! clean API to receive pre-filtered rows without re-implementing the walk.

use velesdb_core::velesql::Condition;

use crate::database::DatabaseInner;
use crate::velesql_value::Params;
use crate::velesql_where;

/// A single scanned row owned by the caller: `(id, score, payload_json)`.
///
/// Owning the payload (rather than borrowing) lets the caller drop the
/// store lock before running the aggregation / sort pipeline.
pub(crate) type OwnedScanRow = (u64, f32, Option<serde_json::Value>);

/// Scans a metadata-only or vector collection end-to-end, applying the
/// optional WHERE clause. Returns rows in insertion order with `score` set
/// to 0.0 (vector search happens in `velesql_select::vector_path`).
pub(crate) fn scan_all(
    db: &DatabaseInner,
    from: &str,
    where_clause: Option<&Condition>,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    let store = db.get_shared_store(from)?;
    let borrowed = store.borrow();
    let mut out = Vec::with_capacity(borrowed.ids.len());
    for (idx, &id) in borrowed.ids.iter().enumerate() {
        let payload = borrowed.payloads.get(idx).and_then(|p| p.as_ref());
        if let Some(cond) = where_clause {
            if !velesql_where::matches(cond, id, payload, params)? {
                continue;
            }
        }
        out.push((id, 0.0, payload.cloned()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use velesdb_core::velesql::Parser;

    fn seed(db: &mut DatabaseInner) {
        db.create_metadata_collection("t").expect("test: create");
        let store = db.get_shared_store("t").expect("test: store");
        let mut b = store.borrow_mut();
        for (id, cat) in [(1u64, "a"), (2, "b"), (3, "a")] {
            b.ids.push(id);
            b.payloads.push(Some(serde_json::json!({"cat": cat})));
        }
    }

    #[test]
    fn test_scan_all_no_where() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let rows = scan_all(&db, "t", None, &Params::new()).expect("test: scan");
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_scan_all_with_where() {
        let mut db = DatabaseInner::new();
        seed(&mut db);
        let q = Parser::parse("SELECT * FROM t WHERE cat = 'a'").expect("test: parse");
        let rows =
            scan_all(&db, "t", q.select.where_clause.as_ref(), &Params::new()).expect("test: scan");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_scan_missing_collection_errors() {
        let db = DatabaseInner::new();
        let err = scan_all(&db, "ghost", None, &Params::new());
        assert!(err.is_err());
    }
}
