//! Wiremock BDD tests for the Sprint 1.5 connector metric
//! introspection hardening.
//!
//! Each test mounts a wiremock server that serves fixture responses
//! captured from the official documentation of the target backend
//! (Qdrant, Weaviate, Milvus, Elasticsearch, Pinecone), constructs
//! the connector pointing at the mock, runs `connect()` +
//! `get_schema()`, and asserts that `SourceSchema.metric` carries
//! the normalised VelesDB core identifier.
//!
//! The fixtures live in `tests/fixtures/` and are shaped after the
//! verbatim examples in the official API docs — not hand-rolled
//! mocks. This matters: if we mocked our own expected shape and
//! then tested against it, the test would be a self-fulfilling
//! tautology. Using docs-backed fixtures catches drift between
//! our parser and the real API surface.

#![allow(clippy::pedantic)]

use std::path::PathBuf;

use serde_json::Value;
use velesdb_migrate::config::{MilvusConfig, QdrantConfig, SourceConfig};
use velesdb_migrate::connectors::create_connector;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Starts a wiremock server and opts the test process into the
/// SSRF escape hatch so `validate_url` accepts the loopback URL.
async fn start_mock_server() -> MockServer {
    // SAFETY: set_var is unsafe on newer Rust editions because it is
    // not thread-safe. These integration tests run under
    // --test-threads=1, so no other thread can observe the store.
    unsafe {
        std::env::set_var("VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS", "1");
    }
    MockServer::start().await
}

fn fixture_path(name: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("tests").join("fixtures").join(name)
}

fn load_fixture(name: &str) -> Value {
    let path = fixture_path(name);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading fixture {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parsing fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Milvus
// ---------------------------------------------------------------------------

/// GIVEN a Milvus server returning a collection with a COSINE-indexed
/// vector field,
/// WHEN the connector calls `connect()` (which fetches and caches
/// the schema),
/// THEN `SourceSchema.metric` must carry `"cosine"`.
///
/// The fixture is the verbatim example from the Milvus v2.5 REST
/// docs (`POST /v2/vectordb/collections/describe`) with an index
/// whose `metricType = "COSINE"`. This test would have caught the
/// S1.5-03 initial implementation bug where the connector chased
/// a non-existent `/indexes/describe` endpoint and always returned
/// `metric: None`.
#[tokio::test]
async fn milvus_schema_has_cosine_metric_when_index_cosine() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/has"))
        .and(query_param("collectionName", "test_collection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_has_true.json")))
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/describe"))
        .and(query_param("collectionName", "test_collection"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(load_fixture("milvus_describe_cosine.json")),
        )
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/stats"))
        .and(query_param("collectionName", "test_collection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_stats.json")))
        .mount(&mock)
        .await;

    let config = SourceConfig::Milvus(MilvusConfig {
        url: mock.uri(),
        collection: "test_collection".to_string(),
        username: None,
        password: None,
    });

    let mut connector = create_connector(&config).expect("create Milvus connector");
    connector.connect().await.expect("Milvus connect should succeed");

    let schema = connector.get_schema().await.expect("Milvus get_schema");
    assert_eq!(
        schema.metric.as_deref(),
        Some("cosine"),
        "Milvus connector must forward the COSINE index metric, got {:?}",
        schema.metric
    );
    assert_eq!(schema.dimension, 128, "dim from fixture params");
    assert_eq!(schema.total_count, Some(1000), "total_count from stats");
}

/// GIVEN a Milvus server whose index uses `metricType = "L2"`,
/// WHEN schema extraction runs,
/// THEN the metric must be normalised to `"euclidean"` and the
/// index whose `fieldName` matches a non-default name is still
/// resolved.
#[tokio::test]
async fn milvus_schema_normalises_l2_to_euclidean_with_custom_index_name() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/has"))
        .and(query_param("collectionName", "test_l2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_has_true.json")))
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/describe"))
        .and(query_param("collectionName", "test_l2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(load_fixture("milvus_describe_l2.json")),
        )
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/stats"))
        .and(query_param("collectionName", "test_l2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_stats.json")))
        .mount(&mock)
        .await;

    let config = SourceConfig::Milvus(MilvusConfig {
        url: mock.uri(),
        collection: "test_l2".to_string(),
        username: None,
        password: None,
    });

    let mut connector = create_connector(&config).expect("create Milvus connector");
    connector.connect().await.expect("Milvus connect");
    let schema = connector.get_schema().await.expect("Milvus get_schema");
    assert_eq!(schema.metric.as_deref(), Some("euclidean"));
    assert_eq!(schema.dimension, 64);
}

// ---------------------------------------------------------------------------
// Qdrant
// ---------------------------------------------------------------------------

