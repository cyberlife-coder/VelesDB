//! Prometheus metrics handler for VelesDB REST API.
//!
//! Provides a `/metrics` endpoint for Prometheus scraping.
//! Requires the `prometheus` feature flag to be enabled.
//! [EPIC-016/US-034, US-035]
//!
//! Metrics exposed:
//! - `velesdb_info`: Server version info
//! - `velesdb_up`: Server availability gauge

#![allow(dead_code)] // Functions exposed via feature flag, used when prometheus feature enabled

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::fmt::Write;
use std::sync::Arc;

use crate::AppState;

fn formatting_error_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        "Failed to format metrics".to_string(),
    )
        .into_response()
}

/// Prometheus text format metrics response.
///
/// Returns metrics in Prometheus exposition format, including plan cache stats.
#[utoipa::path(
    get,
    path = "/metrics",
    responses(
        (status = 200, description = "Prometheus metrics", content_type = "text/plain"),
        (status = 500, description = "Internal server error")
    ),
    tag = "metrics"
)]
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut output = String::new();

    if write_metrics(&mut output, &state).is_err() {
        return formatting_error_response();
    }

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
        .into_response()
}

/// Writes all Prometheus metrics to the output buffer.
///
/// Aggregates server-level metrics (info, uptime, plan cache) with
/// core operational metrics (query throughput, connections, guardrails,
/// traversal stats, query duration histogram).
fn write_metrics(output: &mut String, state: &AppState) -> std::fmt::Result {
    writeln!(output, "# VelesDB Prometheus Metrics")?;
    writeln!(output)?;

    write_server_info(output)?;
    write_uptime_gauge(output)?;

    let cache_metrics = state.db.plan_cache().metrics();
    let cache_stats = state.db.plan_cache().stats();
    write_cache_metrics(output, cache_metrics, &cache_stats)?;

    // Core operational metrics: query throughput, connections, doc counts.
    output.push_str(&state.operational_metrics.export_prometheus());

    // Guard-rails metrics: rate limiting, resource limits, error counters.
    // Uses the process-wide singleton (OnceLock) — always available.
    output.push_str(&velesdb_core::metrics::global_guardrails_metrics().export_prometheus());

    // Graph traversal metrics: nodes visited, depth, edges scanned.
    output.push_str(&state.traversal_metrics.export_prometheus());

    // Query duration histogram (8-bucket + sum + count).
    output.push_str(&state.query_duration_histogram.export_prometheus(
        "velesdb_query_duration_seconds",
        "Query execution duration in seconds",
    ));

    Ok(())
}

