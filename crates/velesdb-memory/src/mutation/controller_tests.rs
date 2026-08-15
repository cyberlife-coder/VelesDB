use std::time::Duration;

use super::controller::{
    ControllerConfig, ControllerPhase, ConvergenceController, ConvergenceSample, ConvergenceVerdict,
};

#[path = "controller_tests/state_tests.rs"]
mod state;

const EPOCH: &str = "00112233445566778899aabbccddeeff";

#[test]
fn measured_net_drain_within_budget_becomes_cutover_ready() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());

    assert_eq!(
        controller
            .observe(sample(0, 100, 80))
            .expect("first")
            .verdict,
        ConvergenceVerdict::CatchingUp
    );
    controller.observe(sample(1, 110, 100)).expect("second");
    let observation = controller.observe(sample(2, 120, 120)).expect("third");

    assert_eq!(observation.metrics.arrival_rate.records, 20);
    assert_eq!(observation.metrics.replay_rate.records, 40);
    assert_eq!(
        observation.metrics.arrival_rate.elapsed,
        Duration::from_secs(2)
    );
    assert_eq!(observation.metrics.window_elapsed, Duration::from_secs(2));
    assert_eq!(observation.metrics.distinct_dirty_facts, 2);
    assert_eq!(observation.metrics.distinct_edge_sources, 1);
    assert_eq!(observation.metrics.pending_journal_bytes, 0);
    assert_eq!(
        observation.metrics.largest_apply_latency,
        Duration::from_millis(20)
    );
    assert_eq!(observation.estimated_pause, Some(Duration::from_millis(70)));
    assert_eq!(observation.verdict, ConvergenceVerdict::CutoverReady);
    assert_eq!(controller.phase(), ControllerPhase::CutoverReady);
}

#[test]
fn growing_backlog_or_arrivals_matching_replay_is_non_converging() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    controller.observe(sample(0, 100, 80)).expect("first");
    controller.observe(sample(1, 110, 85)).expect("second");
    let observation = controller.observe(sample(2, 120, 90)).expect("third");

    assert_eq!(observation.metrics.backlog_records, 30);
    assert!(observation.metrics.backlog_grew);
    assert_eq!(observation.metrics.arrival_rate.records, 20);
    assert_eq!(observation.metrics.replay_rate.records, 10);
    assert_eq!(observation.verdict, ConvergenceVerdict::NonConverging);
    assert_eq!(controller.phase(), ControllerPhase::NonConverging);
}

#[test]
fn drain_estimate_over_operator_budget_keeps_catching_up() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    controller.observe(sample(0, 100, 20)).expect("first");
    controller.observe(sample(1, 110, 31)).expect("second");
    let observation = controller.observe(sample(2, 120, 42)).expect("third");

    assert!(observation.estimated_pause.expect("estimate") > Duration::from_millis(500));
    assert_eq!(observation.verdict, ConvergenceVerdict::CatchingUp);
}

#[test]
fn observations_refuse_non_monotonic_or_inconsistent_watermarks() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    controller.observe(sample(1, 100, 80)).expect("first");

    let time_error = controller
        .observe(sample(1, 101, 81))
        .expect_err("equal time must fail");
    assert!(time_error.to_string().contains("monotonic"), "{time_error}");
    let watermark_error = controller
        .observe(sample(2, 79, 80))
        .expect_err("output ahead of input must fail");
    assert!(
        watermark_error.to_string().contains("watermark"),
        "{watermark_error}"
    );
}

#[test]
fn restart_preserves_audit_sample_but_requires_a_fresh_window() {
    let root = tempfile::tempdir().expect("root");
    {
        let mut controller = open_controller(root.path());
        make_ready(&mut controller);
        assert_eq!(controller.retained_samples(), 3);
    }

    let mut resumed = open_controller(root.path());
    assert_eq!(resumed.phase(), ControllerPhase::CatchingUp);
    assert_eq!(resumed.retained_samples(), 0);
    assert_eq!(
        resumed
            .last_observation()
            .expect("last observation")
            .output_watermark,
        120
    );
    assert_eq!(
        resumed.recovery_action(),
        Some("reopen source and resume catch-up")
    );
    make_ready(&mut resumed);
    resumed
        .begin_quiescing(Duration::from_secs(3))
        .expect("quiesce");
    assert_eq!(
        resumed.phase(),
        ControllerPhase::Quiescing {
            deadline: Duration::from_millis(3_500)
        }
    );
    let error = resumed
        .activate(Duration::from_millis(3_501))
        .expect_err("expired deadline");
    assert!(error.to_string().contains("deadline"), "{error}");
    assert_eq!(resumed.phase(), ControllerPhase::CatchingUp);
    assert_eq!(
        resumed.recovery_action(),
        Some("reopen source and resume catch-up")
    );

    let restored = open_controller(root.path());
    assert_eq!(restored.phase(), ControllerPhase::CatchingUp);
    assert_eq!(
        restored.recovery_action(),
        Some("reopen source and resume catch-up")
    );
}

