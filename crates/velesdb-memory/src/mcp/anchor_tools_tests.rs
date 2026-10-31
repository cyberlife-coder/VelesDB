//! The anchor parameters of `remember` and `recall`, end to end through the
//! MCP tools: the server reads anchored files only inside the ingest roots,
//! and a recall against a checkout reports each anchored fact's freshness.

use std::fs;
use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorCode;
use serde_json::Value;
use tempfile::TempDir;

use super::dto::{RecallParams, RememberParams};
use super::McpServer;
use crate::context::IngestRoots;
use crate::embedder::{DynEmbedder, HashEmbedder};
use crate::service::MemoryService;

const LESSON: &str = "the scheduler module never blocks inside the tokio runtime";

/// A store, and a server allowlisting `allowed` when one is given.
fn anchored_server(allowed: Option<&Path>) -> (TempDir, McpServer) {
    let store = TempDir::new().expect("store dir");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let server = McpServer::new(MemoryService::open(store.path(), embedder).expect("open"));
    let server = match allowed {
        Some(dir) => {
            let roots = IngestRoots::parse(dir.to_str().expect("utf-8")).expect("valid root");
            server.with_ingest_roots(roots)
        }
        None => server,
    };
    (store, server)
}

/// A checkout with one source file, under its own allowlisted directory.
fn checkout() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("checkout dir");
    let repo = fs::canonicalize(dir.path())
        .expect("canonical")
        .join("repo");
    fs::create_dir_all(repo.join("src")).expect("tree");
    fs::write(repo.join("src/sched.rs"), "fn tick() {}").expect("file");
    (dir, repo)
}

fn remember(root: Option<&Path>, anchors: &[&str]) -> RememberParams {
    RememberParams {
        fact: LESSON.to_owned(),
        links: Vec::new(),
        metadata: None,
        ttl_seconds: None,
        anchors: anchors.iter().map(|path| (*path).to_owned()).collect(),
        anchor_root: root.map(|root| root.to_str().expect("utf-8").to_owned()),
    }
}

async fn recall_json(server: &McpServer, root: Option<&Path>) -> Value {
    let Json(result) = server
        .recall(Parameters(RecallParams {
            query: LESSON.to_owned(),
            limit: Some(1),
            filter: None,
            anchor_root: root.map(|root| root.to_str().expect("utf-8").to_owned()),
        }))
        .await
        .expect("recall");
    serde_json::to_value(result).expect("serializable")
}

#[tokio::test]
async fn anchoring_without_ingest_roots_is_refused_and_stores_nothing() {
    let (_store, server) = anchored_server(None);
    let (_dir, repo) = checkout();
    let refused = server
        .remember(Parameters(remember(Some(&repo), &["src/sched.rs"])))
        .await
        .err()
        .expect("anchoring must be refused without an allowlist");
    assert_eq!(refused.code, ErrorCode::INVALID_PARAMS);
    assert!(
        refused.message.contains("VELESDB_MEMORY_INGEST_ROOTS"),
        "{}",
        refused.message
    );
    assert_eq!(
        recall_json(&server, None).await["memories"],
        Value::Array(vec![])
    );
}

#[tokio::test]
async fn anchor_paths_without_a_root_or_escaping_it_are_refused() {
    let (dir, repo) = checkout();
    let (_store, server) = anchored_server(Some(dir.path()));
    for params in [
        remember(None, &["src/sched.rs"]),
        remember(Some(&repo), &["../outside.rs"]),
        remember(Some(&repo), &["/etc/hostname"]),
    ] {
        let refused = server.remember(Parameters(params)).await.err();
        assert_eq!(refused.map(|err| err.code), Some(ErrorCode::INVALID_PARAMS));
    }
    let outside = TempDir::new().expect("outside");
    let refused = server
        .remember(Parameters(remember(Some(outside.path()), &["x.rs"])))
        .await
        .err();
    assert_eq!(refused.map(|err| err.code), Some(ErrorCode::INVALID_PARAMS));
}

#[tokio::test]
async fn recall_against_the_checkout_reports_freshness_then_staleness() {
    let (dir, repo) = checkout();
    let (_store, server) = anchored_server(Some(dir.path()));
    server
        .remember(Parameters(remember(Some(&repo), &["src/sched.rs"])))
        .await
        .expect("anchored remember");

    let plain = recall_json(&server, None).await;
    assert!(plain["memories"][0].get("freshness").is_none(), "{plain}");

    let fresh = recall_json(&server, Some(&repo)).await;
    assert_eq!(
        fresh["memories"][0]["freshness"]["state"], "fresh",
        "{fresh}"
    );

    fs::write(repo.join("src/sched.rs"), "fn tick() { block_on(work()) }").expect("edit");
    let stale = recall_json(&server, Some(&repo)).await;
    assert_eq!(
        stale["memories"][0]["freshness"]["state"], "stale",
        "{stale}"
    );
    assert_eq!(
        stale["memories"][0]["freshness"]["changed"][0],
        "src/sched.rs"
    );
}

/// N7: `anchor_root` without `anchors` is validated anyway — refused when
/// anchoring is disabled or the root is outside the allowlist — and stores
/// nothing, instead of being silently ignored.
#[tokio::test]
async fn a_root_without_anchors_is_validated_not_ignored() {
    let (dir, repo) = checkout();
    let outside = TempDir::new().expect("outside");
    let (_store, disabled) = anchored_server(None);
    let (_store2, allowlisted) = anchored_server(Some(dir.path()));
    for (server, root) in [(&disabled, repo.as_path()), (&allowlisted, outside.path())] {
        let refused = server
            .remember(Parameters(remember(Some(root), &[])))
            .await
            .err()
            .expect("an invalid anchor_root is refused even without anchors");
        assert_eq!(
            refused.code,
            ErrorCode::INVALID_PARAMS,
            "{}",
            refused.message
        );
        assert_eq!(
            recall_json(server, None).await["memories"],
            Value::Array(vec![])
        );
    }
    allowlisted
        .remember(Parameters(remember(Some(&repo), &[])))
        .await
        .expect("a valid root without anchors is a plain remember");
}

/// S3 over the wire: a recall naming another checkout reports `unverified`
/// for a fact anchored elsewhere, never `stale`.
#[tokio::test]
async fn recall_against_another_checkout_reports_unverified() {
    let (dir, repo) = checkout();
    let other = dir.path().join("other-clone");
    fs::create_dir_all(other.join("src")).expect("other tree");
    fs::write(other.join("src/sched.rs"), "fn tick() { different() }").expect("other file");
    let (_store, server) = anchored_server(Some(dir.path()));
    server
        .remember(Parameters(remember(Some(&repo), &["src/sched.rs"])))
        .await
        .expect("anchored remember");
    let elsewhere = recall_json(&server, Some(&other)).await;
    assert_eq!(
        elsewhere["memories"][0]["freshness"]["state"], "unverified",
        "{elsewhere}"
    );
}
