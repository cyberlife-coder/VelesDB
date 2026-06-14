//! Shared Tokio runtime for streaming ingestion.
//!
//! [`VelesCollection::enable_streaming`](crate::VelesCollection) creates a core
//! `StreamIngester` whose background drain task is launched with
//! `tokio::spawn`, which requires an ambient Tokio runtime. A mobile host has
//! none, so the bindings host a single shared multi-thread runtime here.
//! Entering it around `enable_streaming` lets the drain task be scheduled; the
//! task then lives on this runtime — which is intentionally never shut down —
//! for the lifetime of the process.

use std::sync::OnceLock;

use tokio::runtime::Runtime;

use crate::types::VelesError;

/// Returns the process-wide streaming runtime, building it on first use.
///
/// Built lazily so loading the library never spawns runtime threads; they
/// appear only once streaming is actually enabled. Initialization uses
/// first-writer-wins via [`OnceLock::set`], so the build races at most with
/// itself.
///
/// # Errors
///
/// Returns [`VelesError::Database`] if the Tokio runtime cannot be created.
pub(crate) fn stream_runtime() -> Result<&'static Runtime, VelesError> {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("velesdb-stream")
        .build()
        .map_err(|e| VelesError::Database {
            message: format!("failed to start streaming runtime: {e}"),
        })?;
    // First writer wins; a late writer's runtime is dropped immediately.
    let _ = RUNTIME.set(rt);
    RUNTIME.get().ok_or_else(|| VelesError::Database {
        message: "streaming runtime unavailable".to_string(),
    })
}
