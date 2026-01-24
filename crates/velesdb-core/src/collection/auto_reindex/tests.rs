//! Tests for auto-reindex module (TDD - US-004)

use super::*;
use std::time::Duration;

// ============================================================================
// CA-1: Parameter mismatch detection tests
// ============================================================================

#[test]
fn test_detect_params_mismatch_triggers_reindex() {
    let manager = AutoReindexManager::with_defaults();

    // Small dataset created with small params (M=16)
    let small_params = HnswParams::custom(16, 100, 10_000);

    // Dataset grew to 100K - optimal is M=128 for high-dim
    let current_size = 100_000;
    let dimension = 768;

    let check = manager.check_divergence(&small_params, current_size, dimension);

    // Optimal M for 100K@768D is 128, current is 16
    // Ratio = 128/16 = 8.0, threshold is 1.5
    assert!(
        check.should_reindex,
        "Should trigger reindex when params diverge significantly"
    );
    assert!(check.ratio >= 1.5, "Ratio should exceed threshold");
    assert!(check.reason.is_some(), "Should have a reason");

    if let Some(ReindexReason::ParamDivergence {
        current_m,
        optimal_m,
        ratio,
    }) = check.reason
    {
        assert_eq!(current_m, 16);
        assert!(optimal_m > current_m);
        assert!(ratio >= 1.5);
    } else {
        panic!("Expected ParamDivergence reason");
    }
}

#[test]
fn test_no_reindex_when_params_optimal() {
    let manager = AutoReindexManager::with_defaults();

    // Params already optimal for 100K dataset
    let dimension = 768;
    let current_size = 100_000;
    let optimal_params = HnswParams::for_dataset_size(dimension, current_size);

    let check = manager.check_divergence(&optimal_params, current_size, dimension);

    assert!(
        !check.should_reindex,
        "Should not reindex when params are already optimal"
    );
    assert!(
        (check.ratio - 1.0).abs() < 0.01,
        "Ratio should be ~1.0 for optimal params"
    );
}

#[test]
fn test_no_reindex_below_min_size() {
    let manager = AutoReindexManager::with_defaults(); // min_size = 10_000

    // Very suboptimal params but small dataset
    let small_params = HnswParams::custom(8, 50, 1_000);
    let current_size = 5_000; // Below min threshold
    let dimension = 768;

    let check = manager.check_divergence(&small_params, current_size, dimension);

    assert!(
        !check.should_reindex,
        "Should not reindex datasets below min_size_for_reindex"
    );
}

#[test]
fn test_threshold_configurable() {
    // Very high threshold - won't trigger easily
    let config = AutoReindexConfig {
        param_divergence_threshold: 10.0, // Only trigger if optimal is 10x current
        ..Default::default()
    };
    let manager = AutoReindexManager::new(config);

    let small_params = HnswParams::custom(16, 100, 10_000);
    let current_size = 100_000;
    let dimension = 768;

    let check = manager.check_divergence(&small_params, current_size, dimension);

    // Ratio ~8.0 is below threshold of 10.0
    assert!(!check.should_reindex, "Should respect custom threshold");

    // Now with lower threshold
    let sensitive_config = AutoReindexConfig::sensitive(); // threshold = 1.25
    let sensitive_manager = AutoReindexManager::new(sensitive_config);

    let check2 = sensitive_manager.check_divergence(&small_params, current_size, dimension);
    assert!(
        check2.should_reindex,
        "Sensitive config should trigger more easily"
    );
}

#[test]
fn test_disabled_config_never_triggers() {
    let config = AutoReindexConfig::disabled();
    let manager = AutoReindexManager::new(config);

    // Even with huge divergence
    let tiny_params = HnswParams::custom(4, 10, 100);
    let current_size = 1_000_000;
    let dimension = 768;

    let check = manager.check_divergence(&tiny_params, current_size, dimension);

    assert!(
        !check.should_reindex,
        "Disabled config should never trigger reindex"
    );
}

// ============================================================================
// CA-2: Background reindex (state machine) tests
// ============================================================================

#[test]
fn test_reindex_state_machine_idle_to_building() {
    let manager = AutoReindexManager::with_defaults();

    assert_eq!(manager.state(), ReindexState::Idle);

    let started = manager.trigger_manual_reindex();
    assert!(started, "Should be able to start from Idle");
    assert_eq!(manager.state(), ReindexState::Building);
}

#[test]
fn test_reindex_cannot_start_twice() {
    let manager = AutoReindexManager::with_defaults();

    let first = manager.trigger_manual_reindex();
    assert!(first);

    let second = manager.trigger_manual_reindex();
    assert!(!second, "Should not start reindex when already in progress");
}

