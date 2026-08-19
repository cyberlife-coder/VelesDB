use super::*;

#[test]
fn test_guard_fires_on_configured_call_index() {
    let _guard = SaveConfigFaultGuard::activate(2);
    assert!(!should_fail_save_config()); // call 0
    assert!(!should_fail_save_config()); // call 1
    assert!(should_fail_save_config()); // call 2 → fire
    assert!(!should_fail_save_config()); // call 3 → back to normal
}

#[test]
fn test_guard_activate_on_first_call_fails_immediately() {
    let _guard = SaveConfigFaultGuard::activate_on_first_call();
    assert!(should_fail_save_config());
    assert!(!should_fail_save_config());
}

#[test]
fn test_guard_clears_state_on_drop() {
    {
        let _guard = SaveConfigFaultGuard::activate(0);
        assert!(should_fail_save_config());
    }
    // After drop: counter reset, threshold cleared.
    assert_eq!(
        SAVE_CONFIG_FAIL_AT.load(Ordering::SeqCst),
        SAVE_CONFIG_FAIL_DISABLED
    );
    assert!(!should_fail_save_config());
}

#[test]
fn test_guard_clears_flag_even_on_panic() {
    let result = std::panic::catch_unwind(|| {
        let _guard = SaveConfigFaultGuard::activate(0);
        assert!(should_fail_save_config());
        panic!("simulated test failure");
    });
    assert!(result.is_err());
    assert_eq!(
        SAVE_CONFIG_FAIL_AT.load(Ordering::SeqCst),
        SAVE_CONFIG_FAIL_DISABLED
    );
}