/// GIVEN a Qdrant collection configured with a single unnamed
/// vector using the Cosine distance,
/// WHEN `connect()` + `get_schema()` run against the mock,
/// THEN `SourceSchema.metric` must carry `"cosine"` and
/// `dimension` must equal the fixture's `size` field.
#[tokio::test]
async fn qdrant_schema_has_cosine_metric_for_single_vector() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/collections/single_cosine"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(load_fixture("qdrant_describe_single_cosine.json")),
        )
        .mount(&mock)
        .await;

    let config = SourceConfig::Qdrant(QdrantConfig {
        url: mock.uri(),
        collection: "single_cosine".to_string(),
        api_key: None,
        payload_fields: vec![],
    });

    let mut connector = create_connector(&config).expect("create Qdrant connector");
    connector.connect().await.expect("Qdrant connect");
    let schema = connector.get_schema().await.expect("Qdrant get_schema");
    assert_eq!(
        schema.metric.as_deref(),
        Some("cosine"),
        "single-vector Cosine must normalise to 'cosine', got {:?}",
        schema.metric
    );
    assert_eq!(schema.dimension, 384);
    assert_eq!(schema.total_count, Some(1500));
}

/// GIVEN a Qdrant 1.7+ collection with named vectors where a
/// `default` entry uses `Euclid` and a `secondary` entry uses
/// `Cosine`,
/// WHEN the connector extracts the schema,
/// THEN it must pick the `default` entry (policy) and normalise
/// `Euclid` → `euclidean`.
#[tokio::test]
async fn qdrant_schema_picks_default_named_vector_and_normalises_euclid() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/collections/named_default_euclid"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(load_fixture("qdrant_describe_named_default_euclid.json")),
        )
        .mount(&mock)
        .await;

    let config = SourceConfig::Qdrant(QdrantConfig {
        url: mock.uri(),
        collection: "named_default_euclid".to_string(),
        api_key: None,
        payload_fields: vec![],
    });

    let mut connector = create_connector(&config).expect("create Qdrant connector");
    connector.connect().await.expect("Qdrant connect");
    let schema = connector.get_schema().await.expect("Qdrant get_schema");
    assert_eq!(
        schema.metric.as_deref(),
        Some("euclidean"),
        "named 'default' Euclid must normalise to 'euclidean', got {:?}",
        schema.metric
    );
    assert_eq!(
        schema.dimension, 768,
        "must pick 'default' vector dimension, not 'secondary'"
    );
}

/// GIVEN a Qdrant collection with the `Manhattan` distance (1.8+),
/// WHEN extraction runs,
/// THEN the metric must be preserved verbatim as `"manhattan"`
/// (not a VelesDB core identifier) so `check_metric_fidelity`
/// can surface the mismatch honestly rather than silently
/// dropping it.
#[tokio::test]
async fn qdrant_schema_preserves_manhattan_verbatim() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/collections/manhattan_col"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(load_fixture("qdrant_describe_manhattan.json")),
        )
        .mount(&mock)
        .await;

    let config = SourceConfig::Qdrant(QdrantConfig {
        url: mock.uri(),
        collection: "manhattan_col".to_string(),
        api_key: None,
        payload_fields: vec![],
    });

    let mut connector = create_connector(&config).expect("create Qdrant connector");
    connector.connect().await.expect("Qdrant connect");
    let schema = connector.get_schema().await.expect("Qdrant get_schema");
    assert_eq!(
        schema.metric.as_deref(),
        Some("manhattan"),
        "Manhattan must be preserved verbatim, got {:?}",
        schema.metric
    );
}

// ---------------------------------------------------------------------------
// Milvus (continued)
// ---------------------------------------------------------------------------

/// GIVEN a Milvus collection without any index yet (e.g. newly
/// created, before FT.CREATE-equivalent runs),
/// WHEN the connector extracts the schema,
/// THEN `metric` must be `None` so `check_metric_fidelity` skips
/// validation honestly rather than fabricating a fake default.
#[tokio::test]
async fn milvus_schema_metric_is_none_when_no_index_present() {
    let mock = start_mock_server().await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/has"))
        .and(query_param("collectionName", "test_no_index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_has_true.json")))
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/describe"))
        .and(query_param("collectionName", "test_no_index"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(load_fixture("milvus_describe_no_index.json")),
        )
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/v2/vectordb/collections/stats"))
        .and(query_param("collectionName", "test_no_index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(load_fixture("milvus_stats.json")))
        .mount(&mock)
        .await;

    let config = SourceConfig::Milvus(MilvusConfig {
        url: mock.uri(),
        collection: "test_no_index".to_string(),
        username: None,
        password: None,
    });

    let mut connector = create_connector(&config).expect("create Milvus connector");
    connector.connect().await.expect("Milvus connect");
    let schema = connector.get_schema().await.expect("Milvus get_schema");
    assert!(
        schema.metric.is_none(),
        "no index → metric must be None, got {:?}",
        schema.metric
    );
}
