//! CLI flag handling for the `velesdb-memory` binary (onboarding audit P2-3).
//!
//! Regression this catches: the binary previously had no CLI flags at all —
//! `velesdb-memory --version` (a first thing any new dev tries) silently
//! ignored the argument and tried to open the MCP stdio store instead,
//! which either hangs waiting on stdin or fails opening the default store
//! path. `--version` / `-V` must short-circuit BEFORE the store is opened:
//! print `velesdb-memory <CARGO_PKG_VERSION>` to stdout and exit 0, with no
//! store side effects.
//!
//! Spawns the real binary (`env!("CARGO_BIN_EXE_velesdb-memory")`) exactly
//! like `tests/mcp_lifecycle.rs` does, rather than calling an internal
//! function, so the test proves the actual argv-parsing entry point works.

use std::process::Command;
use std::time::Duration;

/// Upper bound the test is willing to wait for `--version` to print and
/// exit. This path never opens the store or touches stdio transport, so it
/// should return near-instantly; 5s is generous headroom over any plausible
/// process-startup cost.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

fn run_with_arg(arg: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .arg(arg)
        // A store path that does not exist and is never created: proves
        // --version exits before any store-opening attempt would fail on
        // this bogus path.
        .env(
            "VELESDB_MEMORY_PATH",
            "/nonexistent/path/that/must/never/be/opened",
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn velesdb-memory binary");

    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll child status") {
            let output = child
                .wait_with_output()
                .expect("failed to collect child output after exit");
            return std::process::Output { status, ..output };
        }
        if start.elapsed() > VERSION_TIMEOUT {
            let _ = child.kill();
            panic!("velesdb-memory {arg} did not exit within {VERSION_TIMEOUT:?} (still trying to open the store?)");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn test_version_flag_long_prints_version_and_exits_zero() {
    let output = run_with_arg("--version");

    assert!(
        output.status.success(),
        "expected exit code 0, got {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout was not valid UTF-8");
    let expected = format!("velesdb-memory {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        stdout, expected,
        "expected stdout to be exactly {expected:?}, got {stdout:?}"
    );
}

#[test]
fn test_version_flag_short_prints_version_and_exits_zero() {
    let output = run_with_arg("-V");

    assert!(
        output.status.success(),
        "expected exit code 0, got {:?}; stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout was not valid UTF-8");
    let expected = format!("velesdb-memory {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        stdout, expected,
        "expected stdout to be exactly {expected:?}, got {stdout:?}"
    );
}
