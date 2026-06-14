//! Shared Tokio runtime for streaming ingestion.
//!
//! [`Collection.enable_streaming`](crate::Collection) creates a core
//! `StreamIngester` whose background drain task is launched with
//! `tokio::spawn`, which requires an ambient Tokio runtime. A Python process
//! has none, so the bindings host a single shared multi-thread runtime here.
//! Entering it around `enable_streaming` lets the drain task be scheduled; the
//! task then lives on this runtime — which is intentionally never shut down —
//! for the lifetime of the process.

use std::sync::OnceLock;

use pyo3::exceptions::PyRuntimeError;
use pyo3::PyResult;
use tokio::runtime::Runtime;

/// Returns the process-wide streaming runtime, building it on first use.
///
/// Built lazily so importing the extension never spawns runtime threads; they
/// appear only once streaming is actually enabled. Initialization is serialized
/// by the caller's GIL, so the build races at most with itself.
///
/// # Errors
///
/// Returns a `RuntimeError` if the Tokio runtime cannot be created.
pub(crate) fn stream_runtime() -> PyResult<&'static Runtime> {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("velesdb-stream")
        .build()
        .map_err(|e| PyRuntimeError::new_err(format!("failed to start streaming runtime: {e}")))?;
    // First writer wins; a late writer's runtime is dropped immediately. The
    // GIL serializes callers, so in practice the first call always wins.
    let _ = RUNTIME.set(rt);
    RUNTIME
        .get()
        .ok_or_else(|| PyRuntimeError::new_err("streaming runtime unavailable"))
}
