//! The CLI's update check, end to end: the binary must hand the check the
//! config file it was started with and name itself `cli`. Runs the real
//! binary (a REPL whose stdin stays open) against a loopback endpoint, using
//! the helper shared with the server's tests.

#![cfg(feature = "update-check")]

#[path = "../../velesdb-server/tests/update_check_support/mod.rs"]
mod update_check_support;

use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use update_check_support::{isolate, write_config, ChildGuard, Endpoint};

const WAIT: Duration = Duration::from_secs(60);
const GRACE: Duration = Duration::from_secs(3);

/// Starts the REPL with its stdin held open, which keeps the process alive
/// long enough for the background check to complete.
fn start_repl(home: &TempDir, args: &[&str]) -> (ChildGuard, std::process::ChildStdin) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb"));
    isolate(&mut command, home.path());
    command
        .args(args)
        .arg("repl")
        .arg(home.path().join("data"))
        .current_dir(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn velesdb");
    let stdin = child.stdin.take().expect("piped stdin");
    (ChildGuard(child), stdin)
}

#[test]
fn test_cli_reports_itself_with_the_config_file_endpoint() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), None, &endpoint.url());

    let _repl = start_repl(&home, &["--config", &config.to_string_lossy()]);

    let request = endpoint
        .receive(WAIT)
        .expect("the CLI sends its check to the configured endpoint");
    assert!(request.contains("\"entry\":\"cli\""), "{request}");
}

#[test]
fn test_cli_sends_nothing_when_its_config_file_opts_out() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), Some(false), &endpoint.url());

    let _repl = start_repl(&home, &["--config", &config.to_string_lossy()]);

    assert!(
        endpoint.receive(GRACE).is_none(),
        "enabled = false in the --config file must silence the check"
    );
}

#[test]
fn test_cli_reads_velesdb_toml_from_its_working_directory() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), Some(false), &endpoint.url());

    let _repl = start_repl(&home, &[]);

    assert!(
        endpoint.receive(GRACE).is_none(),
        "without --config, ./velesdb.toml is the file the check reads"
    );
}
