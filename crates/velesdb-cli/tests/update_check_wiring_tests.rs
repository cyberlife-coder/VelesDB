//! The CLI's update check, end to end: the binary must hand the check the
//! config file it was started with and name itself `cli`. Runs the real
//! binary (a REPL whose stdin stays open) against loopback endpoints, using
//! the helper shared with the server's tests.
//!
//! The `--config` file lives in `conf/` while the working directory holds a
//! different `velesdb.toml`, so a binary that read the wrong one would send
//! to, or stay silent for, the wrong endpoint.

#![cfg(feature = "update-check")]

#[path = "../../velesdb-server/tests/update_check_support/mod.rs"]
mod update_check_support;

use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use update_check_support::{isolate, write_config, ChildGuard, Endpoint};

const WAIT: Duration = Duration::from_secs(60);
const GRACE: Duration = Duration::from_secs(3);

/// Starts the REPL in `home` (its working directory) with its stdin held
/// open, which keeps the process alive long enough for the background check
/// to complete.
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

/// `--config conf/custom.toml` (enabled, endpoint `named`) next to a
/// `./velesdb.toml` that points at `other` and opts out.
#[test]
fn test_cli_uses_the_config_file_it_was_given_not_the_working_directory_one() {
    let home = TempDir::new().expect("tempdir");
    let (named, other) = (Endpoint::bind(), Endpoint::bind());
    let config = write_config(home.path(), "conf/custom.toml", None, &named);
    write_config(home.path(), "velesdb.toml", Some(false), &other);

    let _repl = start_repl(&home, &["--config", &config.to_string_lossy()]);

    let request = named
        .receive(WAIT)
        .expect("the CLI reports to the endpoint named in --config");
    assert!(request.contains("\"entry\":\"cli\""), "{request}");
    assert!(other.receive(Duration::from_millis(500)).is_none());
}

/// `--config` opts out while `./velesdb.toml` would send: silence proves the
/// first one won.
#[test]
fn test_cli_opt_out_in_the_given_config_file_beats_the_working_directory_file() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), "conf/off.toml", Some(false), &endpoint);
    write_config(home.path(), "velesdb.toml", Some(true), &endpoint);

    let _repl = start_repl(&home, &["--config", &config.to_string_lossy()]);

    assert!(endpoint.receive(GRACE).is_none());
}

/// Without `--config` the check reads `./velesdb.toml`, and uses it.
#[test]
fn test_cli_without_config_reads_velesdb_toml_from_its_working_directory() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), "velesdb.toml", None, &endpoint);

    let _repl = start_repl(&home, &[]);

    let request = endpoint
        .receive(WAIT)
        .expect("the CLI reports to the endpoint named in ./velesdb.toml");
    assert!(request.contains("\"entry\":\"cli\""), "{request}");
}

#[test]
fn test_cli_without_config_honours_an_opt_out_in_velesdb_toml() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), "velesdb.toml", Some(false), &endpoint);

    let _repl = start_repl(&home, &[]);

    assert!(endpoint.receive(GRACE).is_none());
}
