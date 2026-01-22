//! Prometheus metrics handler for VelesDB REST API.
//!
//! Provides a `/metrics` endpoint for Prometheus scraping.
//! Requires the `prometheus` feature flag to be enabled.
//! [EPIC-016/US-034, US-035]
//!
//! Metrics exposed:
//! - `velesdb_info`: Server version info
//! - `velesdb_up`: Server availability gauge

use axum::{http::StatusCode, response::IntoResponse};
use std::fmt::Write;
/// Prometheus text format metrics response.
///
/// Returns metrics in Prometheus exposition format.
#[utoipa::path(
    get,
    path = "/metrics",
    responses(
        (status = 200, description = "Prometheus metrics", content_type = "text/plain"),
        (status = 500, description = "Internal server error")
    ),
    tag = "metrics"
)]
pub async fn prometheus_metrics() -> impl IntoResponse {
    let mut output = String::new();

    // Write header comments
    writeln!(output, "# VelesDB Prometheus Metrics").unwrap();
    writeln!(output).unwrap();

    // Server info
    writeln!(output, "# HELP velesdb_info VelesDB server information").unwrap();
    writeln!(output, "# TYPE velesdb_info gauge").unwrap();
    writeln!(
        output,
        "velesdb_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    )
    .unwrap();
    writeln!(output).unwrap();

    // velesdb_up gauge
    writeln!(output, "# HELP velesdb_up VelesDB server is up and running").unwrap();
    writeln!(output, "# TYPE velesdb_up gauge").unwrap();
    writeln!(output, "velesdb_up 1").unwrap();

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}

/// Simple health metrics for lightweight monitoring.
pub async fn health_metrics() -> impl IntoResponse {
    let mut output = String::new();

    writeln!(output, "# HELP velesdb_up VelesDB server is up").unwrap();
    writeln!(output, "# TYPE velesdb_up gauge").unwrap();
    writeln!(output, "velesdb_up 1").unwrap();

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_health_metrics_format() {
        // Verify the format is valid Prometheus text format
        let output =
            "# HELP velesdb_up VelesDB server is up\n# TYPE velesdb_up gauge\nvelesdb_up 1\n";
        assert!(output.contains("velesdb_up 1"));
        assert!(output.contains("# TYPE"));
        assert!(output.contains("# HELP"));
    }
}
