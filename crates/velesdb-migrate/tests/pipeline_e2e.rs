#![allow(clippy::pedantic)]

use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use serde_json::json;
use tempfile::{NamedTempFile, TempDir};
use velesdb_core::Database;
use velesdb_migrate::config::{
    DestinationConfig, DistanceMetric, MigrationConfig, MigrationOptions, PineconeConfig,
    QdrantConfig, SourceConfig, StorageMode,
};
use velesdb_migrate::connectors::elasticsearch::ElasticsearchConfig;
use velesdb_migrate::connectors::{csv_file::CsvFileConfig, json_file::JsonFileConfig};
use velesdb_migrate::Pipeline;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write_json_source(file: &mut NamedTempFile, rows: &[serde_json::Value]) {
    file.as_file_mut().set_len(0).expect("truncate json source");
    file.as_file_mut()
        .seek(SeekFrom::Start(0))
        .expect("seek json source");
    let json = serde_json::to_string(rows).expect("serialize json source");
    file.write_all(json.as_bytes()).expect("write json source");
    file.flush().expect("flush json source");
}

fn write_csv_source(file: &mut NamedTempFile, content: &str) {
    file.as_file_mut().set_len(0).expect("truncate csv source");
    file.as_file_mut()
        .seek(SeekFrom::Start(0))
        .expect("seek csv source");
    file.write_all(content.as_bytes())
        .expect("write csv source");
    file.flush().expect("flush csv source");
}

fn json_config(source_path: &Path, destination_path: &Path, collection: &str) -> MigrationConfig {
    MigrationConfig {
        source: SourceConfig::JsonFile(JsonFileConfig {
            path: source_path.to_path_buf(),
            array_path: String::new(),
            id_field: "id".to_string(),
            vector_field: "vector".to_string(),
            payload_fields: Vec::new(),
        }),
        destination: DestinationConfig {
            path: destination_path.to_path_buf(),
            collection: collection.to_string(),
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 2,
            workers: 1,
            ..MigrationOptions::default()
        },
    }
}

fn csv_config(source_path: &Path, destination_path: &Path, collection: &str) -> MigrationConfig {
    MigrationConfig {
        source: SourceConfig::CsvFile(CsvFileConfig {
            path: source_path.to_path_buf(),
            id_column: "id".to_string(),
            vector_column: "vector".to_string(),
            vector_spread: false,
            dim_prefix: "dim_".to_string(),
            delimiter: ',',
            has_header: true,
        }),
        destination: DestinationConfig {
            path: destination_path.to_path_buf(),
            collection: collection.to_string(),
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 2,
            workers: 1,
            ..MigrationOptions::default()
        },
    }
}

fn open_collection(db_path: &Path, name: &str) -> velesdb_core::Collection {
    Database::open(db_path)
        .expect("open database")
        .get_collection(name)
        .expect("collection exists")
}

#[tokio::test]
async fn test_pipeline_json_e2e_persists_and_reads_back() {
    let source = TempDir::new().expect("source dir");
    let destination = TempDir::new().expect("destination dir");
    let mut file = NamedTempFile::new_in(source.path()).expect("json source file");
    write_json_source(
        &mut file,
        &[
            serde_json::json!({"id": 1, "vector": [1.0, 0.0], "title": "Doc 1"}),
            serde_json::json!({"id": 2, "vector": [0.0, 1.0], "title": "Doc 2"}),
        ],
    );

    let config = json_config(file.path(), destination.path(), "json_docs");
    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 2);
    assert_eq!(stats.loaded, 2);
    assert_eq!(stats.failed, 0);

    let collection = open_collection(destination.path(), "json_docs");
    assert_eq!(collection.len(), 2);

    let points = collection.get(&[1, 2]);
    assert_eq!(
        points[0]
            .as_ref()
            .and_then(|point| point.payload.clone())
            .expect("payload")["title"],
        "Doc 1"
    );
    assert_eq!(
        points[1]
            .as_ref()
            .and_then(|point| point.payload.clone())
            .expect("payload")["title"],
        "Doc 2"
    );

    let mut ids = collection.all_ids();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
}

