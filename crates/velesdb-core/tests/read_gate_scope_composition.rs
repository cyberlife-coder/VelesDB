//! Read-path control-plane gate — scope AND-composition.
//!
//! Feature: core-control-plane-boundary, Property 4
//!
//! **Property 4: `AllowWithScope` AND-composes and never widens** —
//! when the read-path observer hook returns
//! `AccessDecision::AllowWithScope(scope)`, the scope filter `F` is combined
//! with the query's pre-existing WHERE clause via logical AND. The resulting
//! set `R'` is therefore always the intersection `R ∩ F` of the unscoped
//! result set `R` and the scope filter, and is always a subset of `R`
//! (the scope can only narrow, never widen). Any pre-existing WHERE predicate
//! is preserved (AND-combined, never rewritten or dropped).
//!
//! **Validates: Requirements 1.5**
//!
//! Strategy: a single temp-dir `Database` is opened with a `ScopingObserver`
//! whose decision is toggled between iterations. For each generated data set,
//! base query (with or without a pre-existing WHERE), and scope condition:
//!
//! * `R`  = base query with the observer in Allow mode (baseline).
//! * `R'` = the SAME base query with the observer returning
//!   `AllowWithScope(F)`.
//! * `R ∩ F` = a reference query whose WHERE is `(base_where) AND F`, run in
//!   Allow mode.
//!
//! The property asserts `R' ⊆ R` (never widens) and `R' == R ∩ F`
//! (exact AND-composition, pre-existing predicate preserved). Queries use a
//! high `LIMIT` so no truncation masks the set relationship.

#![cfg(feature = "persistence")]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use parking_lot::RwLock;
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use velesdb_core::velesql::{Condition, Parser};
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, DistanceMetric, Point,
    QueryAccessContext,
};

/// The metadata categories generated points are drawn from.
const CATEGORIES: [&str; 4] = ["tech", "science", "art", "food"];

/// Observer whose read-path decision is switched between proptest iterations
/// through a lock, so one opened `Database` can exercise both the unscoped
/// baseline (Allow) and the scoped path (`AllowWithScope`) over identical data.
struct ScopingObserver {
    scope: RwLock<Option<Condition>>,
}

impl ScopingObserver {
    fn new() -> Self {
        Self {
            scope: RwLock::new(None),
        }
    }

    /// Next `on_query_request` returns `Allow` (unscoped baseline / reference).
    fn set_allow(&self) {
        *self.scope.write() = None;
    }

    /// Next `on_query_request` returns `AllowWithScope(cond)`.
    fn set_scope(&self, cond: Condition) {
        *self.scope.write() = Some(cond);
    }
}

impl DatabaseObserver for ScopingObserver {
    fn on_query_request(&self, _ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        match self.scope.read().clone() {
            None => Ok(AccessDecision::Allow),
            Some(cond) => {
                // `AccessScope` is `#[non_exhaustive]`; construct via Default
                // and set the public `filter` field (no literal is possible
                // from a downstream crate).
                #[allow(clippy::field_reassign_with_default)]
                let mut scope = AccessScope::default();
                scope.filter = Some(cond);
                Ok(AccessDecision::AllowWithScope(scope))
            }
        }
    }
}

/// Opens a temp-dir database wired with `observer`, creates the `items`
/// vector collection, and inserts the generated points.
fn setup(observer: Arc<dyn DatabaseObserver>, points: Vec<Point>) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), observer).expect("test: open db");
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get collection");
    collection.upsert(points).expect("test: upsert items");
    (dir, db)
}

/// Builds a WHERE fragment (predicate text, no `WHERE` keyword) for the given
/// kind. Kind `0` yields `None` (no predicate); kinds `1..=4` always yield a
/// fragment, including a compound `AND` fragment (kind `4`) so a pre-existing
/// multi-term WHERE is exercised.
fn where_fragment(kind: u8, price: i64, category: &str) -> Option<String> {
    match kind {
        1 => Some(format!("price > {price}")),
        2 => Some(format!("price <= {price}")),
        3 => Some(format!("category = '{category}'")),
        4 => Some(format!("price > {price} AND category = '{category}'")),
        _ => None,
    }
}

