//! Read-path control-plane gate — `VelesCollection::analyze` must consult it.
//!
//! `analyze()` walks the column store and returns fresh statistics
//! (`row_count`, `column_stats`, `index_stats`) — the same collection data
//! `get_stats()` returns, which IS gated in this branch (`collection.rs`).
//! `analyze()` calls `self.inner.analyze()` with no `authorize_read`, so a
//! `MobileObserver` denial is ignored. This test fails until `analyze()`
//! passes the same gate as `get_stats()`.

use std::sync::Arc;

use tempfile::TempDir;
use velesdb_mobile::{
    DistanceMetric, MobileAccessDecision, MobileObserver, MobileQueryContext, VelesDatabase,
    VelesPoint,
};

struct DenyAll;

impl MobileObserver for DenyAll {
    fn on_query_request(&self, _context: MobileQueryContext) -> MobileAccessDecision {
        MobileAccessDecision::Deny {
            reason: "test policy".to_string(),
        }
    }
}

/// GIVEN a database whose observer denies every read
/// WHEN `VelesCollection::analyze` runs
/// THEN it fails with the denial, as `get_stats` already does.
#[test]
fn analyze_is_gated_like_get_stats() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().to_str().expect("path utf8").to_string();
    let db = VelesDatabase::open_with_observer(path, Arc::new(DenyAll)).expect("open db");
    db.create_collection("vectors".to_string(), 4, DistanceMetric::Cosine)
        .expect("create collection");
    let col = db
        .get_collection("vectors".to_string())
        .expect("get_collection ok")
        .expect("collection exists");
    col.upsert(VelesPoint {
        id: 1,
        vector: vec![1.0, 0.0, 0.0, 0.0],
        payload: Some(r#"{"cat":"a"}"#.to_string()),
    })
    .expect("upsert is a write, not a gated read");

    // Control: the sibling metadata read IS gated — it must fail.
    assert!(
        col.get_stats().is_err(),
        "get_stats is gated and must fail under a denied read"
    );

    // The gap: analyze() returns the same statistics with no gate.
    let result = col.analyze();
    assert!(
        result.is_err(),
        "analyze() returns the same statistics as the gated get_stats() and \
         must fail under a denied read, got {result:?}"
    );
}
