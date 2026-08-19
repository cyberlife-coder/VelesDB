use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;

use super::{duration_millis, McpServer, MigrationStartParams};
use crate::{DynEmbedder, HashEmbedder, MemoryService};

const FUNCTIONAL_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_start_returns_immediately_and_status_reaches_committed() {
    let root = tempfile::tempdir().expect("root");
    let source_path = root.path().join("source");
    let source_embedder: DynEmbedder = Box::new(HashEmbedder::new(2));
    let source = MemoryService::open(&source_path, source_embedder).expect("source");
    let server = McpServer::new(source)
        .with_embedder_identity("source-model", 2)
        .with_online_migration(&source_path, |backend| {
            assert_eq!(backend, "hash-3");
            Ok((
                Box::new(HashEmbedder::new(3)) as DynEmbedder,
                "target-model".to_owned(),
            ))
        })
        .expect("migration manager");
    let accepted = server
        .migration_start(Parameters(MigrationStartParams {
            target_backend: "hash-3".to_owned(),
            pause_budget_ms: duration_millis(FUNCTIONAL_TIMEOUT),
            journal_max_bytes: Some(1024 * 1024),
            fact_batch: Some(16),
            replay_batch: Some(16),
            edge_cap: Some(16),
            observation_window: Some(2),
            verification_reserve_ms: Some(10),
        }))
        .await
        .expect("start");
    assert!(accepted.0.configured);
    assert!(accepted.0.job.expect("job").running);
    let deadline = std::time::Instant::now() + FUNCTIONAL_TIMEOUT;
    loop {
        let status = server.migration_status().await.expect("status").0;
        let job = status.job.expect("job");
        if job.phase == "committed" {
            assert!(job.progress.measured_cutover_ms.is_some());
            break;
        }
        assert!(std::time::Instant::now() < deadline, "phase: {}", job.phase);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
