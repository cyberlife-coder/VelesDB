//! Browser-side query guardrails (#2679).
//!
//! Core's `guardrails` module (EPIC-048) cannot come along: it is
//! `persistence`-gated, and its `QueryContext`/`RateLimiter`/`CircuitBreaker`
//! run on `std::time::Instant`, which aborts on `wasm32-unknown-unknown`.
//! Core's `alloc_guard` byte backstop is also inert here by design — on
//! non-64-bit targets its ceiling is `usize::MAX` (the browser allocator is
//! the effective bound). What remains attackable on this surface is exactly
//! the part the audit flagged: allocations whose size scales with
//! caller-controlled input.
//!
//! So this module bounds the caller-amplified sites with explicit caps in
//! core's own vocabulary: the *numbers* mirror `QueryLimits`' defaults
//! (`DEFAULT_MAX_CARDINALITY`, `DEFAULT_MEMORY_LIMIT_BYTES`), and the
//! refusals carry core's [`velesdb_core::Error::GuardRail`] taxonomy
//! (`VELES-027`) so a
//! browser refusal reads exactly like a core refusal.
//!
//! What is bounded:
//! - WHERE-scan materialized rows, the same unit core checks post-filter
//!   and pre-LIMIT (`QueryContext::check_cardinality`).
//! - Batch / fused-search retained footprint (`num_queries × ids`), priced
//!   in bytes like US-004's memory limit.
//! - `k`-derived fan-out arithmetic, checked before it can wrap.

/// Maximum rows a single WHERE scan may materialize.
///
/// Same number and same refusal semantics as core's
/// `DEFAULT_MAX_CARDINALITY` (EPIC-048 US-003): the check fires on the
/// materialized set *before* `LIMIT` applies, matching the pre-LIMIT point
/// where `QueryContext::check_cardinality` runs.
pub(crate) const MAX_INTERMEDIATE_CARDINALITY: usize = 100_000;

/// Estimated live-result budget for one batch/fused search call, in bytes.
///
/// Same number as core's `DEFAULT_MEMORY_LIMIT_BYTES` (EPIC-048 US-004).
/// 100 MiB is heavy for a browser tab but unreachable by legitimate
/// workloads: it admits e.g. 64 queries × 64K retained rows (~64 MiB).
pub(crate) const QUERY_MEMORY_LIMIT_BYTES: usize = 100 * 1024 * 1024;

/// The retained per-result entry a batch/fused search materializes:
/// `Vec<(u64, f32)>` items — 16 bytes each (8-byte alignment).
const RESULT_ENTRY_BYTES: usize = std::mem::size_of::<(u64, f32)>();

/// Refuses `count` rows of intermediate cardinality above
/// [`MAX_INTERMEDIATE_CARDINALITY`].
///
/// The `String` error carries `Error::GuardRail`'s `Display`, which embeds
/// the `[VELES-027]` code — the `VelesQL` executor's `Result<_, String>`
/// channel pre-dates structured `WasmError`s and degrades every error the
/// same way; the canonical tag survives in the message.
pub(crate) fn check_cardinality(count: usize) -> Result<(), String> {
    if count > MAX_INTERMEDIATE_CARDINALITY {
        return Err(velesdb_core::Error::GuardRail {
            kind: velesdb_core::error::GuardRailKind::Cardinality,
            message: format!(
                "intermediate result cardinality {count} exceeds WASM limit of {MAX_INTERMEDIATE_CARDINALITY} rows"
            ),
        }
        .to_string());
    }
    Ok(())
}

/// Refuses a retained result footprint of `num_queries` top-k passes each
/// holding `per_query_rows` results above [`QUERY_MEMORY_LIMIT_BYTES`].
/// Callers pass the post-truncation row count (`k` — or the fused
/// over-fetch — clamped to the store's `ids.len()`), so the bound prices
/// what `all_results` actually retains, not the transient score pass.
///
/// `Error::GuardRail` so the caller can render the structured `VELES-027`
/// refusal (batch and fused entry points return `Result<_, JsValue>`).
pub(crate) fn check_fanout_memory(
    num_queries: usize,
    per_query_rows: usize,
) -> Result<(), velesdb_core::Error> {
    let bytes = num_queries
        .checked_mul(per_query_rows)
        .and_then(|n| n.checked_mul(RESULT_ENTRY_BYTES))
        .ok_or_else(|| {
            velesdb_core::Error::Overflow("batch fan-out size overflows usize".to_string())
        })?;
    if bytes > QUERY_MEMORY_LIMIT_BYTES {
        return Err(velesdb_core::Error::GuardRail {
            kind: velesdb_core::error::GuardRailKind::Memory,
            message: format!(
                "batch fan-out of {num_queries} queries retaining {per_query_rows} rows each \
                 ({bytes} bytes) exceeds WASM limit of {QUERY_MEMORY_LIMIT_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

/// Doubles `k` for fused-search over-fetch, refusing a wrapped count.
///
/// `k * 2` wraps silently in release and panics in debug; the refusal is
/// `Error::Overflow` — the request is unrepresentable, not merely large.
pub(crate) fn checked_fanout_k(k: usize) -> Result<usize, velesdb_core::Error> {
    k.checked_mul(2).ok_or_else(|| {
        velesdb_core::Error::Overflow(format!(
            "k = {k} overflows usize when doubled for fused over-fetch"
        ))
    })
}

#[cfg(test)]
#[path = "guardrails_tests.rs"]
mod tests;
