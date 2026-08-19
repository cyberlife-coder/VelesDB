use super::*;
use serial_test::serial;

/// Restores an environment variable to its pre-test value on drop,
/// including while unwinding.
///
/// The previous pattern set the variable, asserted, then removed it on the
/// last line. A failing assertion skipped that line and leaked the variable
/// to **every later test in this process** — `VELESDB_NO_UPDATE_CHECK` would
/// stay set and silently disable update checks for the rest of the run, so
/// one red test could mask a second one. The `remove_var`-first tests had the
/// mirror problem: they clobbered whatever the surrounding environment had
/// configured and never put it back.
///
/// `#[serial(env)]` does not help with either: it orders these tests against
/// each other, not against a leak that outlives them.
struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    /// Sets `key` to `value` for the lifetime of the guard.
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    /// Clears `key` for the lifetime of the guard.
    fn unset(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// The guard restores the variable even when the test body panics — the
/// property the old set/assert/remove ordering lacked.
///
/// Uses a dedicated key so it needs no serialization: no other test reads
/// or writes it.
#[test]
fn test_env_var_guard_restores_on_unwind() {
    const KEY: &str = "VELESDB_TEST_ENV_GUARD_PROBE";
    std::env::set_var(KEY, "original");

    let result = std::panic::catch_unwind(|| {
        let _guard = EnvVarGuard::set(KEY, "overridden");
        assert_eq!(std::env::var(KEY).as_deref(), Ok("overridden"));
        panic!("simulated assertion failure");
    });

    assert!(result.is_err());
    assert_eq!(
        std::env::var(KEY).as_deref(),
        Ok("original"),
        "a panicking test must not leak its environment override to the rest \
         of the process"
    );
    std::env::remove_var(KEY);
}

#[test]
#[serial(env)]
fn test_env_var_disables_update_check() {
    let _no_check = EnvVarGuard::set("VELESDB_NO_UPDATE_CHECK", "1");

    let config = UpdateCheckConfig::default();
    assert!(!config.is_enabled());
}

#[test]
#[serial(env)]
fn test_env_var_overrides_config() {
    let _no_check = EnvVarGuard::set("VELESDB_NO_UPDATE_CHECK", "1");

    let config = UpdateCheckConfig {
        enabled: true, // Config says yes
        ..Default::default()
    };

    assert!(!config.is_enabled()); // But env says no
}

#[test]
#[serial(env)]
fn test_config_disabled() {
    let _no_check = EnvVarGuard::unset("VELESDB_NO_UPDATE_CHECK");
    let _check = EnvVarGuard::unset("VELESDB_UPDATE_CHECK");

    let config = UpdateCheckConfig {
        enabled: false,
        ..Default::default()
    };

    assert!(!config.is_enabled());
}

#[test]
#[serial(env)]
fn test_default_enabled() {
    let _no_check = EnvVarGuard::unset("VELESDB_NO_UPDATE_CHECK");
    let _check = EnvVarGuard::unset("VELESDB_UPDATE_CHECK");

    let config = UpdateCheckConfig::default();
    assert!(config.is_enabled());
}

#[test]
fn test_default_endpoint() {
    let config = UpdateCheckConfig::default();
    assert_eq!(config.endpoint, "https://velesdb.com/api/check");
}

#[test]
fn test_default_timeout() {
    let config = UpdateCheckConfig::default();
    assert_eq!(config.timeout_ms, 2000);
}
