//! The config file is looked up beside the **effective** store, not the
//! default one.
//!
//! `VELESDB_MEMORY_PATH` moves the store, and `velesdb-memory.toml` lives
//! beside it. Resolving it against the default directory instead means a
//! caller who moved the store silently reads the config of a store they are
//! not using — and, on a developer machine, picks up a personal
//! `~/.velesdb-memory/velesdb-memory.toml` in the middle of a test run.
//!
//! Spawns the real binary: the lookup happens in `main`, before any library
//! entry point, so nothing below the process boundary can observe it.

use std::io::Read;
use std::net::TcpListener as StdTcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_velesdb-memory")
}

/// A port the OS just confirmed free. Bound and released, so the daemon can
/// take it — the usual small race is acceptable in a test.
fn pick_free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the bound address")
        .port()
}

/// Kills the child on the way out, whatever the test does — no daemon is left
/// running behind a failed assertion.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start the daemon against `store`, let it get past its config load, and
/// return whatever it printed on stderr.
fn startup_stderr(store: &std::path::Path) -> String {
    let mut child = Command::new(binary_path())
        .arg("--http")
        .arg("--http-insecure")
        .arg("--http-port")
        .arg(pick_free_port().to_string())
        .env("VELESDB_MEMORY_PATH", store)
        .env_remove("VELESDB_MEMORY_CONFIG")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn velesdb-memory");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut guard = ChildGuard(child);

    std::thread::sleep(Duration::from_secs(2));
    let _ = guard.0.kill();
    let _ = guard.0.wait();

    let mut out = String::new();
    let _ = stderr.read_to_string(&mut out);
    out
}

#[test]
fn a_config_beside_the_moved_store_is_the_one_that_is_read() {
    let store = tempfile::tempdir().expect("scratch store");
    // A setting that is inert on its own, so the test observes the *lookup*
    // rather than a behaviour change: the banner names the file it loaded.
    std::fs::write(
        store.path().join("velesdb-memory.toml"),
        "[http]\nmax_sessions = 7\n",
    )
    .expect("write the scratch config");

    let stderr = startup_stderr(store.path());

    let expected = store.path().join("velesdb-memory.toml");
    assert!(
        stderr.contains(&expected.display().to_string()),
        "the config beside the moved store must be the one loaded; \
         stderr was: {stderr}"
    );
}

#[test]
fn no_config_beside_the_moved_store_loads_nothing() {
    let store = tempfile::tempdir().expect("scratch store");

    let stderr = startup_stderr(store.path());

    assert!(
        !stderr.contains("setting(s) from"),
        "an empty store must load no config at all — not the default store's; \
         stderr was: {stderr}"
    );
}
