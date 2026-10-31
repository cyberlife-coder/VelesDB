//! The Tauri `query` command's aggregate route honours the core read gate:
//! an observer handed to [`VelesDbState::new_with_observer`] that scopes a
//! collection narrows the aggregate, and one that denies it refuses it.
//!
//! The fixture is `items`: three rows in category `tech`, one in `sports`.
#![cfg(feature = "persistence")]

use std::sync::Arc;

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{App, Manager};
use tauri_plugin_velesdb::commands::{create_collection, query, upsert};
use tauri_plugin_velesdb::types::{
    CreateCollectionRequest, PointInput, QueryRequest, UpsertRequest,
};
use tauri_plugin_velesdb::VelesDbState;
use velesdb_core::{AccessDecision, AccessScope, DatabaseObserver, Error, QueryAccessContext};

/// Scopes `items` to `category = 'tech'`, or denies it.
struct ItemsPolicy {
    deny: bool,
}

impl DatabaseObserver for ItemsPolicy {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        if ctx.collection != "items" {
            return Ok(AccessDecision::Allow);
        }
        if self.deny {
            return Ok(AccessDecision::Deny(Error::Query(
                "items are off limits".to_string(),
            )));
        }
        #[allow(clippy::field_reassign_with_default)] // `AccessScope` is non_exhaustive
        let mut scope = AccessScope::default();
        scope.filter =
            velesdb_core::velesql::Parser::parse("SELECT * FROM items WHERE category = 'tech'")
                .expect("scope predicate parses")
                .select
                .where_clause;
        Ok(AccessDecision::AllowWithScope(scope))
    }
}

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tauri::async_runtime::block_on(fut)
}

/// A mock app whose database is opened under `policy`, holding `items`.
fn app_under(dir: &std::path::Path, policy: ItemsPolicy) -> App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build mock app");
    app.manage(VelesDbState::new_with_observer(
        dir.to_path_buf(),
        Arc::new(policy),
    ));
    let request: CreateCollectionRequest =
        serde_json::from_value(serde_json::json!({"name": "items", "dimension": 4}))
            .expect("create request");
    run(create_collection(
        app.handle().clone(),
        app.state(),
        request,
    ))
    .expect("create items");
    let points: Vec<PointInput> = [(1, "tech"), (2, "tech"), (3, "tech"), (4, "sports")]
        .iter()
        .map(|(id, category)| {
            serde_json::from_value(serde_json::json!({
                "id": id,
                "vector": [1.0, 0.0, 0.0, 0.0],
                "payload": {"category": category},
            }))
            .expect("point")
        })
        .collect();
    run(upsert(
        app.handle().clone(),
        app.state(),
        UpsertRequest {
            collection: "items".to_string(),
            points,
        },
    ))
    .expect("upsert items");
    app
}

fn aggregate(app: &App<tauri::test::MockRuntime>, sql: &str) -> Result<serde_json::Value, String> {
    run(query(
        app.handle().clone(),
        app.state(),
        QueryRequest {
            query: sql.to_string(),
            params: std::collections::HashMap::new(),
        },
    ))
    .map(|response| {
        response.results[0]
            .column_data
            .clone()
            .expect("an aggregate answers in column_data")
    })
    .map_err(|e| format!("{e:?}"))
}

/// GIVEN `items` scoped to `category = 'tech'`
/// WHEN `COUNT(*)` and a `GROUP BY` run through the `query` command
/// THEN they see only the 3 scoped rows, and the `tech` group alone.
#[test]
fn query_command_aggregates_honour_the_read_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = app_under(dir.path(), ItemsPolicy { deny: false });

    let count = aggregate(&app, "SELECT COUNT(*) AS n FROM items").expect("scoped COUNT");
    assert_eq!(count["n"], 3, "{count}");

    let groups = aggregate(
        &app,
        "SELECT category, COUNT(*) AS n FROM items GROUP BY category",
    )
    .expect("scoped GROUP BY");
    assert_eq!(groups.as_array().map(Vec::len), Some(1), "{groups}");
}

/// GIVEN the observer denies `items`
/// WHEN an aggregate runs through the `query` command
/// THEN it fails with the observer's message.
#[test]
fn query_command_aggregates_refuse_a_denied_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = app_under(dir.path(), ItemsPolicy { deny: true });

    let err = aggregate(&app, "SELECT COUNT(*) AS n FROM items").expect_err("denied");

    assert!(err.contains("items are off limits"), "{err}");
}