/// Writes server version info metric.
fn write_server_info(output: &mut String) -> std::fmt::Result {
    writeln!(output, "# HELP velesdb_info VelesDB server information")?;
    writeln!(output, "# TYPE velesdb_info gauge")?;
    writeln!(
        output,
        "velesdb_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    )?;
    writeln!(output)
}

/// Writes the server availability gauge.
fn write_uptime_gauge(output: &mut String) -> std::fmt::Result {
    writeln!(output, "# HELP velesdb_up VelesDB server is up and running")?;
    writeln!(output, "# TYPE velesdb_up gauge")?;
    writeln!(output, "velesdb_up 1")?;
    writeln!(output)
}

/// Writes plan cache counters, size, and hit rate (CACHE-04).
fn write_cache_metrics(
    output: &mut String,
    metrics: &velesdb_core::cache::PlanCacheMetrics,
    stats: &velesdb_core::cache::LockFreeCacheStats,
) -> std::fmt::Result {
    write_cache_hits(output, metrics)?;
    write_cache_misses(output, metrics)?;
    write_cache_size(output, stats)?;
    write_cache_hit_rate(output, metrics)
}

fn write_cache_hits(
    output: &mut String,
    metrics: &velesdb_core::cache::PlanCacheMetrics,
) -> std::fmt::Result {
    writeln!(
        output,
        "# HELP velesdb_plan_cache_hits_total Plan cache hits"
    )?;
    writeln!(output, "# TYPE velesdb_plan_cache_hits_total counter")?;
    writeln!(output, "velesdb_plan_cache_hits_total {}", metrics.hits())?;
    writeln!(output)
}

fn write_cache_misses(
    output: &mut String,
    metrics: &velesdb_core::cache::PlanCacheMetrics,
) -> std::fmt::Result {
    writeln!(
        output,
        "# HELP velesdb_plan_cache_misses_total Plan cache misses"
    )?;
    writeln!(output, "# TYPE velesdb_plan_cache_misses_total counter")?;
    writeln!(
        output,
        "velesdb_plan_cache_misses_total {}",
        metrics.misses()
    )?;
    writeln!(output)
}

fn write_cache_size(
    output: &mut String,
    stats: &velesdb_core::cache::LockFreeCacheStats,
) -> std::fmt::Result {
    writeln!(
        output,
        "# HELP velesdb_plan_cache_size Current number of cached plans"
    )?;
    writeln!(output, "# TYPE velesdb_plan_cache_size gauge")?;
    writeln!(
        output,
        "velesdb_plan_cache_size {}",
        stats.l1_size + stats.l2_size
    )?;
    writeln!(output)
}

fn write_cache_hit_rate(
    output: &mut String,
    metrics: &velesdb_core::cache::PlanCacheMetrics,
) -> std::fmt::Result {
    writeln!(
        output,
        "# HELP velesdb_plan_cache_hit_rate Plan cache hit rate"
    )?;
    writeln!(output, "# TYPE velesdb_plan_cache_hit_rate gauge")?;
    writeln!(
        output,
        "velesdb_plan_cache_hit_rate {:.4}",
        metrics.hit_rate()
    )?;
    // M-7: trailing blank line for Prometheus text format conformance.
    writeln!(output)
}

/// Simple health metrics for lightweight monitoring.
pub async fn health_metrics() -> impl IntoResponse {
    let mut output = String::new();

    if writeln!(output, "# HELP velesdb_up VelesDB server is up").is_err()
        || writeln!(output, "# TYPE velesdb_up gauge").is_err()
        || writeln!(output, "velesdb_up 1").is_err()
        // Prometheus text format requires a trailing blank line after each metric family.
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OnboardingMetrics;

    fn test_app_state() -> Arc<AppState> {
        let dir = tempfile::tempdir().unwrap();
        let db = velesdb_core::Database::open(dir.path()).unwrap();
        Arc::new(AppState {
            db,
            onboarding_metrics: OnboardingMetrics::default(),
            query_limits: parking_lot::RwLock::new(velesdb_core::guardrails::QueryLimits::default()),
            ready: std::sync::atomic::AtomicBool::new(true),
            operational_metrics: velesdb_core::metrics::OperationalMetrics::shared(),
            traversal_metrics: std::sync::Arc::new(velesdb_core::metrics::TraversalMetrics::new()),
            query_duration_histogram: std::sync::Arc::new(
                velesdb_core::metrics::DurationHistogram::new(),
            ),
        })
    }

    #[tokio::test]
    async fn test_prometheus_metrics_response_shape() {
        let state = test_app_state();
        let response = prometheus_metrics(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok());
        assert_eq!(
            content_type,
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn test_health_metrics_response_shape() {
        let response = health_metrics().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok());
        assert_eq!(
            content_type,
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn test_plan_cache_metrics_in_prometheus_output() {
        let state = test_app_state();
        let response = prometheus_metrics(State(state)).await.into_response();
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();

        assert!(
            text.contains("velesdb_plan_cache_hits_total"),
            "should contain plan cache hits"
        );
        assert!(
            text.contains("velesdb_plan_cache_misses_total"),
            "should contain plan cache misses"
        );
        assert!(
            text.contains("velesdb_plan_cache_size"),
            "should contain plan cache size"
        );
        assert!(
            text.contains("velesdb_plan_cache_hit_rate"),
            "should contain plan cache hit rate"
        );
    }
}
