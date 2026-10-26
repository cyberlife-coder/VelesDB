//! The server binary's update check, end to end: `main.rs` must hand the
//! check the config file the server was started with and name itself
//! `server`. A unit test of the plan function cannot catch a call site that
//! stops using it, so these run the real binary against loopback endpoints
//! (`update_check_support`) and watch what it sends.
//!
//! The `--config` file lives in `conf/` while the working directory holds a
//! different `velesdb.toml`, so a binary that read the wrong one would send
//! to, or stay silent for, the wrong endpoint.

#![cfg(all(feature = "persistence", feature = "update-check"))]

mod update_check_support;

use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use update_check_support::{isolate, write_config, ChildGuard, Endpoint};

const WAIT: Duration = Duration::from_secs(60);
const GRACE: Duration = Duration::from_secs(3);

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// Polls until a TCP connection to `port` succeeds or `within` passes.
fn wait_for_port(port: u16, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Starts the server in `home` (its working directory) with `args`.
fn start_server(home: &TempDir, args: &[&str]) -> (ChildGuard, u16) {
    let port = free_port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-server"));
    isolate(&mut command, home.path());
    command
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["--data-dir", &home.path().join("data").to_string_lossy()])
        .args(args)
        .current_dir(home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = ChildGuard(command.spawn().expect("spawn velesdb-server"));
    (child, port)
}

/// `--config conf/custom.toml` (enabled, endpoint `named`) next to a
/// `./velesdb.toml` that points at `other` and opts out.
#[test]
fn test_server_uses_the_config_file_it_was_given_not_the_working_directory_one() {
    let home = TempDir::new().expect("tempdir");
    let (named, other) = (Endpoint::bind(), Endpoint::bind());
    let config = write_config(home.path(), "conf/custom.toml", None, &named);
    write_config(home.path(), "velesdb.toml", Some(false), &other);

    let _server = start_server(&home, &["--config", &config.to_string_lossy()]);

    let request = named
        .receive(WAIT)
        .expect("the server reports to the endpoint named in --config");
    assert!(request.contains("\"entry\":\"server\""), "{request}");
    assert!(request.contains("\"ci\":"), "{request}");
    assert!(other.receive(Duration::from_millis(500)).is_none());
}

/// `--config` opts out while `./velesdb.toml` would send: silence proves the
/// first one won.
#[test]
fn test_server_opt_out_in_the_given_config_file_beats_the_working_directory_file() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), "conf/off.toml", Some(false), &endpoint);
    write_config(home.path(), "velesdb.toml", Some(true), &endpoint);

    let (_server, port) = start_server(&home, &["--config", &config.to_string_lossy()]);

    assert!(wait_for_port(port, WAIT), "the server came up");
    assert!(endpoint.receive(GRACE).is_none());
}

/// Without `--config` the check reads `./velesdb.toml`, and uses it.
#[test]
fn test_server_without_config_reads_velesdb_toml_from_its_working_directory() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), "velesdb.toml", None, &endpoint);

    let _server = start_server(&home, &[]);

    let request = endpoint
        .receive(WAIT)
        .expect("the server reports to the endpoint named in ./velesdb.toml");
    assert!(request.contains("\"entry\":\"server\""), "{request}");
}

#[test]
fn test_server_without_config_honours_an_opt_out_in_velesdb_toml() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), "velesdb.toml", Some(false), &endpoint);

    let (_server, port) = start_server(&home, &[]);

    assert!(wait_for_port(port, WAIT), "the server came up");
    assert!(endpoint.receive(GRACE).is_none());
}
