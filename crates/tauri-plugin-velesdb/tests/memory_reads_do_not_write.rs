//! The read-only memory commands never create the memory collections.
//!
//! The memory handle used to be built on the first memory command of any
//! kind, and building it created `_semantic_memory`, `_episodic_memory` and
//! `_procedural_memory`: under the `read-only` permission set, a first
//! `semantic_query` wrote three collections. A read now opens only what
//! exists, and finds nothing when nothing was stored.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{App, Manager};

use tauri_plugin_velesdb::commands_memory::{
    episodic_older_than, episodic_recall_similar, episodic_recent, procedural_list_all,
    procedural_recall, semantic_dimension, semantic_query, semantic_store,
};
use tauri_plugin_velesdb::types::{
    EpisodicOlderThanRequest, EpisodicRecallSimilarRequest, EpisodicRecentRequest,
    ProceduralRecallRequest, SemanticQueryRequest, SemanticStoreRequest,
};
use tauri_plugin_velesdb::VelesDbState;

const DIM: usize = 4;

fn mock_app(dir: &std::path::Path) -> App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("test: build mock app");
    app.manage(VelesDbState::new(dir.to_path_buf()).with_memory_dimension(DIM));
    app
}

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tauri::async_runtime::block_on(fut)
}

fn collections(app: &App<tauri::test::MockRuntime>) -> Vec<String> {
    app.state::<VelesDbState>()
        .with_db(|db| Ok(db.list_collections()))
        .expect("test: list collections")
}

fn semantic_query_request(embedding: Vec<f32>) -> SemanticQueryRequest {
    SemanticQueryRequest {
        embedding,
        top_k: 5,
        quality: None,
    }
}

#[test]
fn every_memory_read_on_a_fresh_database_finds_nothing_and_creates_nothing() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let app = mock_app(dir.path());
    let h = || app.handle().clone();
    let st = || app.state::<VelesDbState>();
    let embedding = vec![0.5_f32; DIM];

    let semantic = run(semantic_query(
        h(),
        st(),
        semantic_query_request(embedding.clone()),
    ))
    .expect("semantic_query");
    let recent = run(episodic_recent(
        h(),
        st(),
        EpisodicRecentRequest {
            limit: 5,
            since_timestamp: None,
        },
    ))
    .expect("episodic_recent");
    let similar = run(episodic_recall_similar(
        h(),
        st(),
        EpisodicRecallSimilarRequest {
            embedding: embedding.clone(),
            top_k: 5,
        },
    ))
    .expect("episodic_recall_similar");
    let older = run(episodic_older_than(
        h(),
        st(),
        EpisodicOlderThanRequest {
            timestamp: i64::MAX,
            limit: 5,
        },
    ))
    .expect("episodic_older_than");
    let recalled = run(procedural_recall(
        h(),
        st(),
        ProceduralRecallRequest {
            embedding,
            top_k: 5,
            min_confidence: 0.0,
        },
    ))
    .expect("procedural_recall");
    let listed = run(procedural_list_all(h(), st())).expect("procedural_list_all");
    let dimension = run(semantic_dimension(h(), st())).expect("semantic_dimension");

    assert!(semantic.is_empty() && recent.is_empty() && similar.is_empty());
    assert!(older.is_empty() && recalled.is_empty() && listed.is_empty());
    assert_eq!(
        dimension, DIM,
        "the dimension the memory will be created with"
    );
    assert!(collections(&app).is_empty(), "{:?}", collections(&app));
}

/// Finding nothing does not loosen validation: an embedding of the wrong
/// size is refused as it is once the memory exists.
#[test]
fn a_read_on_a_fresh_database_still_checks_the_embedding_dimension() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let app = mock_app(dir.path());

    let err = run(semantic_query(
        app.handle().clone(),
        app.state::<VelesDbState>(),
        semantic_query_request(vec![0.5; DIM + 1]),
    ))
    .expect_err("a wrong-size embedding must be refused");

    assert!(err.message.contains("dimension"), "{err:?}");
    assert!(collections(&app).is_empty(), "{:?}", collections(&app));
}

/// Positive control: once a write created the memory, a read sees it.
#[test]
fn a_read_after_a_write_finds_what_was_stored() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let app = mock_app(dir.path());
    let embedding = vec![0.5_f32; DIM];

    run(semantic_store(
        app.handle().clone(),
        app.state::<VelesDbState>(),
        SemanticStoreRequest {
            id: 9,
            content: "stored".to_string(),
            embedding: embedding.clone(),
        },
    ))
    .expect("semantic_store");
    let found = run(semantic_query(
        app.handle().clone(),
        app.state::<VelesDbState>(),
        semantic_query_request(embedding),
    ))
    .expect("semantic_query");

    assert_eq!(found.iter().map(|r| r.id).collect::<Vec<_>>(), vec![9]);
}