#[tokio::test]
async fn test_pipeline_csv_e2e_persists_and_reads_back() {
    let source = TempDir::new().expect("source dir");
    let destination = TempDir::new().expect("destination dir");
    let mut file = NamedTempFile::new_in(source.path()).expect("csv source file");
    write_csv_source(
        &mut file,
        "id,vector,title\n1,[0.1, 0.2],CSV Doc 1\n2,[0.2, 0.1],CSV Doc 2",
    );

    let config = csv_config(file.path(), destination.path(), "csv_docs");
    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.loaded, 2);

    let collection = open_collection(destination.path(), "csv_docs");
    assert_eq!(collection.len(), 2);
    let points = collection.get(&[1, 2]);
    assert_eq!(
        points[0]
            .as_ref()
            .and_then(|point| point.payload.clone())
            .expect("payload")["title"],
        "CSV Doc 1"
    );
}

#[tokio::test]
async fn test_pipeline_dry_run_does_not_create_collection() {
    let source = TempDir::new().expect("source dir");
    let destination = TempDir::new().expect("destination dir");
    let mut file = NamedTempFile::new_in(source.path()).expect("json source file");
    write_json_source(
        &mut file,
        &[serde_json::json!({"id": 1, "vector": [0.1, 0.2], "title": "Dry Run"})],
    );

    let mut config = json_config(file.path(), destination.path(), "dry_run_docs");
    config.options.dry_run = true;

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("dry run");

    assert_eq!(stats.loaded, 1);
    assert!(Database::open(destination.path())
        .expect("open database")
        .get_collection("dry_run_docs")
        .is_none());
}

#[tokio::test]
async fn test_pipeline_continue_on_error_skips_invalid_point() {
    let source = TempDir::new().expect("source dir");
    let destination = TempDir::new().expect("destination dir");
    let mut file = NamedTempFile::new_in(source.path()).expect("json source file");
    write_json_source(
        &mut file,
        &[
            serde_json::json!({"id": 1, "vector": [0.1, 0.2], "title": "Valid"}),
            serde_json::json!({"id": 2, "vector": [0.1, 0.2, 0.3], "title": "Invalid"}),
        ],
    );

    let mut config = json_config(file.path(), destination.path(), "fallback_docs");
    config.options.continue_on_error = true;

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.loaded, 1);
    assert_eq!(stats.failed, 1);

    let collection = open_collection(destination.path(), "fallback_docs");
    assert_eq!(collection.len(), 1);
    assert!(collection.get(&[1])[0].is_some());
    assert!(collection.get(&[2])[0].is_none());
}

#[tokio::test]
async fn test_pipeline_checkpoint_resumes_from_last_offset() {
    let source = TempDir::new().expect("source dir");
    let destination = TempDir::new().expect("destination dir");
    let mut file = NamedTempFile::new_in(source.path()).expect("json source file");
    let checkpoint_path = destination.path().join("resume-checkpoint.json");

    write_json_source(
        &mut file,
        &[
            serde_json::json!({"id": 1, "vector": [0.1, 0.2], "title": "First"}),
            serde_json::json!({"id": 2, "vector": [0.1, 0.2, 0.3], "title": "Broken"}),
            serde_json::json!({"id": 3, "vector": [0.2, 0.1], "title": "Third"}),
        ],
    );

    let mut config = json_config(file.path(), destination.path(), "resume_docs");
    config.options.batch_size = 1;
    config.options.checkpoint_path = Some(checkpoint_path.clone());

    let mut first_run = Pipeline::new(config.clone()).expect("pipeline");
    let error = first_run.run().await.expect_err("resume should fail");
    assert!(matches!(error, velesdb_migrate::Error::Loading(_)));
    assert!(checkpoint_path.exists());

    write_json_source(
        &mut file,
        &[
            serde_json::json!({"id": 1, "vector": [0.1, 0.2], "title": "First"}),
            serde_json::json!({"id": 2, "vector": [0.2, 0.2], "title": "Second"}),
            serde_json::json!({"id": 3, "vector": [0.2, 0.1], "title": "Third"}),
        ],
    );

    let mut resumed_run = Pipeline::new(config).expect("pipeline");
    let stats = resumed_run.run().await.expect("resume succeeds");

    assert_eq!(stats.loaded, 3);
    assert_eq!(stats.failed, 0);
    assert!(!checkpoint_path.exists());

    let collection = open_collection(destination.path(), "resume_docs");
    assert_eq!(collection.len(), 3);
}

