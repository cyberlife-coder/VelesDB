//! Wire contracts of the Tauri commands, driven through the real handlers
//! with requests built from the JSON a webview sends.
//!
//! Each test pins one promise the TypeScript bindings and the guides make:
//! the documented `graphSchema` short forms, `query`'s `collection` field for
//! a `MATCH`, the shape of `columnData` and of absent optional fields, the
//! error class of a graph command aimed at another kind of collection, the
//! sparse-vector size cap, and `train_pq`'s required `m`.
//!
//! Requires the `persistence` feature, like `command_handlers.rs`.
#![cfg(feature = "persistence")]

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};

use tauri_plugin_velesdb::commands::{create_collection, get_edges, query, upsert};
use tauri_plugin_velesdb::commands_graph::create_graph_collection;
use tauri_plugin_velesdb::commands_sparse::{hybrid_sparse_search, sparse_search, sparse_upsert};
use tauri_plugin_velesdb::types::{CollectionInfo, QueryResponse, TrainPqRequest};
use tauri_plugin_velesdb::{CommandError, VelesDbState};

/// Builds an `App<MockRuntime>` managing a `VelesDbState` rooted in `dir`.
fn mock_app(dir: &std::path::Path) -> App<MockRuntime> {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build mock app");
    app.manage(VelesDbState::new(dir.to_path_buf()));
    app
}

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tauri::async_runtime::block_on(fut)
}

/// Deserializes a request exactly as Tauri does from the `invoke` payload.
fn req<T: serde::de::DeserializeOwned>(json: serde_json::Value) -> T {
    serde_json::from_value(json).expect("request deserializes")
}

fn create_graph(
    app: &App<MockRuntime>,
    json: serde_json::Value,
) -> Result<CollectionInfo, CommandError> {
    run(create_graph_collection(
        app.handle().clone(),
        app.state(),
        req(json),
    ))
}

fn run_query(
    app: &App<MockRuntime>,
    json: serde_json::Value,
) -> Result<QueryResponse, CommandError> {
    run(query(app.handle().clone(), app.state(), req(json)))
}

fn seed_docs(app: &App<MockRuntime>) {
    run(create_collection(
        app.handle().clone(),
        app.state(),
        req(serde_json::json!({ "name": "docs", "dimension": 2 })),
    ))
    .expect("create docs");
    run(upsert(
        app.handle().clone(),
        app.state(),
        req(serde_json::json!({
            "collection": "docs",
            "points": [
                { "id": 1, "vector": [1.0, 0.0], "payload": { "category": "a" } },
                { "id": 2, "vector": [0.0, 1.0], "payload": { "category": "b" } },
                { "id": 3, "vector": [1.0, 1.0], "payload": { "category": "a" } }
            ]
        })),
    ))
    .expect("upsert docs");
}

// ===========================================================================
// graphSchema short forms
// ===========================================================================

/// `graphSchema: { schemaless: true }` is the form the TypeScript docs show;
/// it used to fail VELES-009 "missing field `node_types`".
#[test]
fn schemaless_short_form_creates_a_schemaless_graph() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    create_graph(
        &app,
        serde_json::json!({ "name": "kg", "graphSchema": { "schemaless": true } }),
    )
    .expect("the documented schemaless short form is accepted");

    let schema = app
        .state::<VelesDbState>()
        .with_db(|db| {
            db.get_graph_collection("kg")
                .map(|g| g.schema())
                .ok_or_else(|| tauri_plugin_velesdb::Error::CollectionNotFound("kg".into()))
        })
        .expect("graph collection exists");
    assert!(schema.is_schemaless());
}

/// A strict schema whose types declare no properties may omit `properties`.
#[test]
fn strict_schema_without_properties_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());

    create_graph(
        &app,
        serde_json::json!({
            "name": "kg_strict",
            "graphSchema": {
                "schemaless": false,
                "node_types": [{ "name": "Person" }],
                "edge_types": [{ "name": "KNOWS", "from_type": "Person", "to_type": "Person" }]
            }
        }),
    )
    .expect("node and edge types without `properties` are accepted");
}

// ===========================================================================
// query: `collection` addresses a MATCH
// ===========================================================================

const MATCH_SQL: &str = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name";

/// A top-level `MATCH` names no collection; `collection` supplies it, as the
/// REST server's `/query` body field does. Without it the statement is refused.
#[test]
fn query_collection_field_addresses_a_match() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    create_graph(&app, serde_json::json!({ "name": "social" })).expect("create graph");

    let refused = run_query(&app, serde_json::json!({ "query": MATCH_SQL }))
        .expect_err("a MATCH with no collection is refused");
    assert!(refused.code.starts_with("VELES-"), "{refused:?}");

    run_query(
        &app,
        serde_json::json!({ "query": MATCH_SQL, "collection": "social" }),
    )
    .expect("`collection` addresses the MATCH");
}

/// An explicit `params._collection` keeps precedence over `collection`, as on
/// the REST server's `/query` (the body field only fills an absent one).
#[test]
fn query_explicit_collection_param_takes_precedence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    create_graph(&app, serde_json::json!({ "name": "social" })).expect("create graph");

    run_query(
        &app,
        serde_json::json!({
            "query": MATCH_SQL,
            "collection": "ghost",
            "params": { "_collection": "social" }
        }),
    )
    .expect("the explicit `_collection` wins");

    let err = run_query(
        &app,
        serde_json::json!({
            "query": MATCH_SQL,
            "collection": "social",
            "params": { "_collection": "ghost" }
        }),
    )
    .expect_err("the explicit `_collection` names a missing collection");
    assert_eq!(err.code, "VELES-002", "{err:?}");
}