#[test]
fn restart_during_quiescing_requires_recovery_even_before_deadline() {
    let root = tempfile::tempdir().expect("root");
    drop(quiescing_controller(root.path()));

    let mut resumed = open_controller(root.path());
    assert_eq!(
        resumed.recovery_action(),
        Some("complete or recover cutover before serving traffic")
    );
    let error = resumed
        .activate(Duration::from_millis(3_100))
        .expect_err("restart invalidates deadline ownership");
    assert!(error.to_string().contains("recovery"), "{error}");
    assert!(matches!(resumed.phase(), ControllerPhase::Quiescing { .. }));
}

#[test]
fn equal_nonzero_arrival_and_replay_rates_are_non_converging() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    for (second, watermark) in [(0, 100), (1, 110)] {
        controller
            .observe(sample(second, watermark, watermark))
            .expect("observe");
    }
    let observation = controller.observe(sample(2, 120, 120)).expect("third");
    assert_eq!(observation.verdict, ConvergenceVerdict::NonConverging);
}

#[test]
fn cancellation_permit_requires_source_authority_and_epoch_ownership() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());

    let mismatch = controller
        .cancel(true, "ffeeddccbbaa99887766554433221100")
        .expect_err("wrong epoch");
    assert!(mismatch.to_string().contains("epoch"), "{mismatch}");
    let authority = controller
        .cancel(false, EPOCH)
        .expect_err("not authoritative");
    assert!(
        authority.to_string().contains("authoritative"),
        "{authority}"
    );

    let permit = controller.cancel(true, EPOCH).expect("cancel");
    assert_eq!(permit.epoch_id(), EPOCH);
    assert_eq!(controller.phase(), ControllerPhase::Cancelled);
    assert_eq!(
        open_controller(root.path()).phase(),
        ControllerPhase::Cancelled
    );
}

#[test]
fn cancellation_phase_matrix_refuses_only_after_cutover_started() {
    assert_cancellation(make_ready, true);
    assert_cancellation(make_nonconverging, true);
    assert_cancellation(make_activated, false);
}

fn assert_cancellation(setup: fn(&mut ConvergenceController), allowed: bool) {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    setup(&mut controller);
    let result = controller.cancel(true, EPOCH);
    assert_eq!(result.is_ok(), allowed);
    if !allowed {
        assert_eq!(
            controller.recovery_action(),
            Some("complete or recover cutover before serving traffic")
        );
    }
}

#[test]
fn cancellation_after_quiescing_records_recovery_instead_of_rollback() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = quiescing_controller(root.path());

    let error = controller
        .cancel(false, EPOCH)
        .expect_err("cannot roll back");
    assert!(error.to_string().contains("recovery"), "{error}");
    assert_eq!(
        controller.recovery_action(),
        Some("complete or recover cutover before serving traffic")
    );
    assert_eq!(
        open_controller(root.path()).recovery_action(),
        Some("complete or recover cutover before serving traffic")
    );
}

fn open_controller(workspace: &std::path::Path) -> ConvergenceController {
    ConvergenceController::open(
        workspace,
        EPOCH,
        ControllerConfig {
            observation_window: 3,
            pause_budget: Duration::from_millis(500),
            verification_reserve: Duration::from_millis(50),
        },
    )
    .expect("controller")
}

fn make_ready(controller: &mut ConvergenceController) {
    for (second, input, output) in [(0, 100, 80), (1, 110, 100), (2, 120, 120)] {
        controller
            .observe(sample(second, input, output))
            .expect("observe");
    }
}

fn make_nonconverging(controller: &mut ConvergenceController) {
    for (second, watermark) in [(0, 100), (1, 110), (2, 120)] {
        controller
            .observe(sample(second, watermark, watermark))
            .expect("observe");
    }
}

fn make_activated(controller: &mut ConvergenceController) {
    make_ready(controller);
    controller
        .begin_quiescing(Duration::from_secs(3))
        .expect("quiesce");
    controller
        .activate(Duration::from_millis(3_100))
        .expect("activate");
}

#[test]
fn activation_persists_the_measured_cutover_window() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = quiescing_controller(root.path());
    controller
        .activate(Duration::from_millis(3_125))
        .expect("activate");
    assert_eq!(
        controller.measured_cutover(),
        Some(Duration::from_millis(125))
    );
    assert_eq!(
        open_controller(root.path()).measured_cutover(),
        Some(Duration::from_millis(125))
    );
}

fn quiescing_controller(workspace: &std::path::Path) -> ConvergenceController {
    let mut controller = open_controller(workspace);
    make_ready(&mut controller);
    controller
        .begin_quiescing(Duration::from_secs(3))
        .expect("quiesce");
    controller
}

fn sample(second: u64, input: u64, output: u64) -> ConvergenceSample {
    let backlog = input.saturating_sub(output);
    ConvergenceSample {
        observed_at: Duration::from_secs(second),
        input_watermark: input,
        output_watermark: output,
        distinct_dirty_facts: 2,
        distinct_edge_sources: 1,
        pending_journal_bytes: backlog * 49,
        replay_elapsed: Duration::from_millis(25),
        largest_apply_latency: Duration::from_millis(20),
    }
}