#[tokio::test]
async fn test_pipeline_workers_match_sequential_results_and_text_ids_stable() {
    let source = TempDir::new().expect("source dir");
    let destination_one = TempDir::new().expect("destination dir one");
    let destination_two = TempDir::new().expect("destination dir two");
    let mut file = NamedTempFile::new_in(source.path()).expect("json source file");
    write_json_source(
        &mut file,
        &[
            serde_json::json!({"id": "alpha", "vector": [1.0, 0.0], "title": "Alpha"}),
            serde_json::json!({"id": "beta", "vector": [0.0, 1.0], "title": "Beta"}),
            serde_json::json!({"id": "gamma", "vector": [0.5, 0.5], "title": "Gamma"}),
        ],
    );

    let mut sequential_config = json_config(file.path(), destination_one.path(), "worker_docs");
    sequential_config.options.workers = 1;

    let mut parallel_config = json_config(file.path(), destination_two.path(), "worker_docs");
    parallel_config.options.workers = 4;

    Pipeline::new(sequential_config)
        .expect("pipeline")
        .run()
        .await
        .expect("sequential run");
    Pipeline::new(parallel_config)
        .expect("pipeline")
        .run()
        .await
        .expect("parallel run");

    let sequential = open_collection(destination_one.path(), "worker_docs");
    let parallel = open_collection(destination_two.path(), "worker_docs");

    let mut sequential_ids = sequential.all_ids();
    let mut parallel_ids = parallel.all_ids();
    sequential_ids.sort_unstable();
    parallel_ids.sort_unstable();

    assert_eq!(sequential_ids, parallel_ids);

    assert_eq!(sequential_ids.len(), 3);
}

