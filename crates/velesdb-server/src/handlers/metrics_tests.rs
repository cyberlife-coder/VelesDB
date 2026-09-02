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
        operational_metrics: velesdb_core::metrics::OperationalMetrics::new_arc(),
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

#[tokio::test]
async fn test_match_metrics_in_prometheus_output() {
    let state = test_app_state();
    let response = prometheus_metrics(State(state)).await.into_response();
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        text.contains("velesdb_match_queries_total"),
        "should contain MATCH query throughput"
    );
    assert!(
        text.contains("velesdb_match_latency_seconds"),
        "should contain the MATCH latency histogram"
    );
    assert!(
        text.contains("velesdb_match_guardrail_hits_total"),
        "should contain MATCH guard-rail hits"
    );
}