#[test]
fn test_reindex_progress_updates() {
    let manager = AutoReindexManager::with_defaults();
    let progress_values = Arc::new(std::sync::Mutex::new(Vec::new()));
    let progress_clone = progress_values.clone();

    manager.on_event(move |event| {
        if let ReindexEvent::Progress { percent } = event {
            progress_clone.lock().unwrap().push(percent);
        }
    });

    manager.trigger_manual_reindex();
    manager.report_progress(25);
    manager.report_progress(50);
    manager.report_progress(75);
    manager.report_progress(100);

    let values = progress_values.lock().unwrap();
    assert_eq!(*values, vec![25, 50, 75, 100]);
}

#[test]
fn test_reindex_validation_phase() {
    let manager = AutoReindexManager::with_defaults();

    manager.trigger_manual_reindex();
    assert_eq!(manager.state(), ReindexState::Building);

    let transitioned = manager.start_validation(1000, 950);
    assert!(transitioned);
    assert_eq!(manager.state(), ReindexState::Validating);
}

#[test]
fn test_queries_continue_during_reindex() {
    // This test verifies that the state doesn't block query operations
    // In a real implementation, the index would have Arc<RwLock<>> for concurrent access
    let manager = AutoReindexManager::with_defaults();

    manager.trigger_manual_reindex();
    assert_eq!(manager.state(), ReindexState::Building);

    // State is readable during reindex (non-blocking)
    assert!(manager.is_enabled());
    let config = manager.config();
    assert!(config.enabled);

    // Can check divergence during reindex
    let params = HnswParams::default();
    let check = manager.check_divergence(&params, 10_000, 768);
    assert!(!check.should_reindex); // Already reindexing, so new check returns false
}

// ============================================================================
// CA-3: Rollback tests
// ============================================================================

#[test]
fn test_rollback_on_latency_regression() {
    let manager = AutoReindexManager::with_defaults();

    let old_bench = BenchmarkResult {
        latency_p99_us: 1000,
        recall_estimate: 0.95,
        query_count: 100,
    };

    // New index is 20% slower (regression > 10% threshold)
    let new_bench = BenchmarkResult {
        latency_p99_us: 1200,
        recall_estimate: 0.96,
        query_count: 100,
    };

    let result = manager.validate_benchmark(&old_bench, &new_bench);
    assert!(result.is_err(), "Should reject due to latency regression");
    assert!(result.unwrap_err().contains("Latency regression"));
}

#[test]
fn test_rollback_on_recall_regression() {
    let manager = AutoReindexManager::with_defaults();

    let old_bench = BenchmarkResult {
        latency_p99_us: 1000,
        recall_estimate: 0.95,
        query_count: 100,
    };

    // New index has 5% recall drop (regression > 2% threshold)
    let new_bench = BenchmarkResult {
        latency_p99_us: 900,
        recall_estimate: 0.90,
        query_count: 100,
    };

    let result = manager.validate_benchmark(&old_bench, &new_bench);
    assert!(result.is_err(), "Should reject due to recall regression");
    assert!(result.unwrap_err().contains("Recall regression"));
}

#[test]
fn test_validation_passes_when_improved() {
    let manager = AutoReindexManager::with_defaults();

    let old_bench = BenchmarkResult {
        latency_p99_us: 1000,
        recall_estimate: 0.93,
        query_count: 100,
    };

    // New index is better in both metrics
    let new_bench = BenchmarkResult {
        latency_p99_us: 800,
        recall_estimate: 0.96,
        query_count: 100,
    };

    let result = manager.validate_benchmark(&old_bench, &new_bench);
    assert!(result.is_ok(), "Should pass when new index is better");
}

#[test]
fn test_rollback_preserves_idle_state() {
    let manager = AutoReindexManager::with_defaults();

    manager.trigger_manual_reindex();
    manager.start_validation(1000, 1200);

    let rolled_back = manager.rollback("Latency regression".to_string());
    assert!(rolled_back);
    assert_eq!(
        manager.state(),
        ReindexState::Idle,
        "Should return to Idle after rollback"
    );
}

#[test]
fn test_rollback_event_emitted() {
    let manager = AutoReindexManager::with_defaults();
    let rollback_reason = Arc::new(std::sync::Mutex::new(String::new()));
    let reason_clone = rollback_reason.clone();

    manager.on_event(move |event| {
        if let ReindexEvent::RolledBack { reason } = event {
            *reason_clone.lock().unwrap() = reason;
        }
    });

    manager.trigger_manual_reindex();
    manager.rollback("Test rollback".to_string());

    assert_eq!(*rollback_reason.lock().unwrap(), "Test rollback");
}

// ============================================================================
// CA-4: API and events tests
// ============================================================================

