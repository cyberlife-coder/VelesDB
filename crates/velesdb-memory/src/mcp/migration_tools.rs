use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::ErrorCode;
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{to_error, McpServer};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::service::{JobPhase, MigrationStartConfig, MigrationStatus, OnlineMigrationManager};
use crate::DynEmbedder;

const DEFAULT_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_FACT_BATCH: usize = 256;
const DEFAULT_REPLAY_BATCH: usize = 256;
const DEFAULT_EDGE_CAP: usize = 4_096;
const DEFAULT_OBSERVATION_WINDOW: usize = 3;
const DEFAULT_VERIFICATION_RESERVE_MS: u64 = 100;

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
struct MigrationStartParams {
    target_backend: String,
    pause_budget_ms: u64,
    journal_max_bytes: Option<u64>,
    fact_batch: Option<usize>,
    replay_batch: Option<usize>,
    edge_cap: Option<usize>,
    observation_window: Option<usize>,
    verification_reserve_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
struct MigrationStatusResult {
    configured: bool,
    job: Option<MigrationJobResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
struct MigrationJobResult {
    epoch_id: String,
    phase: String,
    running: bool,
    target_backend: String,
    target_model: String,
    target_dimension: usize,
    destination: String,
    cancellation_requested: bool,
    recovery_action: Option<String>,
    last_error: Option<String>,
    progress: MigrationProgressResult,
}

#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
struct MigrationProgressResult {
    base_facts: u64,
    base_edge_sets: u64,
    base_batches: u64,
    input_watermark: u64,
    output_watermark: u64,
    distinct_dirty_facts: u64,
    distinct_edge_sources: u64,
    pending_journal_bytes: u64,
    estimated_pause_ms: Option<u64>,
    measured_cutover_ms: Option<u64>,
}

#[tool_router(router = migration_tool_router, vis = "pub(super)")]
impl McpServer {
    #[tool(
        name = "migration_start",
        output_schema = crate::schema::wire_safe_output_schema::<MigrationStatusResult>(),
        description = "Start one daemon-owned online embedding migration in the background. The existing MCP transport remains the only control boundary; no listener or credential is persisted. `target_backend` selects a backend configured in this daemon's environment. `pause_budget_ms` is the maximum measured request pause. Returns immediately with durable job status; poll migration_status. Refuses a pre-existing destination or another non-terminal epoch."
    )]
    async fn migration_start(
        &self,
        Parameters(params): Parameters<MigrationStartParams>,
    ) -> Result<Json<MigrationStatusResult>, ErrorData> {
        let manager = self.require_migration_manager()?;
        let status = tokio::task::spawn_blocking(move || {
            manager.start(&params.target_backend, params.config())?;
            current_status(&manager)
        })
        .await
        .map_err(super::join_error)?
        .map_err(to_error)?;
        Ok(Json(status))
    }

    #[tool(
        name = "migration_status",
        output_schema = crate::schema::wire_safe_output_schema::<MigrationStatusResult>(),
        description = "Read the durable online embedding migration phase, progress watermarks, convergence estimate, measured cutover, cancellation flag, last error and required recovery action. Returns immediately and performs no migration work."
    )]
    async fn migration_status(&self) -> Result<Json<MigrationStatusResult>, ErrorData> {
        let Some(manager) = self.online_migration.clone() else {
            return Ok(Json(MigrationStatusResult {
                configured: false,
                job: None,
            }));
        };
        let status = tokio::task::spawn_blocking(move || current_status(&manager))
            .await
            .map_err(super::join_error)?
            .map_err(to_error)?;
        Ok(Json(status))
    }

    #[tool(
        name = "migration_cancel",
        output_schema = crate::schema::wire_safe_output_schema::<MigrationStatusResult>(),
        description = "Durably request cancellation while the source remains authoritative. A running worker observes the request at its next bounded batch; a paused job is cancelled immediately. From quiescing onward cancellation refuses and reports the required recovery action instead of guessing a rollback."
    )]
    async fn migration_cancel(&self) -> Result<Json<MigrationStatusResult>, ErrorData> {
        let manager = self.require_migration_manager()?;
        let status = tokio::task::spawn_blocking(move || {
            manager.cancel()?;
            current_status(&manager)
        })
        .await
        .map_err(super::join_error)?
        .map_err(to_error)?;
        Ok(Json(status))
    }

    #[tool(
        name = "migration_recover",
        output_schema = crate::schema::wire_safe_output_schema::<MigrationStatusResult>(),
        description = "Resume a durable prepared, capturing, base-copied, catching-up, non-converging or cutover-ready job after verifying the environment-backed target model, dimension and vector witness. Quiescing or activated jobs refuse here until crash-safe cutover recovery has restored a single authoritative generation."
    )]
    async fn migration_recover(&self) -> Result<Json<MigrationStatusResult>, ErrorData> {
        let manager = self.require_migration_manager()?;
        let status = tokio::task::spawn_blocking(move || {
            manager.recover()?;
            current_status(&manager)
        })
        .await
        .map_err(super::join_error)?
        .map_err(to_error)?;
        Ok(Json(status))
    }

    fn require_migration_manager(
        &self,
    ) -> Result<Arc<OnlineMigrationManager<DynEmbedder>>, ErrorData> {
        self.online_migration.clone().ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "online migration is not configured for this server".to_owned(),
                None,
            )
        })
    }
}

