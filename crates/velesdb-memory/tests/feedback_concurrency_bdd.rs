//! Feature: learned confidence is exact under concurrent feedback.
//!
//! Several agents sharing one daemon report outcomes on the same memory at
//! the same moment. Each outcome must move the confidence exactly once: a
//! lost increment would make the ranking signal depend on scheduling rather
//! than on what actually happened.

#![cfg(feature = "persistence")]

mod common;

use std::sync::Barrier;

use common::service;

/// Agents reporting at once. Eight successes of `+0.1` from `0.0` land on
/// `0.8`; one lost update lands on `0.7` or lower — far outside the
/// tolerance below.
const AGENTS: usize = 8;
/// Independent facts the scenario is replayed on, so a race that happens to
/// serialize itself once cannot hide a lost update.
const ROUNDS: usize = 12;
/// The confidence `AGENTS` sequential `+0.1` successes reach from `0.0`.
const EXPECTED: f32 = 0.8;

/// Given a memory driven to the floor by sequential negative outcomes,
/// when `AGENTS` agents each report one success at the same instant,
/// then its confidence equals the result of `AGENTS` sequential successes.
#[test]
fn simultaneous_successes_each_move_confidence_once() {
    let (_dir, svc) = service();
    for round in 0..ROUNDS {
        let id = svc
            .remember(&format!("shared lesson number {round}"), &[], None)
            .expect("remember");
        let mut floor = 1.0;
        for _ in 0..20 {
            floor = svc.feedback(id, false).expect("negative feedback");
        }
        assert!(
            floor.abs() < 1e-6,
            "twenty failures reach the floor, got {floor}"
        );

        let start = Barrier::new(AGENTS);
        std::thread::scope(|scope| {
            for _ in 0..AGENTS {
                scope.spawn(|| {
                    start.wait();
                    svc.feedback(id, true).expect("positive feedback");
                });
            }
        });

        let after = svc.feedback(id, false).expect("probe feedback") + 0.05;
        assert!(
            (after - EXPECTED).abs() < 1e-3,
            "round {round}: {AGENTS} simultaneous successes must yield {EXPECTED}, got {after}"
        );
    }
}

/// Times the racing agent remembers the same fact again in one round.
const RESTORES: usize = 40;

/// Given a memory driven to the floor, when `AGENTS` agents each report one
/// success while another agent remembers the very same fact again and again
/// — a re-store that reads the fact's learned confidence and writes it back
/// with the new content — then no success is lost: the re-store and the
/// feedback are serialized per fact, so the re-store never writes back a
/// confidence a success has already moved.
#[test]
fn a_re_store_racing_feedback_loses_no_outcome() {
    let (_dir, svc) = service();
    for round in 0..ROUNDS {
        let fact = format!("re-stored lesson number {round}");
        let id = svc.remember(&fact, &[], None).expect("remember");
        for _ in 0..20 {
            svc.feedback(id, false).expect("negative feedback");
        }

        let start = Barrier::new(AGENTS + 1);
        std::thread::scope(|scope| {
            for _ in 0..AGENTS {
                scope.spawn(|| {
                    start.wait();
                    svc.feedback(id, true).expect("positive feedback");
                });
            }
            scope.spawn(|| {
                start.wait();
                for _ in 0..RESTORES {
                    let again = svc.remember(&fact, &[], None).expect("re-remember");
                    assert_eq!(again, id, "the same fact keeps its id");
                }
            });
        });

        let after = svc.feedback(id, false).expect("probe feedback") + 0.05;
        assert!(
            (after - EXPECTED).abs() < 1e-3,
            "round {round}: a re-store racing {AGENTS} successes must keep all of them \
             ({EXPECTED}), got {after}"
        );
    }
}