#[test]
fn test_manual_trigger_reindex() {
    let manager = AutoReindexManager::with_defaults();

    let triggered = manager.trigger_manual_reindex();
    assert!(triggered);
    assert_eq!(manager.state(), ReindexState::Building);
}

#[test]
fn test_set_auto_reindex_toggle() {
    let manager = AutoReindexManager::with_defaults();

    assert!(manager.is_enabled());

    manager.set_enabled(false);
    assert!(!manager.is_enabled());

    manager.set_enabled(true);
    assert!(manager.is_enabled());
}

#[test]
fn test_events_emitted_correctly() {
    let manager = AutoReindexManager::with_defaults();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_clone = events.clone();

    manager.on_event(move |event| {
        let event_type = match &event {
            ReindexEvent::Started { .. } => "Started",
            ReindexEvent::Progress { .. } => "Progress",
            ReindexEvent::Validating { .. } => "Validating",
            ReindexEvent::Completed { .. } => "Completed",
            ReindexEvent::RolledBack { .. } => "RolledBack",
        };
        events_clone.lock().unwrap().push(event_type.to_string());
    });

    // Full successful reindex cycle
    manager.trigger_manual_reindex();
    manager.report_progress(50);
    manager.start_validation(1000, 900);
    manager.complete_reindex(Duration::from_secs(10));

    let recorded = events.lock().unwrap();
    assert_eq!(
        *recorded,
        vec!["Started", "Progress", "Validating", "Completed"]
    );
}

#[test]
fn test_start_reindex_with_params() {
    let manager = AutoReindexManager::with_defaults();
    let captured_params = Arc::new(std::sync::Mutex::new(None));
    let params_clone = captured_params.clone();

    manager.on_event(move |event| {
        if let ReindexEvent::Started {
            reason,
            old_params,
            new_params,
        } = event
        {
            *params_clone.lock().unwrap() = Some((reason, old_params, new_params));
        }
    });

    let old = HnswParams::custom(16, 100, 10_000);
    let new = HnswParams::custom(64, 400, 100_000);
    let reason = ReindexReason::ParamDivergence {
        current_m: 16,
        optimal_m: 64,
        ratio: 4.0,
    };

    manager.start_reindex(reason.clone(), old, new);

    let captured = captured_params.lock().unwrap();
    assert!(captured.is_some());
    let (r, o, n) = captured.as_ref().unwrap();
    assert_eq!(o.max_connections, 16);
    assert_eq!(n.max_connections, 64);
    assert!(matches!(r, ReindexReason::ParamDivergence { .. }));
}

// ============================================================================
// Cooldown tests
// ============================================================================

#[test]
fn test_cooldown_prevents_immediate_reindex() {
    let config = AutoReindexConfig {
        cooldown: Duration::from_secs(3600), // 1 hour
        ..Default::default()
    };
    let manager = AutoReindexManager::new(config);

    // Complete a reindex
    manager.trigger_manual_reindex();
    manager.start_validation(1000, 900);
    manager.complete_reindex(Duration::from_secs(5));

    // Now should_reindex should return false due to cooldown
    let small_params = HnswParams::custom(16, 100, 10_000);
    let should = manager.should_reindex(&small_params, 100_000, 768);

    assert!(!should, "Should not trigger reindex during cooldown period");
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_zero_current_m_handled() {
    let manager = AutoReindexManager::with_defaults();

    // Edge case: M=0 (invalid but should not panic)
    let zero_params = HnswParams::custom(0, 100, 10_000);

    let check = manager.check_divergence(&zero_params, 100_000, 768);

    // Should handle gracefully (infinite ratio)
    assert!(check.ratio.is_infinite() || check.ratio > 100.0);
}

#[test]
fn test_reset_clears_state() {
    let manager = AutoReindexManager::with_defaults();

    manager.trigger_manual_reindex();
    assert_eq!(manager.state(), ReindexState::Building);

    manager.reset();
    assert_eq!(manager.state(), ReindexState::Idle);
}

#[test]
fn test_config_presets() {
    let default = AutoReindexConfig::default();
    assert!(default.enabled);
    assert!((default.param_divergence_threshold - 1.5).abs() < 0.01);

    let disabled = AutoReindexConfig::disabled();
    assert!(!disabled.enabled);

    let sensitive = AutoReindexConfig::sensitive();
    assert!((sensitive.param_divergence_threshold - 1.25).abs() < 0.01);
    assert_eq!(sensitive.min_size_for_reindex, 5_000);

    let conservative = AutoReindexConfig::conservative();
    assert!((conservative.param_divergence_threshold - 2.0).abs() < 0.01);
    assert_eq!(conservative.min_size_for_reindex, 50_000);
}