// ===========================================================================
// HybridResult wire shape
// ===========================================================================

/// What the webview receives: `columnData` is one object for an ungrouped
/// aggregate and an array of row objects under GROUP BY, and every absent
/// optional field arrives as `null`, never omitted.
#[test]
fn hybrid_result_wire_shape_for_grouped_and_ungrouped_aggregates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_docs(&app);

    let wire = |sql: &str| {
        let resp = run_query(&app, serde_json::json!({ "query": sql })).expect("aggregate");
        serde_json::to_value(&resp).expect("serialize response")["results"][0].clone()
    };

    let ungrouped = wire("SELECT COUNT(*) FROM docs");
    assert!(ungrouped["columnData"].is_object(), "{ungrouped}");
    let grouped = wire("SELECT category, COUNT(*) FROM docs GROUP BY category");
    assert!(grouped["columnData"].is_array(), "{grouped}");

    for row in [&ungrouped, &grouped] {
        for field in ["vectorScore", "graphScore", "bindings"] {
            assert!(
                row[field].is_null(),
                "{field} must be present as null: {row}"
            );
            assert!(
                row.get(field).is_some(),
                "{field} must not be omitted: {row}"
            );
        }
    }

    let search = wire("SELECT * FROM docs LIMIT 1");
    assert!(search["columnData"].is_null() && search.get("columnData").is_some());
    assert!(search["graphScore"].is_null() && search.get("graphScore").is_some());
}

// ===========================================================================
// Graph command on another kind of collection
// ===========================================================================

/// A graph command aimed at a vector collection answers core's
/// GraphNotSupported (VELES-018), not "not found"; a missing name stays
/// VELES-002.
#[test]
fn graph_command_distinguishes_wrong_kind_from_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_docs(&app);

    let edges = |name: &str| {
        run(get_edges(
            app.handle().clone(),
            app.state(),
            req(serde_json::json!({ "collection": name })),
        ))
        .expect_err("not a graph collection")
    };

    let wrong_kind = edges("docs");
    assert_eq!(wrong_kind.code, "VELES-018", "{wrong_kind:?}");
    let missing = edges("ghost");
    assert_eq!(missing.code, "VELES-002", "{missing:?}");
}

// ===========================================================================
// Sparse vector size cap
// ===========================================================================

/// One key past core's `MAX_SPARSE_NNZ`.
fn oversized_sparse() -> serde_json::Value {
    let cap = velesdb_core::api_types::MAX_SPARSE_NNZ;
    let entries: serde_json::Map<String, serde_json::Value> = (0..=cap)
        .map(|i| (i.to_string(), serde_json::json!(1.0)))
        .collect();
    serde_json::Value::Object(entries)
}

fn assert_too_large(result: Result<impl std::fmt::Debug, CommandError>, command: &str) {
    let err = result.expect_err("an oversized sparse vector is refused");
    assert!(
        err.message.contains("too large"),
        "{command} must refuse past MAX_SPARSE_NNZ: {err:?}"
    );
    assert_eq!(err.code, "VELES-009", "{command}: {err:?}");
}

/// Every sparse command refuses a vector past core's NNZ cap, as REST does;
/// the plugin's own parser used to accept it.
#[test]
fn sparse_search_refuses_a_vector_past_the_nnz_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_docs(&app);
    let searched = run(sparse_search(
        app.handle().clone(),
        app.state(),
        req(serde_json::json!({ "collection": "docs", "sparseVector": oversized_sparse() })),
    ));
    assert_too_large(searched, "sparse_search");
}

#[test]
fn hybrid_sparse_search_refuses_a_vector_past_the_nnz_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_docs(&app);
    let hybrid = run(hybrid_sparse_search(
        app.handle().clone(),
        app.state(),
        req(serde_json::json!({
            "collection": "docs", "vector": [1.0, 0.0], "sparseVector": oversized_sparse()
        })),
    ));
    assert_too_large(hybrid, "hybrid_sparse_search");
}

#[test]
fn sparse_upsert_refuses_a_vector_past_the_nnz_cap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = mock_app(dir.path());
    seed_docs(&app);
    let upserted = run(sparse_upsert(
        app.handle().clone(),
        app.state(),
        req(serde_json::json!({
            "collection": "docs",
            "points": [{ "id": 9, "vector": [1.0, 0.0], "sparseVector": oversized_sparse() }]
        })),
    ));
    assert_too_large(upserted, "sparse_upsert");
}

// ===========================================================================
// train_pq: `m` is required
// ===========================================================================

/// `m` has no default — core's TRAIN refuses `m = 0`, and REST's `TRAIN
/// QUANTIZER` requires it — so a request without it is refused at the
/// boundary rather than typed optional and failing every time.
#[test]
fn train_pq_request_requires_m() {
    let missing = serde_json::from_value::<TrainPqRequest>(serde_json::json!({
        "collection": "docs"
    }))
    .expect_err("a request without `m` is refused");
    assert!(missing.to_string().contains("`m`"), "{missing}");

    let _given: TrainPqRequest = req(serde_json::json!({ "collection": "docs", "m": 2 }));
}