impl MigrationStartParams {
    fn config(&self) -> MigrationStartConfig {
        MigrationStartConfig {
            journal_max_bytes: self.journal_max_bytes.unwrap_or(DEFAULT_JOURNAL_BYTES),
            catch_up: CatchUpConfig {
                fact_batch: self.fact_batch.unwrap_or(DEFAULT_FACT_BATCH),
                replay_batch: self.replay_batch.unwrap_or(DEFAULT_REPLAY_BATCH),
                edge_cap: self.edge_cap.unwrap_or(DEFAULT_EDGE_CAP),
            },
            controller: ControllerConfig {
                observation_window: self
                    .observation_window
                    .unwrap_or(DEFAULT_OBSERVATION_WINDOW),
                pause_budget: Duration::from_millis(self.pause_budget_ms),
                verification_reserve: Duration::from_millis(
                    self.verification_reserve_ms
                        .unwrap_or(DEFAULT_VERIFICATION_RESERVE_MS),
                ),
            },
        }
    }
}

fn current_status(
    manager: &OnlineMigrationManager<DynEmbedder>,
) -> Result<MigrationStatusResult, crate::MemoryError> {
    Ok(MigrationStatusResult {
        configured: true,
        job: manager.status()?.map(MigrationJobResult::from),
    })
}

impl From<MigrationStatus> for MigrationJobResult {
    fn from(status: MigrationStatus) -> Self {
        let record = status.record;
        let progress = MigrationProgressResult {
            base_facts: record.progress.base_facts,
            base_edge_sets: record.progress.base_edge_sets,
            base_batches: record.progress.base_batches,
            input_watermark: record.progress.input_watermark,
            output_watermark: record.progress.output_watermark,
            distinct_dirty_facts: record.progress.distinct_dirty_facts,
            distinct_edge_sources: record.progress.distinct_edge_sources,
            pending_journal_bytes: record.progress.pending_journal_bytes,
            estimated_pause_ms: record.progress.estimated_pause.map(duration_millis),
            measured_cutover_ms: record.progress.measured_cutover.map(duration_millis),
        };
        let identity = &record.spec.identity;
        Self {
            epoch_id: identity.epoch_id().to_owned(),
            phase: phase_name(record.phase).to_owned(),
            running: status.running,
            target_backend: record.spec.target_backend.clone(),
            target_model: identity.target_model().to_owned(),
            target_dimension: identity.target_dimension(),
            destination: identity.destination_path().display().to_string(),
            cancellation_requested: record.cancellation_requested,
            recovery_action: record.recovery_action,
            last_error: record.last_error,
            progress,
        }
    }
}

fn phase_name(phase: JobPhase) -> &'static str {
    match phase {
        JobPhase::Prepared => "prepared",
        JobPhase::Capturing => "capturing",
        JobPhase::BaseCopied => "base_copied",
        JobPhase::CatchingUp => "catching_up",
        JobPhase::NonConverging => "non_converging",
        JobPhase::CutoverReady => "cutover_ready",
        JobPhase::Quiescing => "quiescing",
        JobPhase::Activated => "activated",
        JobPhase::Committed => "committed",
        JobPhase::Cancelled => "cancelled",
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
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
}
