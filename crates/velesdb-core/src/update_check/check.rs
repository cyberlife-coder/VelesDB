//! Update Check Logic (US-003)
//!
//! Sends the start-up update check and, for long-running processes, a
//! daily heartbeat. Everything is best effort: a failure is logged at trace
//! level and never reaches the caller.

use super::{ci::is_ci, config::UpdateCheckConfig, instance_id::install_hash};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Delay between two heartbeats of a long-running process.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Telemetry payload. What it contains is listed in
/// `docs/guides/CONFIGURATION.md`; adding a field is a privacy decision.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckPayload {
    /// VelesDB version (e.g., "0.5.4")
    pub version: String,

    /// OS identifier (e.g., "linux", "windows", "macos")
    pub os: String,

    /// Architecture (e.g., "x86_64", "aarch64")
    pub arch: String,

    /// SHA256 hex of a random per-install id (stable per install)
    pub instance_hash: String,

    /// Edition: "core" or "premium"
    pub edition: String,

    /// Binary that sent the check, as the caller declared it ("server", "cli")
    pub entry: String,

    /// True when a CI system's environment variables are present
    pub ci: bool,
}

impl UpdateCheckPayload {
    /// Create a new payload with current system info.
    #[must_use]
    pub fn new(instance_hash: String, edition: &str, entry: &str, ci: bool) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            instance_hash,
            edition: edition.to_string(),
            entry: entry.to_string(),
            ci,
        }
    }
}

/// Who is asking: which binary, which edition, and where it keeps its data.
#[derive(Debug, Clone)]
pub struct UpdateCheckContext {
    entry: String,
    edition: String,
    data_dir: Option<PathBuf>,
}

impl UpdateCheckContext {
    /// `entry` names the calling binary (`"server"`, `"cli"`); `edition` is
    /// `"core"` or `"premium"`.
    #[must_use]
    pub fn new(entry: impl Into<String>, edition: impl Into<String>) -> Self {
        Self {
            entry: entry.into(),
            edition: edition.into(),
            data_dir: None,
        }
    }

    /// Data directory, used only as a fallback place for the install id when
    /// the per-user configuration directory is not writable.
    #[must_use]
    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    /// The binary the check is reported for.
    #[must_use]
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// The edition the check is reported for.
    #[must_use]
    pub fn edition(&self) -> &str {
        &self.edition
    }

    /// The fallback directory for the install id, when one was given.
    #[must_use]
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    fn payload(&self) -> UpdateCheckPayload {
        UpdateCheckPayload::new(
            install_hash(self.data_dir.as_deref()),
            &self.edition,
            &self.entry,
            is_ci(|key| std::env::var(key).ok()),
        )
    }
}

/// What happens after the first check.
#[derive(Debug, Clone, Copy)]
enum Cadence {
    /// Check once, then stop.
    Once,
    /// Check again after every interval, for as long as the process lives.
    Every(Duration),
}

/// Response from update check server
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCheckResponse {
    /// Latest available version
    pub latest_version: String,

    /// True if update is available
    pub update_available: bool,

    /// Optional message (e.g., "Security fix available")
    pub message: Option<String>,
}

/// Sends the check through `ping`, then again after every interval when
/// `cadence` asks for it.
///
/// This is where the opt-out is decided: when `config` reads as disabled
/// through `env`, nothing runs and `build` is not called, so an opted-out
/// install writes no id and sends nothing. `build` produces the payload once
/// for the whole run; `ping` sends one copy of it.
async fn drive<B, P, Fut>(
    config: &UpdateCheckConfig,
    env: impl Fn(&str) -> Option<String>,
    cadence: Cadence,
    build: B,
    mut ping: P,
) where
    B: FnOnce() -> UpdateCheckPayload,
    P: FnMut(UpdateCheckPayload) -> Fut,
    Fut: Future<Output = ()>,
{
    if !config.is_enabled_with(env) {
        tracing::debug!("Update check disabled by user preference");
        return;
    }
    tracing::info!("Checking for updates... (disable with VELESDB_NO_UPDATE_CHECK=1)");
    let payload = build();
    ping(payload.clone()).await;
    let Cadence::Every(interval) = cadence else {
        return;
    };
    loop {
        tokio::time::sleep(interval).await;
        ping(payload.clone()).await;
    }
}

/// Sends one payload, bounded by the configured timeout. Failures are
/// logged at trace level and otherwise ignored.
async fn ping_server(config: &UpdateCheckConfig, payload: UpdateCheckPayload) {
    tracing::debug!(?payload, "Update check payload");
    let timeout = Duration::from_millis(config.timeout_ms);

    match tokio::time::timeout(timeout, send_update_check(&config.endpoint, &payload)).await {
        Ok(Ok(response)) => handle_response(&payload.version, &response),
        Ok(Err(_)) => {
            tracing::trace!("Update check skipped (network unavailable)");
        }
        Err(_) => {
            tracing::trace!("Update check skipped (timeout)");
        }
    }
}

/// [`drive`] against the real environment and the real server.
async fn run(config: &UpdateCheckConfig, context: &UpdateCheckContext, cadence: Cadence) {
    drive(
        config,
        |key| std::env::var(key).ok(),
        cadence,
        || context.payload(),
        |payload| ping_server(config, payload),
    )
    .await;
}

/// Perform the startup update check (async, non-blocking).
///
/// This should be called early in startup, spawned as a background task.
/// It will not block startup - timeout is 2 seconds max.
pub async fn check_for_updates_with(config: &UpdateCheckConfig, context: &UpdateCheckContext) {
    run(config, context, Cadence::Once).await;
}

async fn send_update_check(
    endpoint: &str,
    payload: &UpdateCheckPayload,
) -> Result<UpdateCheckResponse, reqwest::Error> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5)) // Hard limit
        .build()?;

    let response = client
        .post(endpoint)
        .json(payload)
        .send()
        .await?
        .json::<UpdateCheckResponse>()
        .await?;

    Ok(response)
}

fn handle_response(current_version: &str, response: &UpdateCheckResponse) {
    if response.update_available {
        let message = response.message.as_deref().unwrap_or("");
        tracing::info!(
            "Update available: {} -> {} {}",
            current_version,
            response.latest_version,
            message
        );
    } else {
        tracing::debug!("VelesDB is up to date ({})", current_version);
    }
}

/// Runs the check on a detached background thread with its own runtime, so
/// it neither blocks the caller nor keeps the process alive at exit.
fn spawn(config: UpdateCheckConfig, context: UpdateCheckContext, cadence: Cadence) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        match rt {
            Ok(runtime) => runtime.block_on(run(&config, &context, cadence)),
            Err(e) => {
                tracing::trace!("Update check skipped (runtime error: {})", e);
            }
        }
    });
}

/// Sends the update check once from a background thread.
///
/// For short-lived processes such as the CLI.
pub fn spawn_update_check_with(config: UpdateCheckConfig, context: UpdateCheckContext) {
    spawn(config, context, Cadence::Once);
}

/// Sends the update check from a background thread, then once every 24 hours
/// for as long as the process runs.
///
/// For long-running processes such as the server. The thread is detached: it
/// ends with the process and never delays shutdown.
pub fn spawn_update_heartbeat(config: UpdateCheckConfig, context: UpdateCheckContext) {
    spawn(config, context, Cadence::Every(HEARTBEAT_INTERVAL));
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;
