//! The server binary's update check, end to end: `main.rs` must hand the
//! check the config file the server was started with and name itself
//! `server`. A unit test of the plan function cannot catch a call site that
//! stops using it, so these run the real binary against a loopback endpoint
//! (`update_check_support`) and watch what it sends.

#![cfg(all(feature = "persistence", feature = "update-check"))]

mod update_check_support;

use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::time::Instant;
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

fn start_server(home: &TempDir, args: &[&str], cwd: &std::path::Path) -> (ChildGuard, u16) {
    let port = free_port();
    let mut command = Command::new(env!("CARGO_BIN_EXE_velesdb-server"));
    isolate(&mut command, home.path());
    command
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["--data-dir", &home.path().join("data").to_string_lossy()])
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = ChildGuard(command.spawn().expect("spawn velesdb-server"));
    (child, port)
}

#[test]
fn test_server_reports_itself_with_the_config_file_endpoint() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), None, &endpoint.url());

    let _server = start_server(&home, &["--config", &config.to_string_lossy()], home.path());

    let request = endpoint
        .receive(WAIT)
        .expect("the server sends its start-up check to the configured endpoint");
    assert!(request.contains("\"entry\":\"server\""), "{request}");
    assert!(request.contains("\"ci\":"), "{request}");
}

#[test]
fn test_server_sends_nothing_when_its_config_file_opts_out() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    let config = write_config(home.path(), Some(false), &endpoint.url());

    let (_server, port) =
        start_server(&home, &["--config", &config.to_string_lossy()], home.path());

    assert!(wait_for_port(port, WAIT), "the server came up");
    assert!(
        endpoint.receive(GRACE).is_none(),
        "enabled = false in the --config file must silence the check"
    );
}

#[test]
fn test_server_reads_velesdb_toml_from_its_working_directory() {
    let home = TempDir::new().expect("tempdir");
    let endpoint = Endpoint::bind();
    write_config(home.path(), Some(false), &endpoint.url());

    let (_server, port) = start_server(&home, &[], home.path());

    assert!(wait_for_port(port, WAIT), "the server came up");
    assert!(
        endpoint.receive(GRACE).is_none(),
        "without --config, ./velesdb.toml is the file the check reads"
    );
}