/// Runs a `VelesQL` SELECT and collects the resulting point ids as a set.
fn run_ids(db: &Database, sql: &str) -> BTreeSet<u64> {
    let query = Parser::parse(sql).expect("test: parse query");
    db.execute_query(&query, &HashMap::new())
        .expect("test: execute query")
        .into_iter()
        .map(|result| result.point.id)
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 4: `AllowWithScope(F)` AND-composes `F` into the query and
    /// never widens the result set — `R' == R ∩ F` and `R' ⊆ R`, with any
    /// pre-existing WHERE preserved via logical AND.
    #[test]
    fn allow_with_scope_and_composes_and_never_widens(
        data in prop::collection::vec((0usize..4usize, 0i64..=200i64), 1..=15),
        base_kind in 0u8..5,
        base_price in 0i64..=200,
        base_cat in 0usize..4,
        scope_kind in 1u8..5,
        scope_price in 0i64..=200,
        scope_cat in 0usize..4,
    ) {
        // Build the points: unique ids 1..=N, category + price metadata.
        let points: Vec<Point> = data
            .iter()
            .enumerate()
            .map(|(index, (cat_idx, price))| {
                let id = u64::try_from(index).expect("test: index fits u64") + 1;
                Point::new(
                    id,
                    vec![1.0, 0.0, 0.0, 0.0],
                    Some(json!({
                        "_labels": ["Item"],
                        "category": CATEGORIES[*cat_idx],
                        "price": price,
                    })),
                )
            })
            .collect();

        let observer = Arc::new(ScopingObserver::new());
        let (_dir, db) = setup(observer.clone() as Arc<dyn DatabaseObserver>, points);

        // Base query: some iterations carry a pre-existing WHERE, some none.
        let base_frag = where_fragment(base_kind, base_price, CATEGORIES[base_cat]);
        let base_sql = match &base_frag {
            Some(frag) => format!("SELECT * FROM items WHERE {frag} LIMIT 1000"),
            None => "SELECT * FROM items LIMIT 1000".to_string(),
        };

        // Scope filter fragment (kind 1..=4 always yields one) → velesql::Condition
        // obtained by parsing a SELECT and extracting its WHERE clause.
        let scope_frag = where_fragment(scope_kind, scope_price, CATEGORIES[scope_cat])
            .expect("test: scope kind 1..=4 always yields a fragment");
        let scope_cond = Parser::parse(&format!("SELECT * FROM items WHERE {scope_frag}"))
            .expect("test: parse scope condition")
            .select
            .where_clause
            .expect("test: scope query has a WHERE clause");

        // Reference intersection query: (base_where) AND (scope_frag).
        let intersect_sql = match &base_frag {
            Some(frag) => {
                format!("SELECT * FROM items WHERE ({frag}) AND ({scope_frag}) LIMIT 1000")
            }
            None => format!("SELECT * FROM items WHERE {scope_frag} LIMIT 1000"),
        };

        // R: unscoped baseline (observer allows unmodified).
        observer.set_allow();
        let r = run_ids(&db, &base_sql);

        // R': same base query, observer returns AllowWithScope(F).
        observer.set_scope(scope_cond);
        let r_scoped = run_ids(&db, &base_sql);

        // R ∩ F: reference intersection, observer allows unmodified.
        observer.set_allow();
        let r_intersect = run_ids(&db, &intersect_sql);

        // Never widens: R' must be a subset of the unscoped result set.
        prop_assert!(
            r_scoped.is_subset(&r),
            "AllowWithScope widened the result set: R'={r_scoped:?} not a subset of R={r:?}"
        );

        // Exact AND-composition: R' equals the intersection R ∩ F, proving the
        // scope was AND-combined with any pre-existing WHERE (never dropped).
        prop_assert_eq!(
            &r_scoped,
            &r_intersect,
            "R' must equal R ∩ F (base_sql={}, scope={})",
            base_sql,
            scope_frag
        );
    }
}
