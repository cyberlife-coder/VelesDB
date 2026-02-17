//! Update Check Logic (US-003)
//!
//! Performs non-blocking update check at startup.

use super::{config::UpdateCheckConfig, instance_id::compute_instance_hash};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

/// Telemetry payload - Minimal, no PII
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckPayload {
    /// `VelesDB` version (e.g., "0.5.4")
    pub version: String,

    /// OS identifier (e.g., "linux", "windows", "macos")
    pub os: String,

    /// Architecture (e.g., "`x86_64`", "aarch64")
    pub arch: String,

    /// SHA256 hash of instance ID (stable per install)
    pub instance_hash: String,

    /// Edition: "core" or "premium"
    pub edition: String,
}

impl UpdateCheckPayload {
    /// Create a new payload with current system info.
    #[must_use]
    pub fn new(instance_hash: String, edition: &str) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            instance_hash,
            edition: edition.to_string(),
        }
    }
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

/// Perform the startup update check (async, non-blocking).
///
/// This should be called early in startup, spawned as a background task.
/// It will not block startup - timeout is 2 seconds max.
pub async fn check_for_updates(config: &UpdateCheckConfig, data_dir: &Path, edition: &str) {
    if !config.is_enabled() {
        tracing::debug!("Update check disabled by user preference");
        return;
    }

    tracing::info!("Checking for updates... (disable with VELESDB_NO_UPDATE_CHECK=1)");

    let instance_hash = compute_instance_hash(data_dir);
    let payload = UpdateCheckPayload::new(instance_hash, edition);

    // Log payload at debug level for transparency
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

/// Synchronous wrapper for non-async contexts.
///
/// Spawns the update check in a background thread so it doesn't block.
pub fn spawn_update_check(
    config: UpdateCheckConfig,
    data_dir: std::path::PathBuf,
    edition: String,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        match rt {
            Ok(runtime) => {
                runtime.block_on(async {
                    check_for_updates(&config, &data_dir, &edition).await;
                });
            }
            Err(e) => {
                tracing::trace!("Update check skipped (runtime error: {})", e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_creation() {
        let payload = UpdateCheckPayload::new("abc123".to_string(), "core");

        assert!(!payload.version.is_empty());
        assert!(!payload.os.is_empty());
        assert!(!payload.arch.is_empty());
        assert_eq!(payload.instance_hash, "abc123");
        assert_eq!(payload.edition, "core");
    }

    #[test]
    fn test_payload_serialization() {
        let payload = UpdateCheckPayload::new("abc123".to_string(), "core");
        let json = serde_json::to_string(&payload).expect("Failed to serialize");

        assert!(json.contains("\"os\""));
        assert!(json.contains("\"arch\""));
        assert!(json.contains("\"instance_hash\":\"abc123\""));
        assert!(json.contains("\"edition\":\"core\""));
    }
}
