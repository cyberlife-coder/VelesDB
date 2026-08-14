use std::fs;
use std::time::Duration;

use super::{open_controller, sample, ControllerConfig, ConvergenceController, EPOCH};

const STATE_FILE: &str = "online-migration-controller.json";
const STAGING_FILE: &str = "online-migration-controller.json.tmp";

#[test]
fn controller_configuration_bounds_memory_and_pause_inputs() {
    let root = tempfile::tempdir().expect("root");
    for observation_window in [0, 1, 65] {
        assert!(ConvergenceController::open(
            root.path(),
            EPOCH,
            ControllerConfig {
                observation_window,
                pause_budget: Duration::from_secs(1),
                verification_reserve: Duration::ZERO,
            },
        )
        .is_err());
    }
    assert!(ConvergenceController::open(
        root.path(),
        EPOCH,
        ControllerConfig {
            observation_window: 2,
            pause_budget: Duration::ZERO,
            verification_reserve: Duration::ZERO,
        },
    )
    .is_err());
}

#[test]
fn corrupted_or_epoch_mismatched_state_fails_closed() {
    let root = tempfile::tempdir().expect("root");
    drop(open_controller(root.path()));
    let path = root.path().join(STATE_FILE);
    let original = fs::read_to_string(&path).expect("state");

    fs::write(&path, "{truncated").expect("corrupt");
    let corrupt = ConvergenceController::open(root.path(), EPOCH, config())
        .err()
        .expect("corrupt");
    assert!(corrupt.to_string().contains("invalid controller state"));

    fs::write(&path, original).expect("restore");
    let mismatch =
        ConvergenceController::open(root.path(), "ffeeddccbbaa99887766554433221100", config())
            .err()
            .expect("epoch mismatch");
    assert!(mismatch.to_string().contains("epoch ownership"));
}

#[test]
fn activated_state_without_a_cutover_ready_window_fails_closed() {
    let root = tempfile::tempdir().expect("root");
    drop(open_controller(root.path()));
    let path = root.path().join(STATE_FILE);
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("state")).expect("json");
    state["phase"] = serde_json::Value::String("Activated".to_owned());
    fs::write(&path, serde_json::to_vec_pretty(&state).expect("encode")).expect("tamper");

    let error = ConvergenceController::open(root.path(), EPOCH, config())
        .err()
        .expect("invalid activated state");
    assert!(error.to_string().contains("activated state"), "{error}");
}

#[cfg(unix)]
#[test]
fn symlinked_state_is_refused_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    symlink(&victim, root.path().join(STATE_FILE)).expect("symlink");

    let error = ConvergenceController::open(root.path(), EPOCH, config())
        .err()
        .expect("symlink");
    assert!(error.to_string().contains("regular file"), "{error}");
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

#[cfg(unix)]
#[test]
fn failed_persistence_does_not_advance_in_memory_state_or_touch_target() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    let victim = root.path().join("victim");
    fs::write(&victim, b"untouched").expect("victim");
    symlink(&victim, root.path().join(STAGING_FILE)).expect("symlink");

    controller
        .observe(sample(0, 10, 10))
        .expect_err("save fails");
    assert_eq!(controller.retained_samples(), 0);
    assert_eq!(fs::read(&victim).expect("victim"), b"untouched");
}

#[test]
fn durable_state_contains_only_bounded_controller_facts() {
    let root = tempfile::tempdir().expect("root");
    let mut controller = open_controller(root.path());
    for second in 0..100 {
        controller
            .observe(sample(second, second, second))
            .expect("observe");
    }
    let bytes = fs::read(root.path().join(STATE_FILE)).expect("state");
    assert!(bytes.len() < 64 * 1024);
    assert_eq!(controller.retained_samples(), 3);
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(!text.contains("source_path"));
    assert!(!text.contains("destination_path"));
    assert!(!text.contains("credential"));
}

fn config() -> ControllerConfig {
    ControllerConfig {
        observation_window: 3,
        pause_budget: Duration::from_millis(500),
        verification_reserve: Duration::from_millis(50),
    }
}
