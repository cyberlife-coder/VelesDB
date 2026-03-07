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

    // Write header comments
    if writeln!(output, "# VelesDB Prometheus Metrics").is_err() || writeln!(output).is_err() {
        return formatting_error_response();
    }

    // Server info
    if writeln!(output, "# HELP velesdb_info VelesDB server information").is_err()
        || writeln!(output, "# TYPE velesdb_info gauge").is_err()
        || writeln!(
            output,
            "velesdb_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        )
        .is_err()
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    // velesdb_up gauge
    if writeln!(output, "# HELP velesdb_up VelesDB server is up and running").is_err()
        || writeln!(output, "# TYPE velesdb_up gauge").is_err()
        || writeln!(output, "velesdb_up 1").is_err()
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    // Plan cache metrics (CACHE-04).
    let cache_metrics = state.db.plan_cache().metrics();
    let cache_stats = state.db.plan_cache().stats();

    if writeln!(
        output,
        "# HELP velesdb_plan_cache_hits_total Plan cache hits"
    )
    .is_err()
        || writeln!(output, "# TYPE velesdb_plan_cache_hits_total counter").is_err()
        || writeln!(
            output,
            "velesdb_plan_cache_hits_total {}",
            cache_metrics.hits()
        )
        .is_err()
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    if writeln!(
        output,
        "# HELP velesdb_plan_cache_misses_total Plan cache misses"
    )
    .is_err()
        || writeln!(output, "# TYPE velesdb_plan_cache_misses_total counter").is_err()
        || writeln!(
            output,
            "velesdb_plan_cache_misses_total {}",
            cache_metrics.misses()
        )
        .is_err()
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    if writeln!(
        output,
        "# HELP velesdb_plan_cache_size Current number of cached plans"
    )
    .is_err()
        || writeln!(output, "# TYPE velesdb_plan_cache_size gauge").is_err()
        || writeln!(
            output,
            "velesdb_plan_cache_size {}",
            cache_stats.l1_size + cache_stats.l2_size
        )
        .is_err()
        || writeln!(output).is_err()
    {
        return formatting_error_response();
    }

    if writeln!(
        output,
        "# HELP velesdb_plan_cache_hit_rate Plan cache hit rate"
    )
    .is_err()
        || writeln!(output, "# TYPE velesdb_plan_cache_hit_rate gauge").is_err()
        || writeln!(
            output,
            "velesdb_plan_cache_hit_rate {:.4}",
            cache_metrics.hit_rate()
        )
        .is_err()
        // M-7: trailing blank line for Prometheus text format conformance.
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