#[tokio::test]
async fn test_pipeline_qdrant_sparse_vectors_e2e() {
    let mock_server = MockServer::start().await;

    let collection_info_body = json!({
        "result": {
            "vectors_count": 2,
            "points_count": 2,
            "config": {
                "params": {
                    "vectors": {
                        "size": 3
                    }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/collections/test_sparse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&collection_info_body))
        .expect(2) // connect() + get_schema()
        .mount(&mock_server)
        .await;

    let scroll_body = json!({
        "result": {
            "points": [
                {
                    "id": 1,
                    "vector": {
                        "dense": [0.1, 0.2, 0.3],
                        "sparse": {"indices": [10, 45, 16], "values": [0.5, 0.5, 0.2]}
                    },
                    "payload": {"title": "Doc with sparse"}
                },
                {
                    "id": 2,
                    "vector": [0.4, 0.5, 0.6],
                    "payload": {"title": "Doc dense only"}
                }
            ],
            "next_page_offset": null
        }
    });

    Mock::given(method("POST"))
        .and(path("/collections/test_sparse/points/scroll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&scroll_body))
        .expect(1)
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: mock_server.uri(),
            collection: "test_sparse".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "sparse_docs".to_string(),
            dimension: 3,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 10,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 2);
    assert_eq!(stats.loaded, 2);
    assert_eq!(stats.failed, 0);

    let collection = open_collection(destination.path(), "sparse_docs");
    assert_eq!(collection.len(), 2);

    let points = collection.get(&[1, 2]);
    assert_eq!(
        points[0]
            .as_ref()
            .and_then(|point| point.payload.clone())
            .expect("payload for point 1")["title"],
        "Doc with sparse"
    );
    assert_eq!(
        points[1]
            .as_ref()
            .and_then(|point| point.payload.clone())
            .expect("payload for point 2")["title"],
        "Doc dense only"
    );
}

/// Verify that an HTTP 500 on the Qdrant scroll endpoint fails the pipeline
/// immediately with no silent retry. The `expect(1)` on the scroll mock is the
/// key assertion: it proves the connector does not retry on server errors.
#[tokio::test]
async fn test_pipeline_qdrant_500_fails_pipeline() {
    let mock_server = MockServer::start().await;

    let collection_info_body = json!({
        "result": {
            "vectors_count": 10,
            "points_count": 10,
            "config": {
                "params": {
                    "vectors": { "size": 3 }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/collections/test_err"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&collection_info_body))
        .expect(2) // connect() + get_schema()
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/collections/test_err/points/scroll"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .expect(1) // proves no retry
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: mock_server.uri(),
            collection: "test_err".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "err_docs".to_string(),
            dimension: 3,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 10,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let result = pipeline.run().await;

    assert!(result.is_err(), "pipeline must fail on HTTP 500");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("500") || err_msg.contains("scroll failed"),
        "error message should mention 500 or scroll failure, got: {err_msg}"
    );
}

/// Verify that an HTTP 429 on the Qdrant scroll endpoint fails the pipeline
/// immediately. The connector does not implement `with_retry`; this test
/// documents that behavior and ensures no accidental retry loop is introduced.
#[tokio::test]
async fn test_pipeline_qdrant_429_fails_pipeline() {
    let mock_server = MockServer::start().await;

    let collection_info_body = json!({
        "result": {
            "vectors_count": 10,
            "points_count": 10,
            "config": {
                "params": {
                    "vectors": { "size": 3 }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/collections/test_rate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&collection_info_body))
        .expect(2) // connect() + get_schema()
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/collections/test_rate/points/scroll"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_string("Too Many Requests"),
        )
        .expect(1) // proves no retry
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: mock_server.uri(),
            collection: "test_rate".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "rate_docs".to_string(),
            dimension: 3,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 10,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let result = pipeline.run().await;

    assert!(result.is_err(), "pipeline must fail on HTTP 429");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("429") || err_msg.contains("scroll failed"),
        "error should be an Extraction error mentioning 429, got: {err_msg}"
    );
}

/// Verify that a dimension mismatch between the Qdrant source (dimension=5)
/// and the destination config (dimension=3) is caught before any data transfer.
/// No scroll mock is registered — the pipeline must abort before calling it.
#[tokio::test]
async fn test_pipeline_qdrant_schema_mismatch() {
    let mock_server = MockServer::start().await;

    let collection_info_body = json!({
        "result": {
            "vectors_count": 10,
            "points_count": 10,
            "config": {
                "params": {
                    "vectors": { "size": 5 }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/collections/test_dim"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&collection_info_body))
        .expect(2) // connect() + get_schema()
        .mount(&mock_server)
        .await;

    // No scroll mock — pipeline should abort before reaching extraction.

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: mock_server.uri(),
            collection: "test_dim".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "dim_docs".to_string(),
            dimension: 3, // mismatch with source=5
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 10,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let result = pipeline.run().await;

    assert!(result.is_err(), "pipeline must fail on dimension mismatch");
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains('5') && err_msg.contains('3'),
        "error should mention both dimensions (5 and 3), got: {err_msg}"
    );
    assert!(
        matches!(err, velesdb_migrate::Error::SchemaMismatch(_)),
        "error variant should be SchemaMismatch, got: {err:?}"
    );

    // No collection should have been created on disk.
    assert!(
        velesdb_core::Database::open(destination.path())
            .expect("open database")
            .get_collection("dim_docs")
            .is_none(),
        "no collection should exist after schema mismatch"
    );
}

#[tokio::test]
async fn test_pipeline_pinecone_sparse_vectors_e2e() {
    let mock_server = MockServer::start().await;

    // Extract host:port from mock server URI (strip the "http://" scheme).
    let mock_host = mock_server
        .uri()
        .strip_prefix("http://")
        .expect("mock URI has http scheme")
        .to_string();

    // 1. GET /indexes/test-index → IndexDescription
    Mock::given(method("GET"))
        .and(path("/indexes/test-index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "host": mock_host,
            "dimension": 3,
            "metric": "cosine",
            "status": { "ready": true }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 2. POST /describe_index_stats → IndexStats
    Mock::given(method("POST"))
        .and(path("/describe_index_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dimension": 3,
            "totalVectorCount": 2
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 3. GET /vectors/list → ListResponse
    Mock::given(method("GET"))
        .and(path("/vectors/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": [{"id": "vec-1"}, {"id": "vec-2"}],
            "pagination": null
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 4. GET /vectors/fetch → FetchResponse (with sparse vectors)
    Mock::given(method("GET"))
        .and(path("/vectors/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": {
                "vec-1": {
                    "id": "vec-1",
                    "values": [0.1, 0.2, 0.3],
                    "sparseValues": {"indices": [0, 5, 12], "values": [0.9, 0.4, 0.7]},
                    "metadata": {"title": "Sparse doc"}
                },
                "vec-2": {
                    "id": "vec-2",
                    "values": [0.4, 0.5, 0.6],
                    "metadata": {"title": "Dense only doc"}
                }
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Pinecone(PineconeConfig {
            api_key: "test-key".to_string(),
            index: "test-index".to_string(),
            base_url: Some(mock_server.uri()),
            ..Default::default()
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "pinecone_sparse_docs".to_string(),
            dimension: 3,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 10,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 2);
    assert_eq!(stats.loaded, 2);
    assert_eq!(stats.failed, 0);

    let collection = open_collection(destination.path(), "pinecone_sparse_docs");
    assert_eq!(collection.len(), 2);
}

#[tokio::test]
async fn test_pipeline_elasticsearch_e2e() {
    let mock_server = MockServer::start().await;
    let destination = TempDir::new().expect("destination dir");

    // -- Mock: _count endpoint (separate path, no ordering concerns) --
    Mock::given(method("POST"))
        .and(path("/test-docs/_count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 3
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // -- Mock: _search endpoint --
    // wiremock LIFO: last mounted = first matched.
    // Call order: probe (size=1) -> page-1 (size=2) -> page-2 (size=2, search_after)

    // 1. Mount page-2 first (fires last — fallback for size=2 with search_after)
    Mock::given(method("POST"))
        .and(path("/test-docs/_search"))
        .and(body_partial_json(json!({ "search_after": ["doc-002"] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": {
                "total": { "value": 3 },
                "hits": [
                    {
                        "_id": "doc-003",
                        "_source": {
                            "embedding": [0.7, 0.8, 0.9],
                            "title": "Doc 3"
                        },
                        "sort": ["doc-003"]
                    }
                ]
            }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 2. Mount page-1 second (fires before page-2 for size=2 without search_after)
    Mock::given(method("POST"))
        .and(path("/test-docs/_search"))
        .and(body_partial_json(json!({ "size": 2 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": {
                "total": { "value": 3 },
                "hits": [
                    {
                        "_id": "doc-001",
                        "_source": {
                            "embedding": [0.1, 0.2, 0.3],
                            "title": "Doc 1"
                        },
                        "sort": ["doc-001"]
                    },
                    {
                        "_id": "doc-002",
                        "_source": {
                            "embedding": [0.4, 0.5, 0.6],
                            "title": "Doc 2"
                        },
                        "sort": ["doc-002"]
                    }
                ]
            }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // 3. Mount probe last (fires first — connect's schema probe, size=1)
    Mock::given(method("POST"))
        .and(path("/test-docs/_search"))
        .and(body_partial_json(json!({ "size": 1 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "hits": {
                "total": { "value": 3 },
                "hits": [
                    {
                        "_id": "doc-001",
                        "_source": {
                            "embedding": [0.1, 0.2, 0.3],
                            "title": "Doc 1"
                        },
                        "sort": ["doc-001"]
                    }
                ]
            }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    let config = MigrationConfig {
        source: SourceConfig::Elasticsearch(ElasticsearchConfig {
            url: mock_server.uri(),
            index: "test-docs".to_string(),
            vector_field: "embedding".to_string(),
            id_field: "_id".to_string(),
            payload_fields: vec![],
            username: None,
            password: None,
            api_key: None,
            query: None,
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "es_docs".to_string(),
            dimension: 3,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 2,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 3);
    assert_eq!(stats.loaded, 3);
    assert_eq!(stats.failed, 0);

    let collection = open_collection(destination.path(), "es_docs");
    assert_eq!(collection.len(), 3);
}

#[tokio::test]
async fn test_pipeline_qdrant_pagination_multi_page() {
    let mock_server = MockServer::start().await;

    let collection_info_body = json!({
        "result": {
            "vectors_count": 4,
            "points_count": 4,
            "config": {
                "params": {
                    "vectors": {
                        "size": 2
                    }
                }
            }
        }
    });

    Mock::given(method("GET"))
        .and(path("/collections/test_pag"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&collection_info_body))
        .expect(2) // connect() + get_schema()
        .mount(&mock_server)
        .await;

    // Page 1: no offset in body -- return first 2 points with next_page_offset.
    let page1_body = json!({
        "result": {
            "points": [
                {
                    "id": 1,
                    "vector": [0.1, 0.2],
                    "payload": {"p": "a"}
                },
                {
                    "id": 2,
                    "vector": [0.3, 0.4],
                    "payload": {"p": "b"}
                }
            ],
            "next_page_offset": 3
        }
    });

    Mock::given(method("POST"))
        .and(path("/collections/test_pag/points/scroll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page1_body))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Page 2: offset=3 in body -- return last 2 points, no next page.
    let page2_body = json!({
        "result": {
            "points": [
                {
                    "id": 3,
                    "vector": [0.5, 0.6],
                    "payload": {"p": "c"}
                },
                {
                    "id": 4,
                    "vector": [0.7, 0.8],
                    "payload": {"p": "d"}
                }
            ],
            "next_page_offset": null
        }
    });

    Mock::given(method("POST"))
        .and(path("/collections/test_pag/points/scroll"))
        .and(body_partial_json(json!({"offset": 3})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page2_body))
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Qdrant(QdrantConfig {
            url: mock_server.uri(),
            collection: "test_pag".to_string(),
            api_key: None,
            payload_fields: vec![],
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "qdrant_pag_docs".to_string(),
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 2,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 4);
    assert_eq!(stats.loaded, 4);
    assert_eq!(stats.failed, 0);
    assert_eq!(stats.batches, 2);

    let collection = open_collection(destination.path(), "qdrant_pag_docs");
    assert_eq!(collection.len(), 4);
}

#[tokio::test]
async fn test_pipeline_pinecone_pagination_multi_page() {
    let mock_server = MockServer::start().await;

    let mock_host = mock_server
        .uri()
        .strip_prefix("http://")
        .expect("mock URI has http scheme")
        .to_string();

    // 1. GET /indexes/test-pag -- IndexDescription
    Mock::given(method("GET"))
        .and(path("/indexes/test-pag"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "host": mock_host,
            "dimension": 2,
            "metric": "cosine",
            "status": { "ready": true }
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 2. POST /describe_index_stats -- IndexStats
    Mock::given(method("POST"))
        .and(path("/describe_index_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dimension": 2,
            "totalVectorCount": 4
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    // 3. GET /vectors/list -- Page 1 (drain with up_to_n_times(1))
    Mock::given(method("GET"))
        .and(path("/vectors/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": [{"id": "vec-1"}, {"id": "vec-2"}],
            "pagination": {"next": "page2-token"}
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // 4. GET /vectors/list -- Page 2
    Mock::given(method("GET"))
        .and(path("/vectors/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": [{"id": "vec-3"}, {"id": "vec-4"}],
            "pagination": null
        })))
        .mount(&mock_server)
        .await;

    // 5. GET /vectors/fetch -- Fetch 1 (vec-1, vec-2)
    Mock::given(method("GET"))
        .and(path("/vectors/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": {
                "vec-1": {
                    "id": "vec-1",
                    "values": [0.1, 0.2],
                    "metadata": {"p": "a"}
                },
                "vec-2": {
                    "id": "vec-2",
                    "values": [0.3, 0.4],
                    "metadata": {"p": "b"}
                }
            }
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // 6. GET /vectors/fetch -- Fetch 2 (vec-3, vec-4)
    Mock::given(method("GET"))
        .and(path("/vectors/fetch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": {
                "vec-3": {
                    "id": "vec-3",
                    "values": [0.5, 0.6],
                    "metadata": {"p": "c"}
                },
                "vec-4": {
                    "id": "vec-4",
                    "values": [0.7, 0.8],
                    "metadata": {"p": "d"}
                }
            }
        })))
        .mount(&mock_server)
        .await;

    let destination = TempDir::new().expect("destination dir");

    let config = MigrationConfig {
        source: SourceConfig::Pinecone(PineconeConfig {
            base_url: Some(mock_server.uri()),
            api_key: "test".to_string(),
            index: "test-pag".to_string(),
            ..Default::default()
        }),
        destination: DestinationConfig {
            path: destination.path().to_path_buf(),
            collection: "pinecone_pag_docs".to_string(),
            dimension: 2,
            metric: DistanceMetric::Cosine,
            storage_mode: StorageMode::Full,
        },
        options: MigrationOptions {
            batch_size: 2,
            workers: 1,
            ..MigrationOptions::default()
        },
    };

    let mut pipeline = Pipeline::new(config).expect("pipeline");
    let stats = pipeline.run().await.expect("pipeline run");

    assert_eq!(stats.extracted, 4);
    assert_eq!(stats.loaded, 4);
    assert_eq!(stats.failed, 0);

    let collection = open_collection(destination.path(), "pinecone_pag_docs");
    assert_eq!(collection.len(), 4);
}
